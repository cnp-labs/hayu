#!/usr/bin/env bash
# Live bar pointer check: a left click on a workspace pill switches workspace,
# a click elsewhere on the bar does nothing, and the shell survives both.
#
#   cd <repo root>
#   tools/live/pointer-suite.sh
#
# Needs a pointer-capable seat, `zwlr_virtual_pointer_manager_v1` in the
# compositor, and a built `tools/live/keyin`. Starts its own shell against a
# temporary XDG_CONFIG_HOME with only the workspace module in the bar.
set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT" || exit 1
BIN=${HAYU_BIN:-$ROOT/target/release/hayu}
KEYIN=${KEYIN_BIN:-$ROOT/tools/live/keyin/target/release/keyin}
WORK=$(mktemp -d "${TMPDIR:-/tmp}/hayu-pointer-XXXXXX")
CFG=$WORK/config/hayu/config.toml
LOG=$WORK/hayu.log
fail=0
checks=0
SHELL_PID=
say() { printf '%s\n' "$*"; }
ok()  { checks=$((checks + 1)); say "PASS $*"; }
bad() { checks=$((checks + 1)); fail=1; say "FAIL $*"; }
check() { if [ "$1" = "$2" ]; then ok "$3"; else bad "$3: expected [$2] got [$1]"; fi; }

[ -x "$KEYIN" ] || { say "build keyin first: cargo build --release --offline --manifest-path tools/live/keyin/Cargo.toml"; exit 1; }

cleanup() {
    [ -n "$SHELL_PID" ] && kill "$SHELL_PID" 2>/dev/null
    wait 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

mkdir -p "$WORK/config/hayu"
cat >"$CFG" <<'TOML'
schema_version = 1
[bar]
position = "top"
height = 32
left = ["workspaces"]
center = []
right = []
TOML

active_ws() { hyprctl -j activeworkspace | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])'; }
layout_size() {
    hyprctl -j monitors | python3 -c '
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
# Layout-space point at the centre of a pill, using the focused monitor offset.
pill_point() {
    local id=$1 x w mon_x mon_y
    x=$(pill_field "$id" x); w=$(pill_field "$id" w)
    [ -n "$x" ] && [ -n "$w" ] || return 1
    read -r mon_x mon_y < <(hyprctl -j monitors | python3 -c '
import json, sys
for m in json.load(sys.stdin):
    if m["focused"]:
        print(m["x"], m["y"])
        break
')
    python3 -c "print(f'{$mon_x + $x + $w / 2:.1f}:{$mon_y + 16:.1f}')"
}

# Two workspaces on the focused monitor, so the bar has two pills.
# Hyprland 0.57 with a Lua config only accepts the Lua form; older releases
# accept the legacy command.
focus_ws() {
    hyprctl dispatch workspace "$1" >/dev/null 2>&1 \
        || hyprctl repl "hl.dispatch(hl.dsp.focus({ workspace = $1 }))" >/dev/null 2>&1
}
focus_ws 2
focus_ws 1
sleep 1
start_ws=$(active_ws)

XDG_CONFIG_HOME="$WORK/config" HAYU_DEBUG_POINTER=1 "$BIN" run >"$LOG" 2>&1 &
SHELL_PID=$!
sleep 2
if ! kill -0 "$SHELL_PID" 2>/dev/null; then
    bad "the shell started"
    say "log:"
    tail -5 "$LOG"
    exit 1
fi
ok "the shell started"

for _ in $(seq 1 20); do
    [ -n "$(pill_field 2 w)" ] && break
    sleep 0.2
done
if [ -n "$(pill_field 2 w)" ]; then
    ok "workspace 2 has a drawn pill"
else
    bad "workspace 2 has a drawn pill (drawn: $(pills))"
fi

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
    screen_w=${screen%x*}
    empty="$((screen_w - 20)):16"
    before=$(active_ws)
    KEYIN_SCREEN="$screen" "$KEYIN" "click:$empty" >/dev/null 2>&1
    sleep 1
    check "$(active_ws)" "$before" "a click on empty bar space changes nothing"
fi

kill -0 "$SHELL_PID" 2>/dev/null && ok "the shell survived the clicks" || bad "the shell survived the clicks"
"$BIN" quit >/dev/null 2>&1; check "$?" 0 "hayu quit exits 0"
wait "$SHELL_PID" 2>/dev/null
focus_ws "$start_ws"

say "RESULT: $checks checks, $fail failed"
[ "$fail" -eq 0 ]
