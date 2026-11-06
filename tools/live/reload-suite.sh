#!/usr/bin/env bash
# Live `hayu reload` suite: geometry, invalid-config survival, the
# launcher.enabled gate, and the restart-required notice.
#
#   cd <repo root>
#   tools/live/reload-suite.sh
#
# Starts its own shell against a temporary XDG_CONFIG_HOME, so it never reads
# or writes the caller's config.
set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT" || exit 1
BIN=${HAYU_BIN:-$ROOT/target/release/hayu}
WORK=$(mktemp -d "${TMPDIR:-/tmp}/hayu-reload-XXXXXX")
CFG=$WORK/config/hayu/config.toml
LOG=$WORK/hayu.log
fail=0
checks=0
SHELL_PID=

say() { printf '%s\n' "$*"; }
ok()  { checks=$((checks + 1)); say "PASS $*"; }
bad() { checks=$((checks + 1)); fail=1; say "FAIL $*"; }
check() { if [ "$1" = "$2" ]; then ok "$3"; else bad "$3: expected [$2] got [$1]"; fi; }
is_ge() { if [ "$1" -ge "$2" ]; then ok "$3 ($1 >= $2)"; else bad "$3: $1 < $2"; fi; }

bar() { # "h y" of the hayu layer
    hyprctl layers -j | python3 -c '
import json, sys
d = json.load(sys.stdin)
for mon, v in d.items():
    for lvl, layers in v["levels"].items():
        for l in layers:
            if l.get("namespace") == "hayu":
                print(l["h"], l["y"])
                sys.exit()
print("none none")
'
}
bar_h() { bar | cut -d" " -f1; }
bar_y() { bar | cut -d" " -f2; }
launcher_layers() { hyprctl layers -j 2>/dev/null | grep -c hayu-launcher; }
write_cfg() { printf '%s\n' "$@" > "$CFG"; }

cleanup() {
    [ -n "$SHELL_PID" ] && kill "$SHELL_PID" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

[ -x "$BIN" ] || { say "building hayu"; cargo build --release || exit 1; }
command -v hyprctl >/dev/null || { say "hyprctl is required"; exit 1; }

TOP32=('schema_version = 1' '' '[bar]' 'position = "top"' 'height = 32')
BOT40=('schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40')

"$BIN" quit >/dev/null 2>&1
sleep 1
mkdir -p "$(dirname "$CFG")"
write_cfg "${TOP32[@]}"
XDG_CONFIG_HOME="$WORK/config" "$BIN" run >"$LOG" 2>&1 &
SHELL_PID=$!
for _ in $(seq 1 50); do "$BIN" launcher close >/dev/null 2>&1 && break; sleep 0.1; done
sleep 1

check "$(bar_h)" 32 "initial bar height"
if [ "$(bar_y)" -lt 100 ]; then ok "initial bar is at the top (y=$(bar_y))"; else bad "initial bar is at the top (y=$(bar_y))"; fi

# Valid reload: geometry changes, no notice.
write_cfg "${BOT40[@]}"
out=$(XDG_CONFIG_HOME="$WORK/config" "$BIN" reload 2>&1); check "$?" 0 "reload with a valid config exits 0"
check "$out" "" "an applied-only reload prints no notice"
sleep 2
check "$(bar_h)" 40 "reload applied the new height"
is_ge "$(bar_y)" 700 "reload moved the bar to the bottom"

"$BIN" launcher toggle >/dev/null; check "$?" 0 "launcher toggles after reload"
sleep 1
check "$(launcher_layers)" 1 "launcher mapped after reload"
"$BIN" launcher toggle >/dev/null; sleep 1
check "$(launcher_layers)" 0 "launcher closed again"

# A restart-only key is reported while the rest of the file still applies.
write_cfg "${BOT40[@]}" 'outputs = ["DP-1"]' '' '[clock]' 'format = "%H"'
out=$(XDG_CONFIG_HOME="$WORK/config" "$BIN" reload 2>&1); check "$?" 0 "reload with a restart-only key exits 0"
case "$out" in
    *"restart required for: bar.outputs"*) ok "restart-only key is reported" ;;
    *) bad "restart-only key is reported (got: $out)" ;;
esac
sleep 1
check "$(bar_h)" 40 "restart-only reload kept the applied geometry"

# The running config now carries the restart-only outputs value; keep it in the
# later files so they compare equal.
BOT40_OUT=("${BOT40[@]}" 'outputs = ["DP-1"]')

# Invalid reload: rejected, running state untouched.
write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 500'
out=$(XDG_CONFIG_HOME="$WORK/config" "$BIN" reload 2>&1); code=$?
if [ "$code" -ne 0 ]; then ok "invalid config exits non-zero ($code)"; else bad "invalid config exits 0"; fi
case "$out" in *bar.height*) ok "invalid config names bar.height" ;; *) bad "invalid config error: $out" ;; esac
sleep 1
check "$(bar_h)" 40 "invalid reload kept the bar height"
is_ge "$(bar_y)" 700 "invalid reload kept the bar at the bottom"
"$BIN" launcher toggle >/dev/null; check "$?" 0 "launcher still opens after a rejected reload"
sleep 1
check "$(launcher_layers)" 1 "launcher mapped after a rejected reload"
"$BIN" launcher toggle >/dev/null; sleep 1

# launcher.enabled = false takes effect and closes an open overlay.
write_cfg "${BOT40_OUT[@]}" '' '[launcher]' 'enabled = false'
out=$(XDG_CONFIG_HOME="$WORK/config" "$BIN" reload 2>&1); check "$?" 0 "reload with launcher disabled exits 0"
case "$out" in *restart*) bad "launcher.enabled is applied, not reported ($out)" ;; *) ok "launcher.enabled is applied, not reported" ;; esac
sleep 1
"$BIN" launcher toggle 2>/dev/null; code=$?
if [ "$code" -ne 0 ]; then ok "disabled launcher rejects toggle ($code)"; else bad "disabled launcher accepted toggle"; fi
"$BIN" launcher open 2>/dev/null; code=$?
if [ "$code" -ne 0 ]; then ok "disabled launcher rejects open ($code)"; else bad "disabled launcher accepted open"; fi
sleep 1
check "$(launcher_layers)" 0 "disabled launcher left nothing mapped"

write_cfg "${BOT40_OUT[@]}" '' '[launcher]' 'enabled = true'
XDG_CONFIG_HOME="$WORK/config" "$BIN" reload >/dev/null 2>&1; sleep 1
"$BIN" launcher open >/dev/null; sleep 1
check "$(launcher_layers)" 1 "re-enabled launcher maps"
write_cfg "${BOT40_OUT[@]}" '' '[launcher]' 'enabled = false'
XDG_CONFIG_HOME="$WORK/config" "$BIN" reload >/dev/null 2>&1; sleep 1
check "$(launcher_layers)" 0 "reload closed the open overlay when disabled"

kill -0 "$SHELL_PID" 2>/dev/null && ok "shell still alive" || bad "shell died"
"$BIN" quit; check "$?" 0 "hayu quit exits 0"
wait "$SHELL_PID" 2>/dev/null
check "$(launcher_layers)" 0 "no launcher layer after quit"
if [ "$fail" = 0 ]; then
    say "RESULT: all $checks reload checks passed"
else
    say "RESULT: failures above ($checks checks)"
fi
exit "$fail"
