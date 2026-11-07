# Hayu before/after comparison — M0 bar/launcher

Measured 2026-11-07 UTC on one quiet-ish live Hyprland session. No builds, no
user config read or written, no commits, no docs changes.

## Artifacts

- Before: `/tmp/opencode/hayu-comparison/before/`
- After: `/tmp/opencode/hayu-comparison/after/`
- Live suites: `/tmp/opencode/hayu-comparison/live/`
- Preserved before binary: `/tmp/opencode/hayu-baseline/hayu-before`
- Preserved measured after binary: `/tmp/opencode/hayu-baseline/hayu-after-740569b`
  and `/tmp/opencode/hayu-comparison/after/hayu-measured`
- Retained tool: `/home/san/Documents/cnp/hayu/tools/perf/measure.sh`,
  sha256 `736efe7f5fd075629349496b4a565204d9ff31a92c04f062e5efbd3849a14933`

## Identity

| item | before | after |
|---|---|---|
| source HEAD | `ab9e678204ca4224468368c5d1e044cf649b1413` | `740569bdac141f68269df6620c5b0148159b9396` |
| binary | `/tmp/opencode/hayu-baseline/hayu-before` | `target/release/hayu` |
| sha256 | `35f353fa58b4200c1b3077d381d9bca423cd7fae7c77e2d1d57d359e8bd2d542` | `8092aee8312ad353a4aa18f81b301c121a1d5cd3c134c1374663b3baaad3e776` |
| bytes | 13,895,120 | 13,373,488 |
| measurement UTC | 10:18:59 | 10:19:39 |
| loadavg start | 2.01 3.66 3.42 | 1.61 3.36 3.32 |
| loadavg end | 1.66 3.40 3.34 | 1.06 3.03 3.21 |

The measured after binary was built at `740569b` and preserved after the run.
The checkout later advanced to `a182a3b` (docs); the binary hash above is the
measured one, not whatever a later build produces.

Binary size: −521,632 bytes, −3.75%.

Environment: Arch Linux, kernel 7.2.14-zen1-1-zen, Intel Core i3-2120 (4
threads), 15 GiB RAM, AMD Radeon R7 240/340 with amdgpu, rustc/cargo 1.99.1,
Hyprland 0.57.1, one output HDMI-A-1 1366x768 at scale 1, Wayland.

## Runtime comparison

All three idle samples are 10 s each, read from `/proc/<pid>`. CPU ticks are at
100 Hz, so 0 ticks means under 10 ms of CPU in the sample. Context switches are
main-thread voluntary/nonvoluntary counts, not process-wide wakeups.

| metric | before | after |
|---|---|---|
| idle CPU ticks (user+sys), samples 1/2/3 | 0 / 0 / 0 | 0 / 0 / 0 |
| idle RSS, samples 1/2/3 | 172,980 / 172,980 / 172,980 kB | 172,440 / 172,452 / 172,452 kB |
| idle PSS, samples 1/2/3 | 46,505 / 46,505 / 46,505 kB | 46,111 / 46,123 / 46,124 kB |
| idle threads | 17 | 16 |
| idle voluntary switches | 0 / 0 / 0 | 4 / 0 / 0 |
| idle nonvoluntary switches | 0 / 0 / 0 | 0 / 0 / 0 |
| first observed hayu layer | 268 ms | 210 ms |
| launcher command to layer | 87 ms | 90 ms |
| Escape to layer gone | 63 ms | 62 ms |
| launcher frame debug timing | min 0.00 / median 5.07 / max 25.26 ms, n=11 | min 0.00 / median 5.47 / max 25.06 ms, n=11 |
| desktop scan | 47 entries in 1.96 ms | 47 entries in 3.06 ms |
| reload | exit 0, 9 ms, bar survived | exit 0, 6 ms, bar survived |
| SIGTERM cleanup | exit 143, no owned layer, no process | exit 143, no owned layer, no process |

First-layer and command-to-layer values are coarse upper bounds from a 50 ms
`hyprctl layers -j` poll. Frame values are draw timings logged by
`HAYU_DEBUG_KEYS=1`, not input-to-present latency. None of these are p95 or
quiet-system minima; each is one run on a shared machine.

Churn did not measurably change at CPU tick resolution. PSS and thread count
were slightly lower after; frame median and scan time were slightly higher;
none of these single-run differences supports a performance claim.

## Live final verification, one run each

