#!/usr/bin/env bash
# Live `hayu reload` suite: geometry, invalid-config survival, the
# launcher.enabled and bar.enabled gates, bar.outputs, and the
# restart-required notice.
#
#   cd <repo root>
#   tools/live/reload-suite.sh
#
# Starts its own shell against a temporary XDG_CONFIG_HOME, so it never reads
# or writes the caller's config. It creates and removes a headless output for
# the filter check.
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
made_headless=0

say() { printf '%s\n' "$*"; }
ok()  { checks=$((checks + 1)); say "PASS $*"; }
bad() { checks=$((checks + 1)); fail=1; say "FAIL $*"; }
check() { if [ "$1" = "$2" ]; then ok "$3"; else bad "$3: expected [$2] got [$1]"; fi; }
is_ge() { if [ "$1" -ge "$2" ] 2>/dev/null; then ok "$3 ($1 >= $2)"; else bad "$3: $1 < $2"; fi; }

bar_field() { # $1 = h|y of the hayu bar, "none" when absent
    hyprctl layers -j | python3 -c '
import json, sys
d = json.load(sys.stdin)
for mon, v in d.items():
    for lvl, layers in v["levels"].items():
        for l in layers:
            if l.get("namespace") == "hayu":
                print(l[sys.argv[1]])
                sys.exit()
print("none")
' "$1"
}
bar_h() { bar_field h; }
bar_y() { bar_field y; }
bar_on() { # "<monitor> <w> <h>" of the hayu bar, empty when none
    hyprctl layers -j | python3 -c '
import json, sys
d = json.load(sys.stdin)
for mon, v in d.items():
    for lvl, layers in v["levels"].items():
        for l in layers:
            if l.get("namespace") == "hayu":
                print(mon, l["w"], l["h"])
                sys.exit()
print("")
'
}
launcher_layers() { hyprctl layers -j 2>/dev/null | grep -c hayu-launcher; }
write_cfg() { printf '%s\n' "$@" > "$CFG"; }
reload() { XDG_CONFIG_HOME="$WORK/config" "$BIN" reload "$@"; }

cleanup() {
    if [ "$made_headless" = 1 ]; then
        for m in $(hyprctl monitors -j 2>/dev/null | python3 -c 'import json,sys; print(" ".join(x["name"] for x in json.load(sys.stdin) if x["name"].startswith("HEADLESS")))'); do
            hyprctl output remove "$m" >/dev/null 2>&1
        done
    fi
    [ -n "$SHELL_PID" ] && kill "$SHELL_PID" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

[ -x "$BIN" ] || { say "building hayu"; cargo build --release || exit 1; }
command -v hyprctl >/dev/null || { say "hyprctl is required"; exit 1; }

"$BIN" quit >/dev/null 2>&1
sleep 1
mkdir -p "$(dirname "$CFG")"
write_cfg 'schema_version = 1' '' '[bar]' 'position = "top"' 'height = 32'
XDG_CONFIG_HOME="$WORK/config" "$BIN" run >"$LOG" 2>&1 &
SHELL_PID=$!
for _ in $(seq 1 50); do "$BIN" launcher close >/dev/null 2>&1 && break; sleep 0.1; done
sleep 1

check "$(bar_h)" 32 "initial bar height"
if [ "$(bar_y)" -lt 100 ]; then ok "initial bar is at the top (y=$(bar_y))"; else bad "initial bar is at the top (y=$(bar_y))"; fi

# 1. Applied keys only: geometry changes and no notice is printed.
write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40'
out=$(reload 2>&1); check "$?" 0 "reload with a valid config exits 0"
check "$out" "" "an applied-only reload prints no notice"
sleep 2
check "$(bar_h)" 40 "reload applied the new height"
is_ge "$(bar_y)" 700 "reload moved the bar to the bottom"

"$BIN" launcher toggle >/dev/null; check "$?" 0 "launcher toggles after reload"
sleep 1
check "$(launcher_layers)" 1 "launcher mapped after reload"
"$BIN" launcher toggle >/dev/null; sleep 1
check "$(launcher_layers)" 0 "launcher closed again"

# 2. A key that still needs a restart is named, and the applied part lands.
write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40' 'left = ["clock"]' '' '[clock]' 'format = "%H"'
out=$(reload 2>&1); check "$?" 0 "reload with a restart-only key exits 0"
case "$out" in
    *"restart required for: bar.left"*) ok "restart-only key is reported" ;;
    *) bad "restart-only key is reported (got: $out)" ;;
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
is_ge "$(bar_y)" 700 "invalid reload kept the bar at the bottom"
"$BIN" launcher toggle >/dev/null; check "$?" 0 "launcher still opens after a rejected reload"
sleep 1
check "$(launcher_layers)" 1 "launcher mapped after a rejected reload"
"$BIN" launcher toggle >/dev/null; sleep 1

