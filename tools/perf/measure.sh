#!/usr/bin/env bash
# Hayu runtime measurement probe, retained for repeatable before/after checks.
# Writes under /tmp/opencode by default and never reads the user's config.
#
#   tools/perf/measure.sh                      # warm build then measure
#   HAYU_SKIP_BUILD=1 tools/perf/measure.sh    # use existing target binary
#   HAYU_BIN=/path/hayu HAYU_SKIP_BUILD=1 tools/perf/measure.sh
#   HAYU_TRAY_SMOKE=1 tools/perf/measure.sh    # opt in to the private-bus tray suite
#
# Each live scenario has a timeout and records failure instead of waiting
# forever. Only the shell this script spawned is ever signalled.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=${HAYU_ROOT:-$(cd "$HERE/../.." && pwd)}
BIN=${HAYU_BIN:-$ROOT/target/release/hayu}
ART=${HAYU_BASELINE_ART:-/tmp/opencode/hayu-baseline/measure-$(date -u +%Y%m%dT%H%M%SZ)}
KEYIN=${KEYIN_BIN:-$ROOT/tools/live/keyin/target/release/keyin}
mkdir -p "$ART" || exit 1
STATUS=$ART/status.tsv
printf 'step\tstatus\tdetail\n' > "$STATUS"
cp "$HERE/measure.sh" "$ART/measure.sh" 2>/dev/null || true
PID=
BUS_PID=
WORK=

