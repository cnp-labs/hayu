# ADR-0006 — Renderer selection criteria and M0 GPU spike

Date: November 5, 2026. Status: **proposed — criteria fixed before measurement**.
Supersedes the experiment plan in [ADR-0004](0004-renderer-spike.md); ADR-0004
stays as the record of why the question was opened.

## Context

ADR-0004 left the renderer open between GLES/EGL and wgpu, with a CPU path used
only to unblock prototyping. The M0 prototype now draws a 1366×32 layer-shell
bar with tiny-skia over `wl_shm`; text comes from cosmic-text, and nothing has
measured the GPU backends. O-01 in DECISIONS.md cannot stay open indefinitely,
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
per-criterion evidence, DECISIONS.md O-01 is updated, and the losing backend is
removed unless it is kept as a documented fallback. The CPU path is retained for
tests regardless of the outcome.
