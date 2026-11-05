# Performance budgets and measurement

Status: benchmark plan 0.1. **Hayu has no measurements yet. Every figure below is an initial hypothesis, not a performance claim.**

## Defining lightweight

Idle surfaces do not draw continuously; the clock updates only at its required precision; modules without consumers release dedicated work; queues/caches/history are bounded; heavy IO does not block the UI. Evaluate performance alongside functionality and accessibility rather than a single RAM figure.

## Benchmark profiles

| Profile | Contents |
| --- | --- |
| Core | One bar, minute clock, workspaces/title, available but closed launcher; all other modules off |
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
| Not measured | — | — | — | — | — | — | No V1 profile has been measured |
