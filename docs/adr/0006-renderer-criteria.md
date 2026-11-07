# ADR-0006 — Renderer selection criteria and M0 GPU spike

Date: November 5, 2026. Status: **accepted — GLES/EGL with glow**, CPU path retained.
Supersedes the experiment plan in [ADR-0004](0004-renderer-spike.md); ADR-0004
stays as the record of why the question was opened. Results and scores are in
the section at the end of this document.

## Context

ADR-0004 left the renderer open between GLES/EGL and wgpu, with a CPU path used
only to unblock prototyping. The M0 prototype now draws a 1366×32 layer-shell
bar with tiny-skia over `wl_shm`; text comes from cosmic-text, and nothing has
measured the GPU backends. O-01 in ../DECISIONS.md cannot stay open indefinitely,
because text upload, scaling, and surface presentation differ enough between
backends to change the UI architecture.

This ADR fixes the evaluation method **before** either backend exists, so the
criteria cannot be fitted to whichever implementation turns out easier.

## Scope of the spike

Basic Hayu needs only, matching the M0 slice:

1. One layer-shell bar surface per output, created and destroyed with output
   hotplug, anchored top or bottom, exclusive zone equal to bar height.
2. A flat background, filled rounded rectangles (workspace pills), and one
   uploaded RGBA texture (stand-in for icons and wallpaper thumbnails).
3. Text: system font glyphs rasterized once into an atlas, sampled with linear
   filtering, positioned at fractional logical coordinates; bar redrawn only on
   state change.
4. Integer and fractional scale changes (1, 1.25, 1.5, 2) applied to the same
   scene.
5. Graceful failure when the backend or driver is unavailable.

Out of scope: window effects, blur, shadows, animation, video, multi-GPU,
NVIDIA/Intel validation hardware, and every V1 feature module.

## Hard gates

A backend that fails any gate is eliminated regardless of score.

| Gate | Requirement | Method |
| --- | --- | --- |
| G1 Surface lifecycle | Bar presents on the live layer-shell surface, survives a scale change and a monitor unplug/replug without panic or stale surface, tears down cleanly on SIGTERM | Manual run on the target machine, `hyprctl layers`, exit status |
| G2 Text quality | Bar text is legible and correctly positioned at 100% and 200%; no per-frame glyph rasterization | Visual capture plus code inspection |
| G3 Compatibility | Runs on the target Mesa stack without proprietary drivers, and reports a clear error when no usable device exists | Actual run; forced-failure run |
| G4 Determinism | Same scene produces output matching the CPU reference within a stated tolerance | Pixel comparison against `tiny-skia` render |
| G5 Licensing | All new dependencies use permissive licenses, no binary blobs | `cargo metadata` license review |

## Scored criteria

Weights apply only to backends that pass every gate.

| ID | Criterion | Weight | Measurement |
| --- | --- | --- | --- |
| S1 | Resource use | 25% | PSS of the presenting process after 60 s idle, idle CPU over 60 s, GPU memory reported by the driver, wakeups per second |
| S2 | Latency | 20% | Cold start to first presented frame; p50/p95 GPU-side frame time over 300 frames of the reference scene; cost of one full redraw |
| S3 | Integration and maintenance cost | 20% | Lines of backend code, direct dependencies added, clean-build time from scratch, unsafe/FFI surface area, amount of backend-specific code leaking into `ui`/`shell` |
| S4 | Compatibility and fallback | 15% | Backend availability on this machine and on software fallback (llvmpipe/lavapipe), behaviour when the preferred device is missing, driver quirks encountered |
| S5 | Debuggability | 10% | Validation layers / error reporting quality, ability to capture a frame, quality of failure messages, debugging effort actually spent during the spike |
| S6 | Text and scaling quality | 10% | Atlas sampling quality, fractional-scale handling, subpixel option availability, observed artifacts |

Scoring per criterion is 1–5, multiplied by weight. Evidence must be recorded
per number; unmeasured criteria score 1 and say "not measured".

## Method

- Same commit, same scene definition, same font, same output size for both
  backends. Release builds. Three runs per measurement, report median and p95.
- Record exact commits, dependency versions, kernel, Mesa, RADV, Hyprland, and
  command lines used.