Correct `HYPRLAND_INSTANCE_SIGNATURE` came from `hyprctl instances -j`:
`acc1aa0bfd4bd575dcb0f76c35ccf12b660a1ab9_1794040165_172724534`.

| suite | exit | result |
|---|---|---|
| `tools/live/input-suite.sh` | 0 | all 15 checks passed |
| `tools/live/reload-suite.sh` | 0 | all 42 checks passed |
| `tools/live/pointer-suite.sh` | 0 | 7 checks, 0 failed |
| `tools/live/tray-suite.sh` | 0 | 11 checks, 0 failed |
| `python3 tools/live/a11y-suite.py` | 134 | failed: AT-SPI bus socket absent |

Pointer suite left workspace 1; the pre-run active workspace was also 1, and
the final active workspace is 1. Tray suite used its private bus; no fcitx5
process existed before or after.

### A11y failure — no false pass

Exact excerpt:

```
atspi-WARNING: Couldn't connect to accessibility bus: Failed to connect to socket /run/user/1000/at-spi/bus_0: No such file or directory
atspi-ERROR: AT-SPI: Couldn't connect to accessibility bus. Is at-spi-bus-launcher running?
timeout: the monitored command dumped core
```

The suite got as far as starting its isolated shell (PID 59023) and then the
Atspi client aborted because the session had no `at-spi/bus_0`. This is an
environment failure, not evidence that the AT-SPI tree is correct or broken.
It was run exactly once and was not repaired or rerun.

The crash bypassed the suite's own cleanup. Recovery performed and recorded:

- Sent SIGTERM to its own shell PID 59023; process stopped.
- Restored `org.a11y.Status IsEnabled` to `false`.
- Final state: no hayu process, AT-SPI disabled, workspace 1, one monitor, no
  fcitx5.

## `tools/perf/measure.sh` fixes

- Kept the owned shell PID alive through the post-SIGTERM layer check. Previously
  it was cleared first, so `--argjson pid` received an empty value and the
  cleanup check could report success without checking anything.
- Removed report assumptions: no hardcoded `~/.config/hayu` claim and no static
  load-average range. The report now says the user config was neither read nor
  modified, and points at `probe.txt` / `status.tsv` for load.
- Default binary is `target/release/hayu`; `HAYU_SKIP_BUILD=1` skips the warm
  build; the pre-build binary is preserved before a build when the default path
  is used; tray suite is opt-in via `HAYU_TRAY_SMOKE=1`.

Measurement lane edited only `tools/perf/measure.sh`; orchestration later committed the tool as `dfe68a0`. After measurement, a small status-exit check was added so recorded failed probes fail automation; its syntax and success/failure/skip cases were checked without repeating live measurements. The hash above identifies the script actually used during measurement, not that final mechanical follow-up.

## Limitations

- A11y is unverified because the AT-SPI bus was absent.
- One monitor only: no fractional scaling, mixed scales, hotplug, or
  multi-monitor placement test. IME is not covered.
- No p95 or input-to-present latency. Debug launcher draw times and 50 ms layer
  polling are not substitutes.
- Clock `%S` reload was not separately exercised; the reload suite uses `%H`.
  There is no dedicated clock trace to measure without source changes.
- GPU profiling tools (`glxinfo`, `eglinfo`, `intel_gpu_top`, `nvidia-smi`,
  `wgpu-info`) are missing; no clean-build time or V1 workload profile.
- Single-run, shared-machine timings; differences are not claims of
  improvement or regression.

## Exact rerun invocation

```bash
# Original binary
HAYU_BIN=/tmp/opencode/hayu-baseline/hayu-before \
HAYU_SKIP_BUILD=1 \
HAYU_BASELINE_HEAD=ab9e678204ca4224468368c5d1e044cf649b1413 \
HAYU_BASELINE_ART=/tmp/opencode/hayu-comparison/before-rerun \
/home/san/Documents/cnp/hayu/tools/perf/measure.sh

# After binary
HAYU_BIN=/tmp/opencode/hayu-baseline/hayu-after-740569b \
HAYU_SKIP_BUILD=1 \
HAYU_BASELINE_HEAD=740569bdac141f68269df6620c5b0148159b9396 \
HAYU_BASELINE_ART=/tmp/opencode/hayu-comparison/after-rerun \
/home/san/Documents/cnp/hayu/tools/perf/measure.sh
```
