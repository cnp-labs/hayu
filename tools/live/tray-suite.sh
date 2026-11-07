#!/usr/bin/env bash
# Live tray check: an SNI item registers with hayu's watcher, its icon is
# drawn, a left click calls Activate, the item disappears when its owner exits,
# and a second hayu does not take the watcher name from the first.
#
#   cd <repo root>
#   tools/live/tray-suite.sh
#
# Re-execs itself inside dbus-run-session, so the real tray host is untouched.
# fcitx5 is the tray application: it gets a private XDG_RUNTIME_DIR whose
# wayland-1 symlinks the real socket, and is killed on exit. Needs
# `zwlr_virtual_pointer_manager_v1` for the click.
set -u
if [ -z "${HAYU_TRAY_INNER:-}" ]; then
    HAYU_TRAY_INNER=1 exec dbus-run-session -- "$0" "$@"
fi
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT" || exit 1
BIN=${HAYU_BIN:-$ROOT/target/release/hayu}
KEYIN=${KEYIN_BIN:-$ROOT/tools/live/keyin/target/release/keyin}
REAL_RUNTIME=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
WORK=$(mktemp -d "${TMPDIR:-/tmp}/hayu-tray-XXXXXX")
CFG=$WORK/config/hayu/config.toml
CFG2=$WORK/config2/hayu/config.toml
LOG=$WORK/hayu.log
LOG2=$WORK/hayu2.log
MON=$WORK/monitor.log
FC_LOG=$WORK/fcitx5.log
RT1=$WORK/rt1
RT2=$WORK/rt2
fail=0
checks=0
SHELL_PID=
SECOND_PID=
MON_PID=
FC_PIDS=
say() { printf '%s\n' "$*"; }
ok()  { checks=$((checks + 1)); say "PASS $*"; }
bad() { checks=$((checks + 1)); fail=1; say "FAIL $*"; }
check() { if [ "$1" = "$2" ]; then ok "$3"; else bad "$3: expected [$2] got [$1]"; fi; }

[ -x "$KEYIN" ] || { say "build keyin first: cargo build --release --offline --manifest-path tools/live/keyin/Cargo.toml"; exit 1; }

cleanup() {
    [ -n "$MON_PID" ] && kill "$MON_PID" 2>/dev/null
    [ -n "$SHELL_PID" ] && kill "$SHELL_PID" 2>/dev/null
    [ -n "$SECOND_PID" ] && kill "$SECOND_PID" 2>/dev/null
    for pid in $FC_PIDS; do kill "$pid" 2>/dev/null; done
    wait 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

mkdir -p "$WORK/config/hayu" "$WORK/config2/hayu" "$RT1" "$RT2"
chmod 700 "$RT1" "$RT2"
ln -sfn "$REAL_RUNTIME/wayland-1" "$RT1/wayland-1"
ln -sfn "$REAL_RUNTIME/wayland-1" "$RT2/wayland-1"
[ -e "$REAL_RUNTIME/wayland-1.lock" ] && ln -sfn "$REAL_RUNTIME/wayland-1.lock" "$RT1/wayland-1.lock"
[ -e "$REAL_RUNTIME/wayland-1.lock" ] && ln -sfn "$REAL_RUNTIME/wayland-1.lock" "$RT2/wayland-1.lock"

cat >"$CFG" <<'TOML'
schema_version = 1
[bar]
left = ["workspaces"]
center = ["clock"]
right = ["tray"]
TOML
cp "$CFG" "$CFG2"

screen=$(hyprctl -j monitors | python3 -c '
import json, sys
mons = json.load(sys.stdin)
w = max(m["x"] + m["width"] for m in mons)
h = max(m["y"] + m["height"] for m in mons)
print(f"{w}x{h}")
')
mon_x=$(hyprctl -j monitors | python3 -c '
import json, sys
for m in json.load(sys.stdin):
    if m["focused"]:
        print(m["x"]); break
else:
    print(0)
')
mon_y=$(hyprctl -j monitors | python3 -c '
import json, sys
for m in json.load(sys.stdin):
    if m["focused"]:
        print(m["y"]); break
else:
    print(0)
')

# --- tray application and shell -------------------------------------------
before=$(pgrep -x fcitx5 | tr '\n' ' ')
XDG_RUNTIME_DIR="$RT1" fcitx5 -d >"$FC_LOG" 2>&1
sleep 3
for pid in $(pgrep -x fcitx5); do
    case " $before " in *" $pid "*) ;; *) FC_PIDS="$FC_PIDS $pid" ;; esac
done
if [ -n "$FC_PIDS" ]; then ok "fcitx5 started on the private bus"; else bad "fcitx5 started on the private bus"; fi

dbus-monitor --session "type='method_call'" >"$MON" 2>&1 &
MON_PID=$!
name_owner() { busctl --user list --no-pager | awk '$1=="org.kde.StatusNotifierWatcher"{print $5}'; }
XDG_CONFIG_HOME="$WORK/config" HAYU_DEBUG_POINTER=1 "$BIN" run >"$LOG" 2>&1 &
SHELL_PID=$!
sleep 3
kill -0 "$SHELL_PID" 2>/dev/null && ok "the shell started" || bad "the shell started"
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

# --- left click calls Activate --------------------------------------------
point=$(printf '%s' "$tray_line" | sed 's/^hayu: bar tray //' | tr ' ' '\n' | head -1 \
    | awk -F: -v x="$mon_x" -v y="$mon_y" '{ printf "%.1f:%.1f", x + $2 + $3 / 2, y + 16 }')
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
XDG_RUNTIME_DIR="$RT2" XDG_CONFIG_HOME="$WORK/config2" HAYU_DEBUG_POINTER=1 \
    "$BIN" run >"$LOG2" 2>&1 &
SECOND_PID=$!
sleep 3
if grep -q "another StatusNotifier host owns" "$LOG2"; then
    ok "a second shell refused to take the watcher name"
else
    bad "a second shell refused to take the watcher name ($(grep -E 'hayu:' "$LOG2" | tail -3 | tr '\n' ' '))"
fi
owner_last=$(name_owner)
check "$owner_last" "$owner_after" "the refused shell left the name owner unchanged"

# --- item removal ---------------------------------------------------------
for pid in $FC_PIDS; do kill "$pid" 2>/dev/null; done
for _ in $(seq 1 25); do
    items=$(busctl --user get-property org.kde.StatusNotifierWatcher /StatusNotifierWatcher \
        org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems 2>/dev/null)
    [ "$items" = "as 0" ] && break
    sleep 0.2
done
check "$items" "as 0" "the item was removed after its owner exited"
kill -0 "$SHELL_PID" 2>/dev/null && ok "the shell survived the removal" || bad "the shell survived the removal"

"$BIN" quit >/dev/null 2>&1; check "$?" 0 "hayu quit exits 0"
wait "$SHELL_PID" 2>/dev/null

say "RESULT: $checks checks, $fail failed"
[ "$fail" -eq 0 ]