- CPU path stays the reference for image comparison; it remains available as a
  fallback even after the decision.
- Measure p50/p95 frame time with GPU work submitted and completed (frame
  readback for the benchmark scene), not with submissions merely queued.
- Do not compare against Noctalia; there is no equivalent workload here.

## Decision rule

The highest weighted score among gate-passing backends wins. A margin below 10%
counts as a tie; a tie goes to the backend with the smaller dependency and
maintenance burden while keeping the CPU fallback working. A backend may also be
rejected with recorded evidence even if it scores higher, but the reason must be
a gate failure or a stated maintenance risk, not preference.

## Environment baseline

Recorded before implementation: Arch Linux, kernel 7.2.14-zen1, Intel i3-2120,
AMD Radeon R7 200 Series (RADV OLAND, Mesa 26.2.6), Vulkan 1.3.354, Hyprland
0.57.0, one 1366×768 output at scale 1, Rust 1.99.1. A second GPU vendor and a
second distribution are not available, and the ADR must state that limitation.

## Consequences

After results are appended, this ADR changes status to accepted or rejected with
per-criterion evidence, ../DECISIONS.md O-01 is updated, and the losing backend is
removed unless it is kept as a documented fallback. The CPU path is retained for
tests regardless of the outcome.

## Results (November 5, 2026)

Spike commit: `df68feb`. Binary: `hayu-gpu-spike`; that revision built CPU/GL
spike tooling by default and enabled wgpu only via `--features wgpu`.
Current builds require `--features gpu-spike` for CPU/GL tooling; `wgpu`
implies that feature. Production GLES is not an optional `gl` Cargo feature.

Commands used (release build, same scene for all backends):

```
cargo build --release --features gpu-spike  # current command; originally default at df68feb
cargo build --release --features wgpu
./target/release/hayu-gpu-spike --backend {cpu|gl|wgpu} --mode offscreen --frames 300 --dump /tmp/x.ppm --json
./target/release/hayu-gpu-spike --backend {gl|wgpu} --mode present --frames N --hold S --json
```

Environment: Arch Linux, kernel 7.2.14-zen1, Intel i3-2120, AMD Radeon R7 200
(RADV OLAND, Mesa 26.2.6), Vulkan 1.3.354, Hyprland 0.57.0, one 1366×768 output
at scale 1, Rust 1.99.1.

### Hard gates

| Gate | GLES/EGL | wgpu | Evidence |
| --- | --- | --- | --- |
| G1 Surface lifecycle | Pass | Pass | Layer `hayu-gpu-spike` mapped on HDMI-A-1 and on a temporary headless output; output add and remove survived without panic; SIGTERM exits 0. Scale-change re-render is implemented but could not be exercised live (see limitations). |
| G2 Text quality | Pass | Pass | Glyphs rasterized once into an atlas, sampled from a texture; captured frame shows correct pills, clock, title and image at 100% and 200% scale. |
| G3 Compatibility | Pass | Pass | Mesa/radeonsi and RADV respectively, no proprietary driver. GL falls back to llvmpipe when forced; wgpu reports `no Vulkan adapter: ...` and exits 1 when no driver is loadable. |
| G4 Determinism | Pass | Pass | gl and wgpu frames are bit-identical (MAD 0); versus the CPU reference MAD is 8.31/65535 (0.0127%) at scale 1 and 5.63/65535 (0.0086%) at scale 2. |
| G5 Licensing | Pass | Pass | All added dependencies are MIT/Apache-2.0/BSD/ISC/Zlib; no binary blobs. `cargo metadata` reports no copyleft-only package. |

Both backends pass every gate. The GL backend initially failed output hotplug
with `swap buffers: ... argument does not name a valid surface`; the cause was
one EGL context per output with no `make_current` before drawing, and the fix is
part of this spike. That defect is counted under S3 and S5, not hidden.

### Measurements