# 4. launcher.enabled = false is applied, not reported, and closes an overlay.
write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40' 'left = ["clock"]' '' '[launcher]' 'enabled = false'
out=$(reload 2>&1); check "$?" 0 "reload with launcher disabled exits 0"
case "$out" in *restart*) bad "launcher.enabled is applied, not reported ($out)" ;; *) ok "launcher.enabled is applied, not reported" ;; esac
sleep 1
"$BIN" launcher toggle 2>/dev/null; code=$?
if [ "$code" -ne 0 ]; then ok "disabled launcher rejects toggle ($code)"; else bad "disabled launcher accepted toggle"; fi
"$BIN" launcher open 2>/dev/null; code=$?
if [ "$code" -ne 0 ]; then ok "disabled launcher rejects open ($code)"; else bad "disabled launcher accepted open"; fi
sleep 1
check "$(launcher_layers)" 0 "disabled launcher left nothing mapped"

# 5. bar.outputs is applied: a filter with no match removes the bar, and the
#    launcher keeps working through the output list rather than a bar surface.
write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40' 'left = ["clock"]' 'outputs = ["NOPE-1"]' '' '[launcher]' 'enabled = true'
out=$(reload 2>&1); check "$?" 0 "outputs-only reload exits 0"
case "$out" in *restart*) bad "bar.outputs is applied, not reported ($out)" ;; *) ok "bar.outputs is applied, not reported" ;; esac
sleep 1
check "$(bar_h)" none "a filter with no matching output removes the bar"
"$BIN" launcher open >/dev/null; sleep 1
check "$(launcher_layers)" 1 "the launcher still opens without a bar"
"$BIN" launcher close >/dev/null; sleep 1

# 6. bar.enabled = false is applied too.
write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40' 'left = ["clock"]' 'enabled = false' '' '[launcher]' 'enabled = true'
out=$(reload 2>&1); check "$?" 0 "bar.enabled=false reload exits 0"
case "$out" in *restart*) bad "bar.enabled is applied, not reported ($out)" ;; *) ok "bar.enabled is applied, not reported" ;; esac
sleep 1
check "$(bar_h)" none "bar.enabled=false removes the bar"
"$BIN" launcher open >/dev/null; sleep 1
check "$(launcher_layers)" 1 "the launcher still opens with the bar disabled"
"$BIN" launcher close >/dev/null; sleep 1

write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40' 'left = ["clock"]' '' '[launcher]' 'enabled = true'
out=$(reload 2>&1); check "$?" 0 "re-enabling the bar exits 0"
sleep 2
check "$(bar_h)" 40 "the bar comes back at the configured height"

# 7. A named filter moves the bar to the matching output, including a
#    hotplugged one, and removes it again when that output goes away.
before=$(hyprctl monitors -j | python3 -c 'import json,sys; print(" ".join(m["name"] for m in json.load(sys.stdin)))')
hyprctl output create headless >/dev/null 2>&1
made_headless=1
sleep 2
headless=$(hyprctl monitors -j | BEFORE="$before" python3 -c '
import json, os, sys
before = set(os.environ["BEFORE"].split())
new = [m["name"] for m in json.load(sys.stdin) if m["name"] not in before]
print(new[0] if new else "")
')
if [ -z "$headless" ]; then
    bad "a headless output could be created"
else
    ok "a headless output was created ($headless)"
    write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40' 'left = ["clock"]' "outputs = [\"$headless\"]"
    reload >/dev/null 2>&1; sleep 2
    check "$(bar_on)" "$headless 1920 40" "the filter moves the bar to the matching output"
    check "$(bar_h)" 40 "the bar keeps its height on the filtered output"
    hyprctl output remove "$headless" >/dev/null 2>&1
    made_headless=0
    sleep 2
    check "$(bar_h)" none "removing that output removes its bar"
fi
write_cfg 'schema_version = 1' '' '[bar]' 'position = "bottom"' 'height = 40' 'left = ["clock"]'
reload >/dev/null 2>&1; sleep 2
check "$(bar_h)" 40 "restoring * maps the bar on the real output again"

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
