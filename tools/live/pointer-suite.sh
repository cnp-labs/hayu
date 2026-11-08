#!/usr/bin/env bash
# Live bar pointer check: a left click on a workspace pill switches workspace,
# a click elsewhere on the bar does nothing, and the shell survives both.
#
#   cd <repo root>
#   tools/live/pointer-suite.sh
#
# Needs a pointer-capable seat, `zwlr_virtual_pointer_manager_v1` in the
# compositor, and a built `tools/live/keyin`. Starts its own shell in a private
# runtime with only the workspace module in the bar, and restores the original
# workspace, monitor and focused window afterwards.
set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT" || exit 1
. "$ROOT/tools/live/isolation.sh"
BIN=${HAYU_BIN:-$ROOT/target/release/hayu}
KEYIN=${KEYIN_BIN:-$ROOT/tools/live/keyin/target/release/keyin}
iso_init
WORK=$(mktemp -d "${TMPDIR:-/tmp}/hayu-pointer-XXXXXX")
CFG=$WORK/config/hayu/config.toml
LOG=$WORK/hayu.log
fail=0
checks=0
SHELL_PID=
RT=
BAR_RECT=
say() { printf '%s\n' "$*"; }
ok()  { checks=$((checks + 1)); say "PASS $*"; }
bad() { checks=$((checks + 1)); fail=1; say "FAIL $*"; }
check() { if [ "$1" = "$2" ]; then ok "$3"; else bad "$3: expected [$2] got [$1]"; fi; }

[ -x "$KEYIN" ] || { say "build keyin first: cargo build --release --offline --manifest-path tools/live/keyin/Cargo.toml"; exit 1; }

cleanup() {
    iso_evidence "$LOG" pointer-suite
    [ -n "$RT" ] && iso_destroy "$RT"
    [ -n "${ORIG_WS:-}" ] && iso_restore_focus >/dev/null 2>&1
    iso_user_ok || printf 'ISOLATION ABORT: USER SHELL CHANGED\n' >&2
    rm -rf "$WORK"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

mkdir -p "$WORK/config/hayu"
cat >"$CFG" <<'TOML'
schema_version = 1
[bar]
position = "bottom"
height = 32
left = ["workspaces"]
center = []
right = []
TOML

active_ws() { iso_hyprctl -j activeworkspace | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])'; }
layout_size() {
    iso_hyprctl -j monitors | python3 -c '
import json, sys
mons = json.load(sys.stdin)
w = max(m["x"] + m["width"] for m in mons)
h = max(m["y"] + m["height"] for m in mons)
print(f"{w}x{h}")
'
}
# "id:x:w" per drawn pill, in logical bar coordinates.
pills() { grep -o 'hayu: bar pills .*' "$LOG" | tail -1 | sed 's/^hayu: bar pills //'; }
pill_field() { # $1 = workspace id, $2 = x|w
    pills | tr ' ' '\n' | awk -F: -v id="$1" -v col="$2" '$1 == id { print (col == "x" ? $2 : $3) }'
}
# Click point from the test bar's own PID-filtered layer, so a user bar at the
# top can never be hit by mistake.
pill_point() {
    local id=$1 x w bx by bw bh
    x=$(pill_field "$id" x); w=$(pill_field "$id" w)
    [ -n "$x" ] && [ -n "$w" ] && [ -n "$BAR_RECT" ] || return 1
    read -r bx by bw bh <<<"$BAR_RECT"
    python3 -c "print(f'{$bx + $x + $w / 2:.1f}:{$by + $bh / 2:.1f}')"
}

# Two workspaces on the focused monitor, so the bar has two pills.
# Hyprland 0.57 with a Lua config only accepts the Lua form; iso_focus_ws
# verifies the resulting active workspace instead of trusting reply text.

iso_capture_focus
case "$ORIG_MON" in
    HEADLESS*) say "ABORT: original focus is on headless output $ORIG_MON"; exit 1 ;;
esac
iso_focus_ws 2 || { say "ABORT: cannot focus workspace 2"; exit 1; }
iso_focus_ws 1 || { say "ABORT: cannot focus workspace 1"; exit 1; }
sleep 1

RT=$(iso_rt hr-ptr) || exit 1
iso_start "$RT" "$LOG" env XDG_CONFIG_HOME="$WORK/config" HAYU_DEBUG_POINTER=1 "$BIN" run
SHELL_PID=$ISO_PID
sleep 2
if ! kill -0 "$SHELL_PID" 2>/dev/null; then
    bad "the shell started"
    say "log:"
    tail -5 "$LOG"
    exit 1
fi
iso_assert_env "$SHELL_PID" "$RT" || { bad "the shell has the wrong runtime"; exit 1; }
ok "the shell started"

for _ in $(seq 1 20); do
    [ -n "$(pill_field 2 w)" ] && break
    sleep 0.2
done
for _ in $(seq 1 20); do
    BAR_RECT=$(iso_layer_rect "$SHELL_PID" hayu 2>/dev/null) && [ -n "$BAR_RECT" ] && break
    sleep 0.2
done
if [ -n "$(pill_field 2 w)" ]; then
    ok "workspace 2 has a drawn pill"
else
    bad "workspace 2 has a drawn pill (drawn: $(pills))"
fi
[ -n "$BAR_RECT" ] && ok "the test bar layer was found" || bad "the test bar layer was found"

point=$(pill_point 2) || point=
if [ -z "$point" ]; then
    bad "workspace 2 pill geometry is available"
else
    screen=$(layout_size)
    KEYIN_SCREEN="$screen" "$KEYIN" "click:$point" >/dev/null 2>&1
    for _ in $(seq 1 20); do
        [ "$(active_ws)" = 2 ] && break
        sleep 0.1
    done
    check "$(active_ws)" 2 "a left click on pill 2 switches to workspace 2"

    point=$(pill_point 1) || point=
    if [ -n "$point" ]; then
        KEYIN_SCREEN="$screen" "$KEYIN" "click:$point" >/dev/null 2>&1
        for _ in $(seq 1 20); do
            [ "$(active_ws)" = 1 ] && break
            sleep 0.1
        done
        check "$(active_ws)" 1 "a left click on pill 1 switches back"
    else
        bad "workspace 1 pill geometry is available"
    fi

    # Far right of the bar: inside the surface, on no pill.
    read -r bx by bw bh <<<"$BAR_RECT"
    empty="$((bx + bw - 20)):$((by + bh / 2))"
    before=$(active_ws)
    KEYIN_SCREEN="$screen" "$KEYIN" "click:$empty" >/dev/null 2>&1
    sleep 1
    check "$(active_ws)" "$before" "a click on empty bar space changes nothing"
fi

kill -0 "$SHELL_PID" 2>/dev/null && ok "the shell survived the clicks" || bad "the shell survived the clicks"
iso_hayu "$RT" quit >/dev/null 2>&1; check "$?" 0 "hayu quit exits 0"
wait "$SHELL_PID" 2>/dev/null
iso_restore_focus >/dev/null 2>&1
check "$(active_ws)" "$ORIG_WS" "the original workspace was restored"
iso_user_ok && ok "the user shell and socket are unchanged" || bad "the user shell and socket are unchanged"

say "RESULT: $checks checks, $fail failed"
[ "$fail" -eq 0 ]
