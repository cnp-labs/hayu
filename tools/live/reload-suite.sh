#!/usr/bin/env bash
# Live `hayu reload` suite: geometry, invalid-config survival, the
# launcher.enabled and bar.enabled gates, bar.outputs, repeated tray
# enable/disable, and the restart-required notice.
#
#   dbus-run-session -- tools/live/reload-suite.sh
#
# Running under `dbus-run-session` is required for the tray section: with a
# private session bus the test shell owns org.kde.StatusNotifierWatcher, so
# "exactly one worker" is a real assertion. On the user's bus the name is
# already taken, hayu takes DoNotQueue and reports Disabled, and the suite
# fails the private-bus guard instead of passing vacuously.
#
# Runtime isolation comes from tools/live/isolation.sh: every client and the
# test shell use a short private runtime with only the live Wayland/Hyprland
# sockets symlinked, and every layer observation is filtered by the test PID.
# The user's shell socket and focus are checked before and after. Only a
# headless output this run created is ever removed.
set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT" || exit 1
BIN=${HAYU_BIN:-$ROOT/target/release/hayu}
. "$ROOT/tools/live/isolation.sh"
iso_init

WORK=$(mktemp -d "${TMPDIR:-/tmp}/hayu-reload-XXXXXX")
CFG=$WORK/config/hayu/config.toml
LOG=$WORK/hayu.log
export XDG_CONFIG_HOME="$WORK/config"
export XDG_DATA_HOME="$WORK/data"
export XDG_STATE_HOME="$WORK/state"
export XDG_CACHE_HOME="$WORK/cache"
fail=0
checks=0
SHELL_PID=
RT=
OWNED_HEAD=
ORIG_WS=
ORIG_MON=
ORIG_WIN=

say() { printf '%s\n' "$*"; }
ok()  { checks=$((checks + 1)); say "PASS $*"; }
bad() { checks=$((checks + 1)); fail=1; say "FAIL $*"; }
check() { if [ "$1" = "$2" ]; then ok "$3"; else bad "$3: expected [$2] got [$1]"; fi; }
is_ge() { if [ "$1" -ge "$2" ] 2>/dev/null; then ok "$3 ($1 >= $2)"; else bad "$3: $1 < $2"; fi; }

