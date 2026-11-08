# Performance budgets and measurement

Status: prototype measurements and benchmark plan. Historical prototype/spike results below are measurements with limited scope; candidate budgets are hypotheses, not achieved performance claims. No full Core, V1 normal or V1 stress benchmark has been completed. The November 7 refocus comparison is recorded separately below.

## November 8 consumer-owned lifecycle comparison

Same-session isolated comparison, not a V1 profile benchmark. Preserved earlier artifact SHA-256 `8092aee8312ad353a4aa18f81b301c121a1d5cd3c134c1374663b3baaad3e776` versus updated release `4ef082475d6ddd550b7be58c72c57b558a567dcf0213b1ca442a7fea343e048e`. The earlier hash identifies the November 7 artifact, not a freshly rebuilt HEAD baseline. Each profile/binary ran once, warmed for two seconds, then sampled three times at ten-second intervals. Both used the same profile config and compositor session; core had one PID-filtered bar layer, off had none. Launcher and tray were disabled. User shell PID/control-socket inode stayed unchanged.

| Profile | Earlier PSS kB | Updated PSS kB | Earlier threads | Updated threads |
| --- | --- | --- | --- | --- |
| Core bar: workspaces/title/minute clock | 43,961 | 41,550–41,559 | 18 | 14 |
| Bar/launcher/tray disabled | 8,314–8,315 | 6,049–6,051 | 5 | 1 |

These local samples show lower PSS/thread counts, not a general performance guarantee. PSS depends on shared mappings and the surrounding session. Cumulative user+system ticks stayed 17 across all core samples and 7 across all off samples for both artifacts; this establishes no CPU-speed improvement. No latency distribution, clean-build comparison, repeated-run variance, full V1 workload or screen-reader performance was measured. Raw evidence: `/tmp/opencode/hayu-lifecycle/resource-20261108-132109-a68LJS/`, `samples.tsv` SHA-256 `f4c0d564c336d3f7c7c2b3e3c9958ee88ba1a89fa9f029a0a9f564354680e510`. Temporary artifacts are not durable repository evidence; the scoped observations and binary identities are retained here.

## November 7 refocus build comparison

Initial binary of `ab9e678` was preserved before rebuild (SHA-256 `35f353fa58b4200c1b3077d381d9bca423cd7fae7c77e2d1d57d359e8bd2d542`), 13,895,120 bytes. Default release at `740569b` is 13,373,488 bytes: 521,632 bytes smaller (3.75%). This is one local unstripped artifact comparison, not installed/package size or runtime memory savings. Production normal/build dependency nodes decrease from 185 to 178 using unique package name/version pairs; optional experiments remain locked but are not default production dependencies.

Before warm no-op build took 0.212 s; final release rebuild after switching features took 120.9 s. These do **not** establish a build-speed improvement: cache/compilation work differs. No clean-build A/B exists. GPU spike builds and short functionality measurements are recorded in [REFOCUS_VALIDATION.md](REFOCUS_VALIDATION.md), not used to reopen ADR-0006. Resource comparison uses the retained `../tools/perf/measure.sh` with the same configuration and preserved original binary; short samples remain prototype evidence, not a V1 profile.

## November 7 runtime comparison

Same Intel Core i3-2120 (4 threads), 15 GiB RAM, Radeon R7 240/340 Oland/amdgpu, Arch/kernel 7.2.14-zen1-1-zen, Hyprland 0.57.1, one HDMI-A-1 1366×768 scale-1 output. Preserved before/after release binaries ran sequentially without builds, on an isolated config and private bus. Each idle run contains three 10-second samples, not three independent startups. See [REFOCUS_MEASUREMENTS.md](REFOCUS_MEASUREMENTS.md) for hashes, raw-artifact paths, exact config/method and failures.

| Metric | Before `ab9e678` | After source `740569b` |
| --- | --- | --- |
| Idle CPU ticks, 3 × 10 s at 100 Hz | 0 / 0 / 0 | 0 / 0 / 0 |
| RSS | 172,980 kB | 172,440–172,452 kB |
| PSS | 46,505 kB | 46,111–46,124 kB |
| Threads | 17 | 16 |
| First observed layer, 50 ms polling | 268 ms | 210 ms |
| Launcher map / close | 87 / 63 ms | 90 / 62 ms |
| Debug draw median / max, 11 samples | 5.07 / 25.26 ms | 5.47 / 25.06 ms |
| Reload command | 9 ms | 6 ms |

These single-run differences do not establish a speed or RAM improvement. Zero ticks means below sample resolution, not zero CPU use. Clock-minute boundaries were not guaranteed in these short windows. GPU activity/memory, wakeups, allocations, input-to-present latency, startup distributions, active-tray resource samples and full V1 profiles remain unavailable. The tray live suite validates behavior only; it is not a tray-resource benchmark. Multi-monitor/fractional/hotplug and IME were not rerun for this refocus. CPU/GL/wgpu offscreen tooling does run, but does not measure desktop GPU utilization.

## Defining lightweight

Idle surfaces do not draw continuously; the clock updates only at its required precision; modules without consumers release dedicated work; queues/caches/history are bounded; heavy IO does not block the UI. Evaluate performance alongside functionality and accessibility rather than a single RAM figure.

## Benchmark profiles

| Profile | Contents |
| --- | --- |
| Core | One bar, minute clock, workspaces/title, launcher disabled; all optional modules off (D-12). Historical prototype runs with an enabled closed launcher are not this profile |
| V1 normal | Every V1 feature category active; clipboard opt-in enabled specifically for testing; static wallpaper; one idle player and several tray items |
| V1 stress | Two mixed-scale monitors, history at cap, event/notification bursts, open launcher/picker, cold thumbnails |