record() { printf '%s\t%s\t%s\n' "$1" "$2" "$3" >> "$STATUS"; }
say() { printf '%s\n' "$*"; }
abort() { say "ABORT: $*"; record abort "$1" "$*"; exit 2; }
cleanup() {
    if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        kill -TERM "$PID" 2>/dev/null
        wait "$PID" 2>/dev/null
    fi
    [ -n "$BUS_PID" ] && kill -TERM "$BUS_PID" 2>/dev/null
    [ -n "$WORK" ] && rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

mkdir -p "$ART" || abort "cannot create $ART"
if [ "${HAYU_SKIP_BUILD:-0}" != "1" ]; then
    if [ -x "$BIN" ] && [ -z "${HAYU_BIN:-}" ]; then
        cp -p "$BIN" "$ART/hayu-before" || abort "cannot preserve $BIN"
    fi
    ( cd "$ROOT" && timeout 600 cargo build --release ) > "$ART/build.log" 2>&1 \
        || abort "cargo build --release failed; see $ART/build.log"
    [ -n "${HAYU_BIN:-}" ] || BIN="$ROOT/target/release/hayu"
fi
[ -x "$BIN" ] || abort "$BIN is not executable"
CURRENT_HEAD=$(git -C "$ROOT" rev-parse HEAD)
BASE_HEAD=${HAYU_BASELINE_HEAD:-$CURRENT_HEAD}
[ -x "$KEYIN" ] || say "WARN keyin missing at $KEYIN; launcher scenario will be skipped"
command -v jq >/dev/null || abort "jq is required"
command -v hyprctl >/dev/null || abort "hyprctl is required"

{
    echo "utc=$(date -u +%FT%TZ)"
    echo "baseline_head=$BASE_HEAD"
    echo "skip_build=${HAYU_SKIP_BUILD:-0}"
    echo "build_log=$([ -f "$ART/build.log" ] && echo "$ART/build.log" || echo none)"
    echo "worktree_head=$CURRENT_HEAD"
    echo "binary=$BIN"
    echo "binary_sha256=$(sha256sum "$BIN" | cut -d' ' -f1)"
    echo "binary_bytes=$(stat -c %s "$BIN")"
    echo "keyin=$KEYIN"
    echo "keyin_sha256=$(sha256sum "$KEYIN" 2>/dev/null | cut -d' ' -f1)"
    echo "loadavg_start=$(cut -d' ' -f1-3 /proc/loadavg)"
    echo "git_status_begin"
    git -C "$ROOT" status --porcelain
    echo "git_status_end"
} > "$ART/probe.txt"

# ---- environment ----------------------------------------------------------
{
    echo "## kernel"; uname -a
    echo "## distro"; head -6 /etc/os-release
    echo "## cpu"; lscpu | grep -E 'Model name|^CPU\(s\)|Thread|Core|Socket|CPU max MHz|CPU min MHz'
    echo "## memory"; free -h
    echo "## gpu"; lspci -nnk 2>/dev/null | grep -A3 -E 'VGA compatible|3D controller|Display controller'
    echo "## session"; echo "XDG_SESSION_TYPE=$XDG_SESSION_TYPE XDG_CURRENT_DESKTOP=$XDG_CURRENT_DESKTOP WAYLAND_DISPLAY=$WAYLAND_DISPLAY XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR"
    echo "## rust"; rustc --version; cargo --version
    echo "## tools"
    if [ -x "$KEYIN" ]; then echo "keyin=present $KEYIN"; else echo "keyin=MISSING $KEYIN"; fi
    for t in jq python3 hyprctl fcitx5 busctl dbus-run-session dbus-monitor glxinfo eglinfo vulkaninfo intel_gpu_top nvidia-smi wgpu-info; do
        if command -v "$t" >/dev/null; then echo "$t=present $(command -v "$t")"; else echo "$t=MISSING"; fi
    done
} > "$ART/environment.txt"

# ---- compositor discovery (JSON only; abort on stale signature) -----------
INST=$(hyprctl instances -j 2>/dev/null) || abort "hyprctl instances -j failed"
printf '%s' "$INST" | jq -e 'type == "array" and length > 0' >/dev/null 2>&1 \
    || abort "hyprctl instances -j did not return a non-empty JSON array"
SIG=$(printf '%s' "$INST" | jq -r '.[0].instance')
[ -n "$SIG" ] && [ "$SIG" != "null" ] || abort "no Hyprland instance signature in JSON"
export HYPRLAND_INSTANCE_SIGNATURE=$SIG
printf '%s\n' "$INST" > "$ART/hyprland-instances.json"
hyprctl version > "$ART/hyprland-version.txt" 2>&1 || abort "hyprctl version with $SIG failed"
hyprctl -j monitors > "$ART/monitors.json" 2>"$ART/monitors.err" || abort "hyprctl -j monitors with $SIG failed"
jq -e 'type == "array" and length > 0' "$ART/monitors.json" >/dev/null 2>&1 \
    || abort "monitor JSON is invalid or empty"
MON_COUNT=$(jq 'length' "$ART/monitors.json")
MON_SCALES=$(jq -r '.[] | "\(.name) \(.width)x\(.height) scale=\(.scale) transform=\(.transform) focused=\(.focused)"' "$ART/monitors.json")
hyprctl layers > "$ART/layers-before.txt" 2>&1
say "Hyprland signature: $SIG"
say "monitors: $MON_COUNT"
printf '%s\n' "$MON_SCALES"

# ---- default dependency counts (no network, no build) ---------------------
{
  if [ "$BASE_HEAD" = "$CURRENT_HEAD" ]; then
    printf 'cargo_lock_packages=%s\n' "$(grep -c '^name = ' "$ROOT/Cargo.lock")"
    printf 'normal_tree_unique=%s\n' "$(cd "$ROOT" && timeout 60 cargo tree --locked --offline -e normal --prefix none 2>/dev/null | sort -u | wc -l)"
  else
    printf 'cargo_lock_packages=%s (from git %s)\n' "$(git -C "$ROOT" show "$BASE_HEAD:Cargo.lock" | grep -c '^name = ')" "$BASE_HEAD"
    printf 'normal_tree_unique=unavailable (worktree advanced past %s; not recomputed)\n' "$BASE_HEAD"
  fi
} > "$ART/deps.txt"

# ---- isolated startup, first surface, 3x10 s idle -------------------------
WORK=$(mktemp -d "$HERE/session.XXXXXX") || abort "mktemp failed"
mkdir -p "$WORK/config/hayu" "$WORK/data/applications"
cp "$ROOT"/tools/live/data/applications/*.desktop "$WORK/data/applications/"
cat > "$WORK/config/hayu/config.toml" <<'TOML'
schema_version = 1
[bar]
position = "top"
height = 32
left = ["workspaces"]
center = ["clock"]
right = ["tray"]
TOML
cp "$WORK/config/hayu/config.toml" "$ART/config-used.toml"

mapfile -t BUS < <(timeout 10 dbus-daemon --session --fork --print-address=1 --print-pid=1)
[ "${#BUS[@]}" -ge 2 ] || abort "could not start a private session bus"
DBUS_ADDR=${BUS[0]}
BUS_PID=${BUS[1]}
export DBUS_SESSION_BUS_ADDRESS=$DBUS_ADDR

START_NS=$(date +%s%N)
XDG_CONFIG_HOME="$WORK/config" XDG_DATA_HOME="$WORK/data" \
HAYU_DEBUG_KEYS=1 HAYU_DEBUG_SCAN=1 \
    "$BIN" run > "$ART/hayu.log" 2>&1 &
PID=$!
say "shell pid=$PID"

bar_present() {
    local out
    out=$(hyprctl layers -j 2>/dev/null) || return 1
    printf '%s' "$out" | jq -e --argjson pid "$PID" \
        '[.. | objects | select(.namespace? == "hayu" and .pid? == $pid)] | length > 0' >/dev/null 2>&1
}
launcher_present() {
    local out
    out=$(hyprctl layers -j 2>/dev/null) || return 1
    printf '%s' "$out" | jq -e --argjson pid "$PID" \
        '[.. | objects | select(.namespace? == "hayu-launcher" and .pid? == $pid)] | length > 0' >/dev/null 2>&1
}
FIRST_MS=
for _ in $(seq 1 200); do
    if ! kill -0 "$PID" 2>/dev/null; then
        record shell_start fail "process exited before first surface"
        break
    fi
    if bar_present; then
        NOW_NS=$(date +%s%N)
        FIRST_MS=$(( (NOW_NS - START_NS) / 1000000 ))
        break
    fi
    sleep 0.05
done
if [ -n "$FIRST_MS" ]; then
    record first_hayu_surface_ms ok "$FIRST_MS (coarse upper bound: 50 ms poll of hyprctl layers -j, not presentation latency)"
    hyprctl layers -j > "$ART/layers-first-surface.json" 2>&1
    jq -r 'to_entries[] | .key as $m | .value.levels | to_entries[] | .value[] | select(.namespace == "hayu") | "\($m) xywh=\(.x),\(.y) \(.w)x\(.h) namespace=\(.namespace) pid=\(.pid)"' \
        "$ART/layers-first-surface.json" || true
    say "first observed hayu layer (upper bound): ${FIRST_MS} ms"
else
    record first_hayu_surface_ms fail "no hayu layer within 10 s"
    say "WARN no hayu layer appeared; see $ART/hayu.log"
fi

sleep 1
HZ=$(getconf CLK_TCK)
printf 'sample\tuser_ticks\tsys_ticks\tcpu_user_sec\tcpu_sys_sec\tcpu_pct_of_one_core\tvoluntary_delta\tnonvoluntary_delta\trss_kb\tpss_kb\tthreads\n' > "$ART/idle.tsv"
ctxt() { awk '/^voluntary_ctxt_switches:/{v=$2} /^nonvoluntary_ctxt_switches:/{n=$2} END{print v+0, n+0}' "/proc/$PID/status" 2>/dev/null; }
for n in 1 2 3; do
    if ! kill -0 "$PID" 2>/dev/null; then
        record "idle_sample_$n" fail "process not running"
        break
    fi
    read -r U0 S0 < <(awk '{print $14, $15}' "/proc/$PID/stat")
    read -r CV0 CN0 < <(ctxt)
    sleep 10
    if ! kill -0 "$PID" 2>/dev/null; then
        record "idle_sample_$n" fail "process exited during sample"
        break
    fi
    read -r U1 S1 < <(awk '{print $14, $15}' "/proc/$PID/stat")
    read -r CV1 CN1 < <(ctxt)
    DU=$((U1-U0)); DS=$((S1-S0)); DV=$((CV1-CV0)); DN=$((CN1-CN0))
    RSS=$(awk '/^VmRSS:/{print $2}' "/proc/$PID/status")
    THR=$(awk '/^Threads:/{print $2}' "/proc/$PID/status")
    PSS=$(awk '/^Pss:/{print $2}' "/proc/$PID/smaps_rollup" 2>/dev/null)
    PSS=${PSS:-NA}
    awk -v n="$n" -v du="$DU" -v ds="$DS" -v dv="$DV" -v dn="$DN" \
        -v rss="$RSS" -v pss="$PSS" -v thr="$THR" -v hz="$HZ" 'BEGIN{
            printf "%s\t%d\t%d\t%.3f\t%.3f\t%.3f\t%d\t%d\t%s\t%s\t%s\n",
                n,du,ds,du/hz,ds/hz,(du+ds)/hz/10*100,dv,dn,rss,pss,thr
        }' >> "$ART/idle.tsv"
    say "idle sample $n: user+sys $((DU+DS)) ticks, ctxt +$DV/+$DN, RSS ${RSS} kB, PSS ${PSS} kB, ${THR} threads"
done
record idle_samples ok "3 x 10 s from /proc/$PID; ctxt switches are not wakeups"

# ---- launcher open / type / close (keyin) ---------------------------------
if [ -x "$KEYIN" ] && [ -n "$FIRST_MS" ]; then
    printf 'event\tstatus\tvalue\n' > "$ART/launcher.tsv"
    T0=$(date +%s%N)
    timeout 10 "$BIN" launcher open > "$ART/launcher-open.out" 2>&1
    RC=$?
    record launcher_open "$([ "$RC" = 0 ] && echo ok || echo fail)" "exit=$RC"
    MAP_MS=
    for _ in $(seq 1 100); do
        if launcher_present; then
            NOW=$(date +%s%N); MAP_MS=$(( (NOW-T0)/1000000 )); break
        fi
        sleep 0.05
    done
    printf 'launcher_map\t%s\t%s ms\n' "$([ -n "$MAP_MS" ] && echo ok || echo fail)" "${MAP_MS:-NA}" >> "$ART/launcher.tsv"
    record launcher_map_ms "$([ -n "$MAP_MS" ] && echo ok || echo fail)" "${MAP_MS:-NA} (client command to launcher layer; poll 50 ms)"
    LOG_MARK=$(wc -l < "$ART/hayu.log")
    T1=$(date +%s%N)
    timeout 10 "$KEYIN" "type:hayu test" > "$ART/keyin-type.out" 2>&1
    RC=$?
    record keyin_type "$([ "$RC" = 0 ] && echo ok || echo fail)" "exit=$RC"
    FILTER_MS=
    for _ in $(seq 1 100); do
        if tail -n +$((LOG_MARK+1)) "$ART/hayu.log" | grep -q 'results=3'; then
            NOW=$(date +%s%N); FILTER_MS=$(( (NOW-T1)/1000000 )); break
        fi
        sleep 0.02
    done
    printf 'typed_filter_seen\t%s\t%s ms\n' "$([ -n "$FILTER_MS" ] && echo ok || echo fail)" "${FILTER_MS:-NA}" >> "$ART/launcher.tsv"
    record typed_filter_seen "$([ -n "$FILTER_MS" ] && echo ok || echo fail)" "${FILTER_MS:-NA} ms (keyin type to results=3 in log; not input latency)"
    sleep 1
    T2=$(date +%s%N)
    timeout 10 "$KEYIN" press:Escape > "$ART/keyin-escape.out" 2>&1
    RC=$?
    record keyin_escape "$([ "$RC" = 0 ] && echo ok || echo fail)" "exit=$RC"
    CLOSE_MS=
    for _ in $(seq 1 100); do
        if ! launcher_present; then NOW=$(date +%s%N); CLOSE_MS=$(( (NOW-T2)/1000000 )); break; fi
        sleep 0.05
    done
    printf 'launcher_close\t%s\t%s ms\n' "$([ -n "$CLOSE_MS" ] && echo ok || echo fail)" "${CLOSE_MS:-NA}" >> "$ART/launcher.tsv"
    record launcher_close_ms "$([ -n "$CLOSE_MS" ] && echo ok || echo fail)" "${CLOSE_MS:-NA} (Escape to layer gone; poll 50 ms)"
    grep -o 'launcher frame [0-9.]* ms' "$ART/hayu.log" | awk '{print $3}' > "$ART/frame-ms.txt" || true
    grep -o 'desktop scan: [0-9]* entries in [0-9.]* ms' "$ART/hayu.log" > "$ART/scan-lines.txt" || true
    if [ -s "$ART/frame-ms.txt" ]; then
        sort -n "$ART/frame-ms.txt" | awk '{a[NR]=$1} END{printf "min=%.2f median=%.2f max=%.2f n=%d\n",a[1],a[int((NR+1)/2)],a[NR],NR}' > "$ART/frame-summary.txt"
        record debug_launcher_frame ok "$(cat "$ART/frame-summary.txt") (HAYU_DEBUG_KEYS draw timing)"
    else
        record debug_launcher_frame fail "no 'launcher frame' lines"
    fi
    [ -s "$ART/scan-lines.txt" ] && record debug_desktop_scan ok "$(tail -1 "$ART/scan-lines.txt")"
    hyprctl layers > "$ART/layers-after-launcher.txt" 2>&1
else
    record launcher_scenario skip "keyin unavailable or shell never mapped"
fi

# ---- reload ---------------------------------------------------------------
printf '\n[clock]\nformat = "%%H:%%M:%%S"\n' >> "$WORK/config/hayu/config.toml"
T0=$(date +%s%N)
timeout 10 "$BIN" reload > "$ART/reload.out" 2>&1
RC=$?
T1=$(date +%s%N)
RELOAD_MS=$(( (T1-T0)/1000000 ))
record reload "$([ "$RC" = 0 ] && echo ok || echo fail)" "exit=$RC elapsed=${RELOAD_MS}ms output=$(tr '\n' ' ' < "$ART/reload.out")"
sleep 0.3
if bar_present; then record reload_bar_survived ok "bar layer still present"; else record reload_bar_survived fail "bar layer gone"; fi
grep -E 'restart required|error|failed' "$ART/hayu.log" | tail -5 > "$ART/reload-log-lines.txt" || true

# ---- stop isolated shell before the tray suite uses the same socket -------
kill -TERM "$PID" 2>/dev/null
WAIT_RC=NA
for _ in $(seq 1 50); do
    if ! kill -0 "$PID" 2>/dev/null; then WAIT_RC=0; break; fi
    sleep 0.1
done
if [ "$WAIT_RC" = 0 ]; then wait "$PID" 2>/dev/null; WAIT_RC=$?; fi
record shell_sigterm_exit "$([ "$WAIT_RC" = 0 ] || [ "$WAIT_RC" = 143 ] && echo ok || echo fail)" "exit=$WAIT_RC within 5 s (143 = SIGTERM)"
sleep 1
if bar_present; then record shell_layer_cleanup fail "owned hayu layer still present"; else record shell_layer_cleanup ok "no owned hayu layer after SIGTERM"; fi
PID=
cp "$ART/hayu.log" "$ART/hayu-final.log" 2>/dev/null || true

# ---- tray smoke: existing suite, private bus ------------------------------
if [ "${HAYU_TRAY_SMOKE:-0}" != "1" ]; then
    record tray_suite skip "HAYU_TRAY_SMOKE != 1"
elif [ ! -x "$ROOT/tools/live/tray-suite.sh" ] || ! command -v fcitx5 >/dev/null || ! command -v dbus-run-session >/dev/null; then
    record tray_suite skip "tray-suite.sh, fcitx5, or dbus-run-session missing"
else
    BEFORE_FC=$(pgrep -x fcitx5 | tr '\n' ' ')
    T0=$(date +%s%N)
    HAYU_BIN="$BIN" timeout 120 "$ROOT/tools/live/tray-suite.sh" > "$ART/tray-suite.log" 2>&1
    RC=$?
    T1=$(date +%s%N)
    AFTER_FC=$(pgrep -x fcitx5 | tr '\n' ' ')
    record tray_suite "$([ "$RC" = 0 ] && echo ok || echo fail)" "exit=$RC elapsed=$(( (T1-T0)/1000000 ))ms $(grep -E '^RESULT:' "$ART/tray-suite.log" | tail -1)"
    record tray_fcitx_leak "$([ "$BEFORE_FC" = "$AFTER_FC" ] && echo ok || echo fail)" "before=[$BEFORE_FC] after=[$AFTER_FC]"
fi

if pgrep -a hayu >/dev/null 2>&1; then
    record leftover_hayu fail "$(pgrep -a hayu | tr '\n' ';')"
else
    record leftover_hayu ok "none"
fi
record loadavg_end ok "$(cut -d' ' -f1-3 /proc/loadavg)"

# ---- report ---------------------------------------------------------------
PY=python3
"$PY" - "$ART" "$MON_COUNT" "$MON_SCALES" "$ROOT" <<'PY'
import csv, pathlib, statistics, sys
art = pathlib.Path(sys.argv[1]); mon_count = sys.argv[2]; mon_scales = sys.argv[3]; root = sys.argv[4]
def read(name, default=""):
    p = art / name
    return p.read_text().strip() if p.exists() else default
def tsv(name):
    p = art / name
    if not p.exists(): return []
    return list(csv.DictReader(p.open(), delimiter="\t"))
status = tsv("status.tsv")
idle = tsv("idle.tsv")
frames = [float(x) for x in read("frame-ms.txt").splitlines() if x.strip()]
scan = read("scan-lines.txt").splitlines()
deps = read("deps.txt")
env = read("environment.txt")
probe = read("probe.txt")
hyprver = read("hyprland-version.txt")
def tab(rows, cols):
    if not rows: return "_no data_\n"
    out = ["| " + " | ".join(cols) + " |", "|" + "|".join("---" for _ in cols) + "|"]
    for r in rows:
        out.append("| " + " | ".join(str(r.get(c, "")) for c in cols) + " |")
    return "\n".join(out) + "\n"
idle_cols = ["sample","cpu_user_sec","cpu_sys_sec","cpu_pct_of_one_core","voluntary_delta","nonvoluntary_delta","rss_kb","pss_kb","threads"]
cpu_ticks = " ".join(f"{r['sample']}:{int(r['user_ticks'])+int(r['sys_ticks'])}" for r in idle) or "NA"
frame_line = read("frame-summary.txt", "unavailable")
reload_line = next((r["detail"] for r in status if r["step"] == "reload"), "unavailable")
first_line = next((r["detail"] for r in status if r["step"] == "first_hayu_surface_ms"), "unavailable")
report = f"""# Hayu runtime measurement

Status: measured on the live session; no source or tracked file was changed.
This is M0 bar/launcher evidence, not a V1 profile and not a T-01..T-20 result.

Binary provenance is in `probe.txt` (`baseline_head`, `worktree_head`,
`binary_sha256`). The measured binary is not rebuilt by the probe when
`HAYU_SKIP_BUILD=1`.

## Probe identity

```
{probe}
```

## Environment

```
{env}
```

Hyprland version:

```
{hyprver}
```

Monitors: {mon_count}
```
{mon_scales}
```

## Build and size

| metric | value |
|---|---|
| baseline binary | `{next((line for line in probe.splitlines() if line.startswith('binary=')), '')}` |
| sha256 | `{next((line for line in probe.splitlines() if line.startswith('binary_sha256=')), '')}` |
| bytes | {next((line for line in probe.splitlines() if line.startswith('binary_bytes=')), '')} |
| `cargo build --release` | see `build.log`; skipped when `HAYU_SKIP_BUILD=1` |
| Cargo.lock packages / normal tree | `{deps.replace(chr(10), "; ")}` |

The probe never runs a clean rebuild; it builds warm at most, and preserves the
pre-build binary as `hayu-before` when the default target path is used.

## Runtime: isolated start and first observed layer

Config was a temp `XDG_CONFIG_HOME` with `left=["workspaces"]`,
`center=["clock"]`, `right=["tray"]`; the user config was neither read nor
modified. The shell ran with a private session bus.

| metric | value |
|---|---|
| first observed hayu layer (upper bound, not presentation) | {first_line} |
| monitor count | {mon_count} |
| load average during probe | see `probe.txt` loadavg_start and `status.tsv` loadavg_end |

Load averages are recorded in `probe.txt` (`loadavg_start`) and `status.tsv`
(`loadavg_end`). Treat command-to-layer values and frame times as upper bounds
on a shared machine, not universal minima.

## Idle bar, 3 x 10 s, `/proc/<pid>`

CPU ticks are process-wide deltas at `CLK_TCK=100` (10 ms resolution), so a
0-tick sample means less than 10 ms of CPU in 10 s. Context-switch columns are
voluntary/nonvoluntary switches, **not wakeups**.

CPU ticks per sample: `{cpu_ticks}`

{tab(idle, idle_cols)}
## Launcher scenario (open, type, Escape)

Driven through the control socket and the existing `keyin` virtual keyboard;
the desktop catalogue came from `tools/live/data/applications`.

{tab(tsv("launcher.tsv"), ["event","status","value"])}
Debug-only frame timing (`HAYU_DEBUG_KEYS=1`): `{frame_line}`.

Desktop scan line: `{scan[-1] if scan else 'unavailable'}`.

No input latency is claimed: the table reports command-to-layer polling and
log observation, not input-to-present latency.

## Reload

`{reload_line}`

## Tray smoke (opt-in)

With `HAYU_TRAY_SMOKE=1`, the existing `tools/live/tray-suite.sh` runs on its
own `dbus-run-session` / private StatusNotifierWatcher with fcitx5. Result:

{tab([r for r in status if r["step"].startswith("tray")], ["step","status","detail"])}
## Measurements that are unavailable

- Multi-monitor behaviour beyond the default placement on {mon_count} monitor(s): not measured; scales were not changed.
- GPU profiling tools: see the `## tools` section in environment.txt for which are missing.
- Wakeup counts, input-to-present latency, per-frame damage/present timing: not emitted by the M0 binary.
- Clean-build time, package size on distro, and V1 workload profiles: not measured.

## Exact commands and exit statuses

Artifacts in this directory: `status.tsv` (step, status, detail), `hayu.log`,
`hayu-final.log`, `layers-*.txt`, `monitors.json`, `idle.tsv`,
`launcher.tsv`, `tray-suite.log`, `reload.out`.

```bash
HAYU_SKIP_BUILD=1 {root}/tools/perf/measure.sh
# Or any binary:
HAYU_BIN=/path/to/hayu HAYU_SKIP_BUILD=1 {root}/tools/perf/measure.sh
```

{tab(status, ["step","status","detail"])}
"""
(art / "baseline-report.md").write_text(report)
print(report)
PY

say "artifacts: $ART"
# A recorded failure must also fail automation; skipped probes are not failures.
if awk -F '\t' 'NR > 1 && $2 == "fail" { found = 1 } END { exit !found }' "$STATUS"; then
    exit 1
fi
