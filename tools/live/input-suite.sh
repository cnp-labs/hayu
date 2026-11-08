#!/usr/bin/env bash
# Live launcher suite: drives the real binary on a real Hyprland session with
# real input events, then reads the debug trace and `hyprctl layers -j`.
#
#   cd <repo root>
#   tools/live/input-suite.sh
#
# Or set HAYU_BIN and KEYIN_BIN to override the binaries. The script builds
# both automatically if they are missing.
#
# Requirements: running Hyprland, `hyprctl`, libxkbcommon data, and foot for
# the Terminal=true check. Does not read the user's config: it starts its own
# shell in a private runtime with XDG_CONFIG_HOME/XDG_DATA_HOME pointing at a
# temp directory. Every client command uses that private runtime, never the
# user's.
set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT" || exit 1
. "$ROOT/tools/live/isolation.sh"
BIN=${HAYU_BIN:-$ROOT/target/release/hayu}
KEYIN=${KEYIN_BIN:-$ROOT/tools/live/keyin/target/release/keyin}
iso_init
WORK=$(mktemp -d "${TMPDIR:-/tmp}/hayu-live-XXXXXX")
LOG=$WORK/hayu.log
ART=$WORK/art
fail=0
checks=0
SHELL_PID=
RT=
say() { printf '%s\n' "$*"; }
ok()  { checks=$((checks + 1)); say "PASS $*"; }
bad() { checks=$((checks + 1)); fail=1; say "FAIL $*"; }
check() { if [ "$1" = "$2" ]; then ok "$3"; else bad "$3: expected [$2] got [$1]"; fi; }

layers()     { iso_layer_count "$SHELL_PID" hayu-launcher; }
mark()       { wc -l < "$LOG"; }
seen()       { [ -f "$LOG" ] && tail -n +"$(( $1 + 1 ))" "$LOG" | grep -qF -- "$2"; }
wait_seen()  { local from=$1 text=$2 i; for i in $(seq 1 80); do seen "$from" "$text" && return 0; sleep 0.1; done; return 1; }
wait_layers(){ local want=$1 i; for i in $(seq 1 80); do [ "$(layers)" = "$want" ] && return 0; sleep 0.1; done; return 1; }