Core does not replace the V1 scope. V1 normal and stress results are required before release.

## Candidate budgets

| Metric | Hypothesis | Definition |
| --- | --- | --- |
| Core idle memory | PSS ≤100 MiB | Sum of PSS for every Hayu-required process/worker/helper |
| V1 normal memory | PSS ≤180 MiB | Standard workload and warm caches; not validated |
| Idle CPU | Average <0.5% of one logical CPU | All Hayu process CPU time / wall time ×100 over 5 minutes |
| Warm launcher | p95 ≤100 ms | IPC request received to input-ready surface and first presented frame; excludes application launch |
| Warm startup | p95 ≤1 second | Process start to interactive bar and available launcher; slow services may be degraded |
| Texture cache | Candidate ≤64 MiB reusable cache | Not a total GPU memory or surface-buffer limit |
| Reload soak | No monotonic growth after warmup | Report baseline/peak/final with cache explanations; candidate final tolerance +10% after resources become quiescent |

Also record RSS for observability, but do not sum it as a replacement for PSS. Report GPU allocations and compositor-buffer impact separately; PSS does not cover all VRAM. Existing compositor/services are outside Hayu's PSS total, but additional Hayu-specific helpers are included. Report whole-session overhead changes when measurable.

## Procedure

1. Record Hayu commit, release/debug profile, compiler, dependency lock, distribution/kernel, Hyprland, GPU/driver, CPU/RAM, monitors/scale, configuration, and other processes.
2. Use the same release build and fixed power profile. Separate cold and warm starts. Measure a session baseline without Hayu for context.
3. Warm up for 60 seconds; measure idle for 5 minutes with at least three runs per profile. Record the exact clock format, media, tray, wallpaper, and history payloads.
4. Measure startup/launcher for at least 30 iterations for an initial baseline and retain all samples; increase the count if p95 is unstable. Use monotonic timestamps and correlated readiness/presentation signals; submitted frames do not prove presentation.
5. Test stress separately: 100 reloads, repeated opening/closing, hotplug, service reconnects, volume bursts, and notification payloads. Compare quiescent resource usage with the warm baseline.
6. Report p50/p95, maximum, run count, variation, and failures. Do not discard failed samples without reasons.

Candidate tools: `/proc/<pid>/smaps_rollup` for PSS, `pidstat`/perf for CPU, wakeups, and threads, plus vendor-specific GPU tools. Record tools used; do not present a single system monitor memory figure without defining it.

For power, run A/B measurements on capable hardware with identical power profile/backlight, multiple runs, and variability ranges. No watt target is approved until a reference machine exists.

## Gates and comparisons

M0 establishes candidate-renderer baselines; M1 sets realistic budgets; M6 checks all profiles. Material CPU/memory/latency regressions require explanation and follow-up; candidate review threshold: 10% on identical workloads, not automatic failure from noise.

Compare Noctalia only with equivalent versions, hardware, outputs, workloads, and enabled features. Separate cold/warm caches and supporting processes; include configurations and benchmark scripts. Rust alone must not justify claims that Hayu is faster or lighter.

## Results template

| Commit/profile/hardware | Samples | PSS | Idle CPU | GPU | Startup p95 | Launcher p95 | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 02a7687 / release / AMD Radeon R7 240, Hyprland 0.57.0 | 1 run, 30 s idle | 5.6 MiB | 0.07% of one core | not measured | not measured | not measured | M0 bar only (1366×32, one output, CPU shm path); not a Core, V1 normal, or V1 stress profile |
| df68feb / release / Radeon R7 240, Hyprland 0.57.0 | 3 offscreen runs × 300 frames | ≈50 MiB GL / ≈32.6 MiB wgpu (spike bar) | <0.03% while holding | not attributable | start 172 ms GL / 141 ms wgpu | not measured | Renderer spike, 1366×32 scene; offscreen p50 0.095 ms GL, 0.125 ms wgpu, 0.538 ms CPU; not a Core/V1 profile |
| 10fbb4c / release / Radeon R7 240, Hyprland 0.57.0 | 16 launcher frames, one 20 s idle sample | 6.6 MiB PSS with the launcher open, 6.1–6.3 MiB closed (+~0.5 MiB) | 0 ticks in 20 s while open | not measured | 3–8 ms CLI round trip; first frame 12.6 ms cold | 7.25 ms frame p95 (p50 2.08 ms) | Launcher at scale 1, 46 desktop entries: scan 2.0–3.9 ms per open; sampling method and limits are in [TESTING.md](TESTING.md); not a Core/V1 profile |
| 9e0a4d3 / release / Radeon R7 240, Hyprland 0.57.0 | 62 launcher frames at scale 2; PSS sampled once | 8.4 MiB PSS open vs 6.4 MiB closed at scale 2 (+2.1 MiB for the 1120×840 buffer) | not sampled at scale 2 (0 ticks in 20 s at scale 1) | not measured | not measured | 9.99 ms frame p95 (p50 5.05 ms) at scale 2 | One 1366×768 output at 2× via `hl.monitor`, 46 desktop entries; cold frame 32.8 ms; not a Core/V1 profile |
| 19b6f8b / release / Radeon R7 240, Hyprland 0.57.0 | 1 run, 80 s idle across two minute boundaries | 7.0 MiB | 0 ticks in 80 s (<0.02% of one core), 6 voluntary context switches | not measured | not measured | not measured | Idle 1366×32 bar, `%H:%M` clock, one output: two commits total outside the startup burst, both the minute tick; not a Core/V1 profile |
| Not measured | — | — | — | — | — | — | No V1 profile has been measured |