# Every Hayu client goes through the private runtime.
hayu() { iso_hayu "$RT" "$@"; }
# PID-filtered test-surface geometry: "x y w h", empty when absent.
bar_rect() { iso_layer_rect "$SHELL_PID" hayu 2>/dev/null; }
bar_h() { local rect; rect=$(bar_rect); [ -n "$rect" ] && set -- $rect && printf '%s\n' "$4" || printf 'none\n'; }
bar_y() { local rect; rect=$(bar_rect); [ -n "$rect" ] && set -- $rect && printf '%s\n' "$2" || printf 'none\n'; }
bar_w() { local rect; rect=$(bar_rect); [ -n "$rect" ] && set -- $rect && printf '%s\n' "$3" || printf 'none\n'; }
# A closed launcher keeps its layer surface and only detaches the buffer, so
# count it as shown only while it has a real geometry.
launcher_shown() {
    local rect
    rect=$(iso_layer_rect "$SHELL_PID" hayu-launcher 2>/dev/null) || { printf '0\n'; return; }
    set -- $rect
    [ "${3:-0}" -gt 0 ] && [ "${4:-0}" -gt 0 ] && printf '1\n' || printf '0\n'
}
tray_workers() {
    # Threads the test shell actually spawned; "hayu-tray" is the worker name.
    grep -l '^hayu-tray$' /proc/"$SHELL_PID"/task/*/comm 2>/dev/null | wc -l
}
shell_alive() { kill -0 "$SHELL_PID" 2>/dev/null; }
no_protocol_error() {
    local hit
    hit=$(grep -inE "protocol error|invalid object" "$LOG" 2>/dev/null | head -1)
    if [ -z "$hit" ]; then
        ok "no Wayland protocol error after $1"
    else
        bad "Wayland protocol error after $1: $hit"
    fi
}
write_cfg() { printf '%s\n' "$@" > "$CFG"; }
reload() { hayu reload "$@"; }
monitors_json() { iso_hyprctl -j monitors 2>/dev/null; }
mon_field() {
    monitors_json | iso_json "import json, sys
for m in json.load(sys.stdin):
    if m['focused']:
        print(m['$1']); break"
}
mon_height() { mon_field height; }
head_width() {
    monitors_json | iso_json '
import json, sys
for m in json.load(sys.stdin):
    if m["name"] == sys.argv[1]:
        print(m["width"]); break
' "$1"
}

cleanup() {
    if [ -n "$OWNED_HEAD" ]; then
        iso_hyprctl output remove "$OWNED_HEAD" >/dev/null 2>&1
        OWNED_HEAD=
    fi
    [ -n "$RT" ] && iso_destroy "$RT"
    RT=
    [ -n "${ORIG_WS:-}" ] && iso_restore_focus
    # Keep the log outside the throwaway WORK dir, even on an early abort.
    iso_evidence "$LOG" reload-suite
    rm -rf "$WORK"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

[ -x "$BIN" ] || { say "ABORT: hayu binary is not executable: $BIN"; exit 1; }
mkdir -p "$(dirname "$CFG")"
iso_capture_focus
BEFORE_HEADS=$(monitors_json | iso_json 'import json,sys; print(" ".join(m["name"] for m in json.load(sys.stdin)))')
case "$ORIG_MON" in HEADLESS*) say "ABORT: user focus is on a headless output"; exit 1 ;; esac

RT=$(iso_rt hr-r) || { say "ABORT: cannot create private runtime"; exit 1; }

write_cfg 'schema_version = 1' '' '[bar]' 'position = "top"' 'height = 32'
iso_start "$RT" "$LOG" "$BIN" run
SHELL_PID=$ISO_PID
for _ in $(seq 1 50); do hayu launcher close >/dev/null 2>&1 && break; sleep 0.1; done
iso_assert_env "$SHELL_PID" "$RT" \
    && ok "the test shell runs in the private runtime" \
    || bad "the test shell does not carry the private runtime"
bus_addr() { tr '\0' '\n' <"/proc/$1/environ" 2>/dev/null | sed -n 's/^DBUS_SESSION_BUS_ADDRESS=//p'; }
if [ -z "$USER_PID" ] || [ "$(bus_addr "$SHELL_PID")" != "$(bus_addr "$USER_PID")" ]; then
    ok "the test shell has its own session bus"
else
    bad "the test shell shares the user bus; run this suite under dbus-run-session"
fi
for _ in $(seq 1 50); do [ -n "$(bar_rect)" ] && break; sleep 0.1; done

check "$(bar_h)" 32 "initial bar height"
mon_h=$(mon_height)
if [ "$(bar_y)" -lt $((mon_h / 2)) ]; then ok "initial bar is at the top (y=$(bar_y))"; else bad "initial bar is at the top (y=$(bar_y))"; fi

# 1. Applied keys only: geometry changes and no notice is printed.
write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40'
out=$(reload 2>&1); check "$?" 0 "reload with a valid config exits 0"
check "$out" "" "an applied-only reload prints no notice"
sleep 2
check "$(bar_h)" 40 "reload applied the new height"
mon_h=$(mon_height)
is_ge "$(bar_y)" $((mon_h - 42)) "reload moved the bar to the bottom"

hayu launcher toggle >/dev/null; check "$?" 0 "launcher toggles after reload"
sleep 1
check "$(launcher_shown)" 1 "launcher shown after reload"
hayu launcher toggle >/dev/null; sleep 1
check "$(launcher_shown)" 0 "launcher hidden again"

# 2. An unread key still needs restart; tray enablement applies live.
write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40' 'center = ["clock"]' '' '[clock]' 'format = "%H"' '' '[notifications]' 'enabled = false' '' '[tray]' 'enabled = false'
out=$(reload 2>&1); check "$?" 0 "reload with a restart-only key exits 0"
case "$out" in
    *"restart required for: notifications"*) ok "restart-only key is reported" ;;
    *) bad "restart-only key is reported (got: $out)" ;;
esac
case "$out" in
    *bar.left*|*bar.center*) bad "module areas are applied, not reported ($out)" ;;
    *) ok "module areas are applied, not reported" ;;
esac
sleep 1
check "$(bar_h)" 40 "restart-only reload kept the applied geometry"

# 3. Invalid reload: rejected, running state untouched.
write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 500'
out=$(reload 2>&1); code=$?
if [ "$code" -ne 0 ]; then ok "invalid config exits non-zero ($code)"; else bad "invalid config exits 0"; fi
case "$out" in *bar.height*) ok "invalid config names bar.height" ;; *) bad "invalid config error: $out" ;; esac
sleep 1
check "$(bar_h)" 40 "invalid reload kept the bar height"
mon_h=$(mon_height)
is_ge "$(bar_y)" $((mon_h - 42)) "invalid reload kept the bar at the bottom"
hayu launcher toggle >/dev/null; check "$?" 0 "launcher still opens after a rejected reload"
sleep 1
check "$(launcher_shown)" 1 "launcher shown after a rejected reload"
hayu launcher toggle >/dev/null; sleep 1

# 4. launcher.enabled = false is applied, not reported, and closes an overlay.
write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40' 'left = ["clock"]' '' '[launcher]' 'enabled = false'
out=$(reload 2>&1); check "$?" 0 "reload with launcher disabled exits 0"
case "$out" in *launcher.enabled*) bad "launcher.enabled is applied, not reported ($out)" ;; *) ok "launcher.enabled is applied, not reported" ;; esac
sleep 1
hayu launcher toggle 2>/dev/null; code=$?
if [ "$code" -ne 0 ]; then ok "disabled launcher rejects toggle ($code)"; else bad "disabled launcher accepted toggle"; fi
hayu launcher open 2>/dev/null; code=$?
if [ "$code" -ne 0 ]; then ok "disabled launcher rejects open ($code)"; else bad "disabled launcher accepted open"; fi
sleep 1
check "$(launcher_shown)" 0 "disabled launcher left nothing mapped"

# 5. bar.outputs is applied: a filter with no match removes the bar, and the
#    launcher keeps working through the output list rather than a bar surface.
write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40' 'left = ["clock"]' 'outputs = ["NOPE-1"]' '' '[launcher]' 'enabled = true'
out=$(reload 2>&1); check "$?" 0 "outputs-only reload exits 0"
case "$out" in *bar.outputs*) bad "bar.outputs is applied, not reported ($out)" ;; *) ok "bar.outputs is applied, not reported" ;; esac
sleep 1
check "$(bar_h)" none "a filter with no matching output removes the bar"
hayu launcher open >/dev/null; sleep 1
check "$(launcher_shown)" 1 "the launcher still opens without a bar"
hayu launcher close >/dev/null; sleep 1

# 6. bar.enabled = false is applied too.
write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40' 'left = ["clock"]' 'enabled = false' '' '[launcher]' 'enabled = true'
out=$(reload 2>&1); check "$?" 0 "bar.enabled=false reload exits 0"
case "$out" in *bar.enabled*) bad "bar.enabled is applied, not reported ($out)" ;; *) ok "bar.enabled is applied, not reported" ;; esac
sleep 1
check "$(bar_h)" none "bar.enabled=false removes the bar"
hayu launcher open >/dev/null; sleep 1
check "$(launcher_shown)" 1 "the launcher still opens with the bar disabled"
hayu launcher close >/dev/null; sleep 1

write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40' 'left = ["clock"]' '' '[launcher]' 'enabled = true'
out=$(reload 2>&1); check "$?" 0 "re-enabling the bar exits 0"
sleep 2
check "$(bar_h)" 40 "the bar comes back at the configured height"

# 6b. Repeated launcher disable releases the retained overlay surface, which is
#     destroyed only after the dispatch that dropped its layer role: the shell
#     must survive and log no protocol error.
for cycle in 1 2; do
    hayu launcher open >/dev/null; sleep 1
    check "$(launcher_shown)" 1 "launcher shown before disable $cycle"
    write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40' 'left = ["clock"]' '' '[launcher]' 'enabled = false'
    out=$(reload 2>&1); check "$?" 0 "launcher disable $cycle exits 0"
    sleep 1
    check "$(launcher_shown)" 0 "launcher disable $cycle released the overlay"
    shell_alive && ok "shell alive after launcher disable $cycle" || bad "shell died after launcher disable $cycle"
    write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40' 'left = ["clock"]' '' '[launcher]' 'enabled = true'
    reload >/dev/null 2>&1; sleep 1
done
no_protocol_error "repeated launcher disable"

# 7. Repeated tray enable/disable: exactly one worker while enabled (the
#    private bus means this shell owns the watcher, so the count is real),
#    and none after disable. The bar needs a tray module to consume it.
tray_cfg() {
    write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40' 'right = ["tray"]' '' '[tray]' "enabled = $1"
}
for cycle in 1 2 3; do
    tray_cfg true
    out=$(reload 2>&1); check "$?" 0 "tray enable reload $cycle exits 0"
    case "$out" in *[Tt]ray*) bad "tray.enabled is applied, not reported ($out)" ;; *) ok "tray.enabled is applied, not reported" ;; esac
    sleep 1
    check "$(tray_workers)" 1 "tray enable $cycle runs exactly one worker"
    tray_cfg false
    out=$(reload 2>&1); check "$?" 0 "tray disable reload $cycle exits 0"
    sleep 1
    check "$(tray_workers)" 0 "tray disable $cycle released its worker"
done
if grep -q "cannot start tray service" "$LOG"; then
    bad "repeated reload never failed to start the tray service"
else
    ok "repeated reload never failed to start the tray service"
fi

# 8. A named filter moves the bar to the matching output, including a
#    hotplugged one, and removes it again when that output goes away. Only the
#    headless output created here may be removed.
iso_hyprctl output create headless >/dev/null 2>&1
for _ in $(seq 1 30); do
    OWNED_HEAD=$(iso_hyprctl -j monitors | BEFORE="$BEFORE_HEADS" python3 -c '
import json, os, sys
before = set(os.environ["BEFORE"].split())
new = [m["name"] for m in json.load(sys.stdin) if m["name"] not in before]
print(new[0] if len(new) == 1 else "")
')
    [ -n "$OWNED_HEAD" ] && break
    sleep 0.2
done
if [ -z "$OWNED_HEAD" ]; then
    bad "exactly one owned headless output could be created"
else
    ok "owned headless output created ($OWNED_HEAD)"
    write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40' 'left = ["clock"]' "outputs = [\"$OWNED_HEAD\"]"
    reload >/dev/null 2>&1; sleep 2
    head_w=$(head_width "$OWNED_HEAD")
    check "$(bar_w) $(bar_h)" "$head_w 40" "the filter moves the test bar to the owned output"
    iso_hyprctl output remove "$OWNED_HEAD" >/dev/null 2>&1
    OWNED_HEAD=
    sleep 2
    check "$(bar_h)" none "removing the owned output removes its bar"
    shell_alive && ok "the shell survived removing the owned output" \
        || bad "the shell died removing the owned output"
    no_protocol_error "removing the owned output"
fi
iso_restore_focus
ORIG_WS=
write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40' 'left = ["clock"]'
reload >/dev/null 2>&1; sleep 2
check "$(bar_h)" 40 "restoring * maps the bar on the real output again"

kill -0 "$SHELL_PID" 2>/dev/null && ok "test shell still alive" || bad "test shell died"
hayu quit; check "$?" 0 "hayu quit exits 0"
wait "$SHELL_PID" 2>/dev/null
check "$(launcher_shown)" 0 "no launcher overlay after quit"
SHELL_PID=

iso_user_ok && ok "the user shell socket and PID are unchanged" || bad "the user shell changed"

if [ "$fail" = 0 ]; then
    say "RESULT: all $checks reload checks passed"
else
    say "RESULT: failures above ($checks checks)"
fi
exit "$fail"