# Only a window created by this run and carrying the private runtime may be
# closed; the user's terminals are never in that set.
client_addrs() {
    iso_hyprctl -j clients | iso_json \
        'import json,sys; print(" ".join(c["address"] for c in json.load(sys.stdin)))'
}
close_owned_new_clients() {
    local before=$1 addr
    for addr in $(iso_hyprctl -j clients | iso_json '
import json, os, sys
before = set(sys.argv[1].split())
rt = sys.argv[2]
for c in json.load(sys.stdin):
    if c["address"] in before:
        continue
    try:
        env = open("/proc/%d/environ" % c["pid"], "rb").read().split(b"\0")
    except OSError:
        continue
    if ("XDG_RUNTIME_DIR=" + rt).encode() in env:
        print(c["address"])
' "$before" "$RT"); do
        iso_hyprctl dispatch closewindow "address:$addr" >/dev/null 2>&1
    done
}

cleanup() {
    iso_evidence "$LOG" input-suite
    [ -n "$RT" ] && iso_destroy "$RT"
    [ -n "${ORIG_WS:-}" ] && iso_restore_focus >/dev/null 2>&1
    iso_user_ok || printf 'ISOLATION ABORT: USER SHELL CHANGED\n' >&2
    rm -rf "$WORK"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

[ -x "$BIN" ] || { say "building hayu"; cargo build --release || exit 1; }
[ -x "$KEYIN" ] || {
    say "building the key injector"
    cargo build --release --offline \
        --manifest-path tools/live/keyin/Cargo.toml || exit 1
}
command -v hyprctl >/dev/null || { say "hyprctl is required"; exit 1; }

mkdir -p "$ART" "$WORK/config/hayu" "$WORK/data/applications"
# Fixtures write into their own per-run artifact directory, not /tmp/hayu-live,
# so a stale marker cannot make a check pass.
for f in tools/live/data/applications/*.desktop; do
    sed "s#/tmp/hayu-live#$ART#g" "$f" >"$WORK/data/applications/$(basename "$f")"
done
cat > "$WORK/config/hayu/config.toml" <<'EOF'
schema_version = 1

[bar]
position = "top"
height = 32
EOF

iso_capture_focus
RT=$(iso_rt hr-in) || exit 1
iso_start "$RT" "$LOG" env XDG_CONFIG_HOME="$WORK/config" XDG_DATA_HOME="$WORK/data" \
    HAYU_DEBUG_KEYS=1 HAYU_DEBUG_SCAN=1 "$BIN" run
SHELL_PID=$ISO_PID
for _ in $(seq 1 50); do iso_hayu "$RT" launcher close >/dev/null 2>&1 && break; sleep 0.1; done
iso_hayu "$RT" launcher close >/dev/null 2>&1 || { say "shell did not come up"; exit 1; }
iso_assert_env "$SHELL_PID" "$RT" || { say "test shell has the wrong runtime"; exit 1; }

# 1-5: open, filter, navigate, launch, close.
iso_hayu "$RT" launcher toggle >/dev/null
wait_layers 1 && ok "toggle maps the overlay" || bad "toggle maps the overlay"
from=$(mark); "$KEYIN" "type:hayu test"
wait_seen "$from" 'results=3' \
    && ok "typed query filters to three entries" \
    || bad "typed query filters to three entries"
from=$(mark); "$KEYIN" press:Down
wait_seen "$from" 'sel=1' && ok "Down moves the selection" || bad "Down moves the selection"
from=$(mark); "$KEYIN" press:Return
wait_layers 0 && ok "Enter launches and closes the overlay" || bad "Enter launches and closes the overlay"
if [ -f "$ART/one" ] || [ -f "$ART/two" ] || [ -f "$ART/three" ]; then
    ok "the launched Exec actually ran"
else
    bad "the launched Exec actually ran"
fi

# 6-8: reopen, empty results, Escape.
iso_hayu "$RT" launcher toggle >/dev/null
wait_layers 1 && ok "reopening after a launch maps the surface again" || bad "reopening maps the surface again"
from=$(mark); "$KEYIN" "type:zzzzz"
wait_seen "$from" 'results=0' && ok "unmatched query yields no results" || bad "unmatched query yields no results"
from=$(mark); "$KEYIN" press:Escape
wait_layers 0 && ok "Escape closes the overlay" || bad "Escape closes the overlay"

# 9-11: query reset on reopen, held key repeats, Escape from query.
iso_hayu "$RT" launcher toggle >/dev/null; wait_layers 1
from=$(mark); "$KEYIN" "type:hayu test"
wait_seen "$from" 'results=3' && ok "reopening resets the previous query" || bad "reopening resets the previous query"
from=$(mark); "$KEYIN" hold:BackSpace; sleep 1
repeats=$(tail -n +"$((from + 1))" "$LOG" | grep -c 'Changed query=')
if [ "$repeats" -ge 3 ]; then ok "held BackSpace repeats ($repeats changes)"; else bad "held BackSpace repeats ($repeats)"; fi
"$KEYIN" press:Escape; wait_layers 0

# 12-14: Terminal=true, failing Exec, quit.
if command -v foot >/dev/null; then
    before_clients=$(client_addrs)
    iso_hayu "$RT" launcher toggle >/dev/null; wait_layers 1
    from=$(mark); "$KEYIN" "type:hayu terminal touch"
    wait_seen "$from" 'results=1'
    from=$(mark); "$KEYIN" press:Return; wait_layers 0; sleep 1
    if [ -f "$ART/terminal" ]; then
        ok "Terminal=true entry runs in foot and closes the overlay"
    else
        bad "Terminal=true entry runs in foot and closes the overlay"
    fi
    close_owned_new_clients "$before_clients"
else
    say "SKIP Terminal=true check: foot is not installed"
fi

iso_hayu "$RT" launcher toggle >/dev/null; wait_layers 1
from=$(mark); "$KEYIN" "type:hayu failing"
wait_seen "$from" 'results=1'
from=$(mark); "$KEYIN" press:Return; sleep 1
found=$(tail -n +"$((from + 1))" "$LOG" | grep -ciE "cannot launch|no such file")
if [ "$(layers)" = 1 ] && [ "$found" -ge 1 ]; then
    ok "a failing Exec keeps the overlay open and shows the error"
else
    bad "a failing Exec keeps the overlay open and shows the error (found=$found layers=$(layers))"
fi
"$KEYIN" press:Escape; wait_layers 0

iso_hayu "$RT" quit; check "$?" 0 "hayu quit exits 0"
wait "$SHELL_PID" 2>/dev/null
check "$(layers)" 0 "no launcher layer after quit"
if grep -qiE "protocol error|unknown opcode" "$LOG"; then
    bad "no protocol error in the log"
else
    ok "no protocol error in the log"
fi
iso_user_ok && ok "the user shell and socket are unchanged" || bad "the user shell and socket are unchanged"

if [ "$fail" = 0 ]; then
    say "RESULT: all $checks live checks passed"
else
    say "RESULT: failures above ($checks checks)"
fi
exit "$fail"