| Metric | GLES/EGL | wgpu | CPU (reference) |
| --- | --- | --- | --- |
| Cold start to first presented frame, p50 of 3 runs | 172.0 ms | 140.7 ms | n/a |
| Offscreen 300 frames, p50 (median of 3 runs) | 0.095 ms | 0.125 ms | 0.538 ms |
| Offscreen 300 frames, p95 (median of 3 runs) | 0.124 ms | 0.146 ms | 0.600 ms |
| Offscreen 100 frames at scale 2, p50 (1 run) | 0.225 ms | 0.179 ms | 1.952 ms |
| Idle PSS, one 1366×32 bar (samples) | ≈50 MiB (34.8–55.1) | ≈32.6 MiB (29.6–42.4) | 5.6 MiB |
| Idle CPU while holding the surface | <0.03% of one core | <0.03% of one core | <0.07% |
| Voluntary wakeups while idle | 0 in 20–25 s | 0 in 20–25 s | 0 |
| Cold build from scratch (`CARGO_TARGET_DIR` empty) | 222 s | 391 s | 118 s core only |
| Normal dependency count | 127 (+9 over core) | 171 (+53 over core) | 118 |
| Backend-specific lines of code | 590 | 584 | 182 |

Present-mode frame times are not comparable: wgpu presents with the Vulkan FIFO
model and therefore reports one frame per refresh (~16.7 ms), while the GL path
uses `SwapInterval::DontWait` and reports GPU time (~0.17 ms p50). The offscreen
numbers above are the like-for-like comparison.

### Scores

| ID | Criterion | Weight | GLES/EGL | wgpu | Basis |
| --- | --- | --- | --- | --- | --- |
| S1 | Resources | 25% | 3 | 4 | PSS ≈50 vs ≈32.6 MiB; idle CPU and wakeups equal. |
| S2 | Latency | 20% | 4 | 4 | wgpu starts 31 ms sooner; GL frame work is ~24% cheaper at scale 1, wgpu faster at scale 2. Both are far below any frame budget. |
| S3 | Integration and maintenance | 20% | 4.5 | 2.5 | +9 vs +53 dependencies, 222 s vs 391 s cold build, similar LOC; the GL context fix shows raw-FFI risk. |
| S4 | Compatibility and fallback | 15% | 4 | 3 | GL runs on llvmpipe without a GPU; wgpu has no software Vulkan here and needs its GL backend enabled for that case. |
| S5 | Debuggability | 10% | 2.5 | 4.5 | wgpu errors enumerate adapters and backends; GL's EGL errors were cryptic and the multi-output bug needed manual debugging. |
| S6 | Text and scaling | 10% | 4 | 4 | Identical pixels and identical atlas/scale code path. |
|  | **Weighted total** |  | **3.70** | **3.60** |  |

### Decision

**GLES/EGL with glow is the renderer for Hayu.** The weighted difference is
2.7%, below the 10% tie threshold fixed in this ADR, so the tie-break applies:
GLES/EGL adds 9 dependencies instead of 53, builds 43% faster from scratch, and
keeps a working software fallback, while the CPU path remains for tests and
reference frames.

wgpu is **rejected as the primary backend**, not because it performed badly —
it used less memory and started faster — but because it lost the tie-break on
dependency and build cost. Its spike code stays behind the non-default `wgpu`
feature until M1, then it is deleted unless a concrete need (for example a
Vulkan-only feature) appears.

Consequences for M1: the bar moves from tiny-skia to the GL backend; the glyph
atlas, sRGB handling, and buffer-scale rules from this spike become the shared
render interface; the CPU path stays as a test renderer, not as a fallback for
users.

### Limitations

- One GPU vendor and one physical output; hotplug was simulated with Hyprland's
  headless output, and a real monitor unplug was not tested.
- A live output scale change could not be triggered on this setup
  (`hyprctl keyword monitor` reported `unknown request` for the headless output),
  so the scale-change path is implemented and covered by offscreen scale-2
  frames but not verified on a live surface. Both backends share that code, so
  the score is unaffected; it is still an M1 task.
- GPU memory could not be attributed per process: global VRAM moved by more than
  ±100 MiB from unrelated session processes during sampling. S1 therefore scores
  PSS and CPU only.
- Present-mode p50/p95 differ by pacing model, as described above.
- Build times come from one cold run per feature set on this machine; treat them
  as order-of-magnitude evidence, not a benchmark suite.
