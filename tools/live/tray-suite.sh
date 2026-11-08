#!/usr/bin/env bash
# Live tray check: an SNI item registers with hayu's watcher, its icon is
# drawn, a left click calls Activate, the item disappears when its owner exits,
# and a second hayu does not take the watcher name from the first.
#
#   cd <repo root>
#   tools/live/tray-suite.sh
#
# Re-execs itself inside dbus-run-session, so the real tray host is untouched.
# fcitx5 is the tray application: it runs in the foreground with a private
# runtime/bus/config/home and is killed by its own PID on exit. Both hayu
# shells get their own short private runtime. Needs
# `zwlr_virtual_pointer_manager_v1` for the click.
set -u
if [ -z "${HAYU_TRAY_INNER:-}" ]; then
    HAYU_TRAY_INNER=1 exec dbus-run-session -- "$0" "$@"
fi
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT" || exit 1
. "$ROOT/tools/live/isolation.sh"
BIN=${HAYU_BIN:-$ROOT/target/release/hayu}
KEYIN=${KEYIN_BIN:-$ROOT/tools/live/keyin/target/release/keyin}
iso_init
WORK=$(mktemp -d "${TMPDIR:-/tmp}/hayu-tray-XXXXXX")
CFG=$WORK/config/hayu/config.toml
CFG2=$WORK/config2/hayu/config.toml
LOG=$WORK/hayu.log
LOG2=$WORK/hayu2.log
MON=$WORK/monitor.log
FC_LOG=$WORK/fcitx5.log
RT1=
RT2=
RT_FC=
fail=0
checks=0
SHELL_PID=
SECOND_PID=
MON_PID=
FC_PID=
say() { printf '%s\n' "$*"; }
ok()  { checks=$((checks + 1)); say "PASS $*"; }
bad() { checks=$((checks + 1)); fail=1; say "FAIL $*"; }
check() { if [ "$1" = "$2" ]; then ok "$3"; else bad "$3: expected [$2] got [$1]"; fi; }

[ -x "$KEYIN" ] || { say "build keyin first: cargo build --release --offline --manifest-path tools/live/keyin/Cargo.toml"; exit 1; }

cleanup() {
    iso_evidence "$LOG" tray-suite
    iso_evidence "$LOG2" tray-suite-second
    iso_evidence "$FC_LOG" tray-suite-fcitx
    [ -n "$MON_PID" ] && kill "$MON_PID" 2>/dev/null
    [ -n "$FC_PID" ] && kill "$FC_PID" 2>/dev/null
    [ -n "$RT1" ] && iso_destroy "$RT1"
    [ -n "$RT2" ] && iso_destroy "$RT2"
    [ -n "$RT_FC" ] && iso_destroy "$RT_FC"
    iso_user_ok || printf 'ISOLATION ABORT: USER SHELL CHANGED\n' >&2
    rm -rf "$WORK"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

mkdir -p "$WORK/config/hayu" "$WORK/config2/hayu"
cat >"$CFG" <<'TOML'
schema_version = 1
[bar]
position = "bottom"
left = ["workspaces"]
center = ["clock"]
right = ["tray"]
TOML
cp "$CFG" "$CFG2"

screen=$(iso_hyprctl -j monitors | python3 -c '
import json, sys
mons = json.load(sys.stdin)
w = max(m["x"] + m["width"] for m in mons)
h = max(m["y"] + m["height"] for m in mons)
print(f"{w}x{h}")
')

# --- tray application and shell -------------------------------------------
mkdir -p "$WORK/fc-config" "$WORK/fc-data" "$WORK/fc-cache" "$WORK/fc-home"
RT_FC=$(iso_rt hr-fc) || exit 1
env XDG_RUNTIME_DIR="$RT_FC" XDG_CONFIG_HOME="$WORK/fc-config" \
    XDG_DATA_HOME="$WORK/fc-data" XDG_CACHE_HOME="$WORK/fc-cache" \
    HOME="$WORK/fc-home" \
    fcitx5 -D -k --disable=wayland,waylandim,xcb,xim >"$FC_LOG" 2>&1 &
FC_PID=$!
sleep 3
if kill -0 "$FC_PID" 2>/dev/null; then
    ok "fcitx5 started on the private bus and runtime"
else
    bad "fcitx5 started on the private bus and runtime ($(tail -3 "$FC_LOG" | tr '\n' ' '))"
fi

dbus-monitor --session "type='method_call'" >"$MON" 2>&1 &
MON_PID=$!
name_owner() { busctl --user list --no-pager | awk '$1=="org.kde.StatusNotifierWatcher"{print $5}'; }
RT1=$(iso_rt hr-tr1) || exit 1
iso_start "$RT1" "$LOG" env XDG_CONFIG_HOME="$WORK/config" HAYU_DEBUG_POINTER=1 "$BIN" run
SHELL_PID=$ISO_PID
sleep 3
kill -0 "$SHELL_PID" 2>/dev/null && ok "the shell started" || bad "the shell started"
iso_assert_env "$SHELL_PID" "$RT1" || bad "the shell has the private runtime"
owner_after=$(name_owner)
case "$owner_after" in
    ""|"org.kde.StatusNotifierWatcher") bad "the shell owns the watcher name ($owner_after)" ;;
    *) ok "the shell owns the watcher name ($owner_after)" ;;
esac

items=$(busctl --user get-property org.kde.StatusNotifierWatcher /StatusNotifierWatcher \
    org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems 2>/dev/null)
case "$items" in
    "as 0") bad "the item registered with hayu's watcher" ;;
    "as "*) ok "the item registered with hayu's watcher ($items)" ;;
    *) bad "the item registered with hayu's watcher ($items)" ;;
esac

for _ in $(seq 1 25); do
    grep -q "hayu: bar tray" "$LOG" && break
    sleep 0.2
done
tray_line=$(grep -o "hayu: bar tray .*" "$LOG" | tail -1)
if [ -n "$tray_line" ]; then
    ok "an icon was drawn with a hit rect ($tray_line)"
else
    bad "an icon was drawn with a hit rect"
fi
for _ in $(seq 1 20); do
    BAR_RECT=$(iso_layer_rect "$SHELL_PID" hayu 2>/dev/null) && [ -n "$BAR_RECT" ] && break
    sleep 0.2
done
[ -n "$BAR_RECT" ] && ok "the test bar layer was found" || bad "the test bar layer was found"

# --- left click calls Activate --------------------------------------------
# Hit rect from the test shell's own layer, not the user's top bar.
point=
if [ -n "$tray_line" ] && [ -n "$BAR_RECT" ]; then
    icon=$(printf '%s' "$tray_line" | sed 's/^hayu: bar tray //' | tr ' ' '\n' | head -1)
    IFS=: read -r _icon_id icon_x icon_w <<<"$icon"
    read -r bx by bw bh <<<"$BAR_RECT"
    point=$(python3 -c "print(f'{$bx + $icon_x + $icon_w / 2:.1f}:{$by + $bh / 2:.1f}')")
fi
if [ -n "$point" ]; then
    KEYIN_SCREEN="$screen" "$KEYIN" "click:$point" >/dev/null 2>&1
    sleep 1
    if grep -q "member=Activate" "$MON"; then
        ok "a left click called org.kde.StatusNotifierItem.Activate"
    else
        bad "a left click called org.kde.StatusNotifierItem.Activate"
    fi
else
    bad "the icon click point could be computed"
fi

# --- a second shell must not take the watcher name ------------------------
RT2=$(iso_rt hr-tr2) || exit 1
iso_start "$RT2" "$LOG2" env XDG_CONFIG_HOME="$WORK/config2" HAYU_DEBUG_POINTER=1 "$BIN" run
SECOND_PID=$ISO_PID
sleep 3
if grep -q "another StatusNotifier host owns" "$LOG2"; then
    ok "a second shell refused to take the watcher name"
else
    bad "a second shell refused to take the watcher name ($(grep -E 'hayu:' "$LOG2" | tail -3 | tr '\n' ' '))"
fi
owner_last=$(name_owner)
check "$owner_last" "$owner_after" "the refused shell left the name owner unchanged"

# --- item removal ---------------------------------------------------------
kill "$FC_PID" 2>/dev/null
for _ in $(seq 1 25); do
    items=$(busctl --user get-property org.kde.StatusNotifierWatcher /StatusNotifierWatcher \
        org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems 2>/dev/null)
    [ "$items" = "as 0" ] && break
    sleep 0.2
done
check "$items" "as 0" "the item was removed after its owner exited"
kill -0 "$SHELL_PID" 2>/dev/null && ok "the shell survived the removal" || bad "the shell survived the removal"

iso_hayu "$RT1" quit >/dev/null 2>&1; check "$?" 0 "hayu quit exits 0"
wait "$SHELL_PID" 2>/dev/null
iso_user_ok && ok "the user shell and socket are unchanged" || bad "the user shell and socket are unchanged"

say "RESULT: $checks checks, $fail failed"
[ "$fail" -eq 0 ]
