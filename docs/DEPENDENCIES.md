# Hayu dependency and resource audit

Baseline inventory: `ab9e678204ca4224468368c5d1e044cf649b1413`, audited November 7, 2026. The table records baseline usage/features; changes applied afterward are recorded below. Costs not supported by measurements remain unmeasured.

## Applied changes

`23f7016` also disables zbus defaults while retaining `async-io`: `blocking-api` is genuinely absent, not re-enabled by AccessKit. It replaces the direct `wayland-backend` feature-selector dependency with `wayland-client`'s `system` feature; `client_system` remains active for raw EGL pointers. Default, gpu-spike and wgpu all-target compile checks pass. No versions changed and no additional package disappeared from the lockfile. The exact intermediate feature table in [DEPENDENCY_FEATURES.md](DEPENDENCY_FEATURES.md) therefore changes as follows: zbus loses `default` and `blocking-api`, zbus_macros loses `blocking-api`, wayland-client gains `system`, and wayland-backend becomes transitive-only with `client_system` unchanged.

`281fb65` trims glutin to EGL/Wayland, isolates `src/gpu/` and the spike binary behind `gpu-spike`, and makes tiny-skia test/spike-only without `png-format`. `wgpu` implies `gpu-spike`; pollster stays optional. Production still uses GLES/EGL. Build tooling with `cargo build --release --features gpu-spike` or `--features wgpu`. No dependency was replaced by internal protocol/text/parser/accessibility code. `libc` stays transitively present where required; making its direct spike edge optional does not remove libc from the runtime graph.

Using `cargo tree --offline --prefix none -e normal,build`, unique name/version nodes fall from 185 to 178; with dev edges included, 185 to 183. Lockfile package entries fall from 274 to 271, with no package version updates. These are graph counts, not CPU/RAM savings. Unreachable `glutin_glx_sys`, `glutin_wgl_sys` and `x11-dl` lock entries are removed. Tiny-skia and its helper graph remain for tests/spike; optional wgpu lock entries remain intentionally.

## Validation

Commands run in repository root:

- `cargo metadata --format-version 1 --offline --no-deps`
- `cargo tree --offline > /tmp/opencode/hayu-tree.txt`
- `cargo tree -e features --offline > /tmp/opencode/hayu-tree-features.txt`
- `cargo tree --duplicates --offline > /tmp/opencode/hayu-tree-duplicates.txt`
- `cargo tree -e features --offline --depth 1 > /tmp/opencode/hayu-depth1-features.txt`
- `cargo tree -e features -i <crate> --offline` for selected dependencies; consolidated output: `/tmp/opencode/hayu-direct-feature-inbound.txt`
- `cargo check --offline` — exit 0
- `cargo test --offline` — exit 0; 56 unit tests passed, 0 failed
- `cargo check --offline --features wgpu` — exit 0
- `cargo check --offline --no-default-features` — exit 0

Lockfile has 274 package entries, 228 resolved metadata packages. `cargo tree -d` reported repeated versions: font-types 0.11.3/0.12.7; read-fonts 0.37.0/0.41.0; skrifa 0.40.0/0.44.0; miniz_oxide 0.8.9/0.9.1; syn 2.0.119/3.0.9; hashbrown 0.17.1 appears through separate graph paths; plus single-version repetitions from graph paths. Do not assume all `cargo tree -d` output means duplicate compiled copies: same version may print due to multiple inbound paths. Actual cross-version splits are expected transitive version requirements; no safe direct pin/update inferred.

Exact resolved active features and key transitive features are listed in [DEPENDENCY_FEATURES.md](DEPENDENCY_FEATURES.md). The inventory below preserves the baseline so changes can be reviewed rather than retroactively rewriting evidence.

## Dependency inventory

“Default” means Cargo default feature set in normal package build. `wgpu` is opt-in feature; `pollster` and `wgpu` optional. Build/test classification refers to use site; Cargo currently makes all ordinary direct deps available to all package targets.

| Direct dependency | Source purpose/use path | Default features and active features | Classification and disposition |
|---|---|---|---|
| `accesskit` | `src/a11y.rs`; launcher accessibility tree/events | default | Runtime, accessibility. Keep. |
| `accesskit_unix` | `src/a11y.rs`; Unix accessibility adapter | default -> `async-io` | Runtime, accessibility. Keep; substantial AT-SPI/zbus graph is required by the feature. Verify whether upstream offers a sync/minimal feature before trimming; do not remove accessibility. |
| `async-channel` | `src/tray.rs`; worker command channel | default | Runtime, tray. Keep. |
| `calloop` | `src/app.rs` (formerly `src/bar.rs`), `src/hyprland.rs`, `src/ipc.rs`, `src/tray.rs`; event loop, sources, channels, signal handling | default plus requested `signals` | Runtime. Keep; `signals` explicitly used. |
| `chrono` | `src/app.rs` (formerly `src/bar.rs`), `src/render.rs`; local clock and formatted time | defaults disabled, `clock` enabled (pulls clock/now/std/iana-time-zone) | Runtime. Keep; `clock` required for local time. |
| `cosmic-text` | bar/launcher text layout, glyph rasterization; GPU spike atlas too | default -> std, swash, fontconfig, sys-locale | Runtime plus spike. Keep. Feature reduction is risky: fontconfig and swash back font discovery/rasterization. Test `default-features = false` only with equivalent platform/font coverage. |
| `futures-lite` | `src/tray.rs`; async stream and future combinators | default | Runtime, tray. Keep. |
| `glow` | `src/render/gl.rs`, `src/gpu/gl.rs`; GLES calls | default (no meaningful optional feature activated) | Runtime renderer and spike. Keep. Spike source shares dependency with production renderer. |
| `glutin` | `src/render/gl.rs`, `src/gpu/gl.rs`; EGL context/surface APIs | default: `egl`, `glx`, `x11`, `wayland`, `wgl` | Runtime + spike. Strong feature-trim candidate: explicit `default-features = false, features = ["egl", "wayland"]` matches EGL/Wayland source APIs; drops GLX/X11/WGL platform support and transitive system/window crates. Validate build, doctor, GL spike, live bar on target Arch/Hyprland, and docs/platform scope before adoption. |
| `libc` | `src/gpu/present.rs`; SIGINT/SIGTERM for spike | default | Spike only (present module only). Isolate GPU spike; then optionalize dependency with spike feature or replace signal setup only if needed. Do not remove before module split. |
| `png` | `src/tray.rs`; theme PNG icon decoding | default | Runtime, tray. Keep. Also pulled by tiny-skia `png-format`; setting tiny-skia no-default-features removes its edge, not this direct runtime edge. |
| `pollster` | `src/gpu/wgpu.rs`; synchronous adapter/device setup | optional, default features | Spike-only; optional behind `wgpu` already. Keep/isolated with spike. |
| `raw-window-handle` | production GL and spike GL/WGPU Wayland EGL handle construction | default | Runtime + spike. Keep; shared API surface makes it difficult to isolate without module/feature boundary. |
| `serde` | `src/config.rs`, `src/hyprland.rs`, `src/ipc.rs`; derives | default plus `derive` | Runtime. Keep; optional `rc` etc not requested. |
| `serde_json` | IPC messages, Hyprland JSON, spike `--json` | default | Runtime + spike. Keep; spike use alone insufficient to split package dependency since runtime needs it. |
| `smithay-client-toolkit` | `src/app.rs` (formerly `src/bar.rs`), `src/launcher.rs`; Wayland layer shell, seats, registry; `src/gpu/present.rs` spike | default (`calloop`, `xkbcommon`) plus explicit `calloop` | Runtime + spike. Keep. Defaults appear required by source keyboard/Wayland support. `calloop` requested redundantly since default already enables it; remove explicit feature only after checking locked default policy. |
| `tiny-skia` | `src/render.rs` reference rasterizer used by tests; `src/launcher.rs` test PPM helper; `src/gpu/cpu.rs` experimental CPU backend | default -> `std`, `simd`, `png-format` | Tests + GPU spike; no production runtime path calls `render::rasterize` (production GL draws `Frame` directly). Candidate to isolate: move GPU spike out of default library build, gate reference rasterizer/test helper with `cfg(test)` or a test/spike feature, and enable tiny-skia only for those targets. Validate `cargo test`, `cargo check`, `cargo check --features ...`. Keep CPU reference/test capability unless explicitly abandoned. Drop tiny-skia's `png-format` feature: Hayu does not use its PNG encoder/decoder API; `tiny-skia` itself also depends optionally on png only for this feature. Verify feature-off tests. |
| `toml` | `src/config.rs`; config parse/validation | default | Runtime. Keep. |
| `wayland-backend` | `src/app.rs` (formerly `src/bar.rs`)/Wayland internals through direct import? Direct grep found no `wayland_backend::` source references; backend API enabled via feature `client_system` | default + `client_system` | Runtime/system integration. `client_system` likely intentional linking mode; investigate direct use first. Do not remove solely on grep: may be chosen to select SCTK/Wayland backend feature-unification behavior. Cargo tree feature inbound inspection needed before trim. |
| `wayland-client` | bar, launcher, spike presenter and GL Wayland display proxy | default | Runtime + spike. Keep. |
| `wayland-protocols` | bar and launcher fractional scale, text-input, viewporter | defaults disabled, `client`, `staging` | Runtime. Keep both features: imports include staging protocols and client objects. |
| `xkbcommon` | launcher keysym constants in code/tests; actual keymap handling via SCTK | default | Runtime/test. Keep unless SCTK exposes same exact keysyms without public direct use; avoid duplicating keyboard/keymap logic. |
| `wgpu` | `src/gpu/wgpu.rs`; optional backend for GPU spike | optional, `default-features = false`, features `std`, `vulkan`, `wgsl` | Spike only; optional. Keep isolated/optional. Explicitly excluding DX12/Metal/GLES/WebGPU/parking_lot is a good safe trim. |
| `zbus` | `src/tray.rs`; StatusNotifierWatcher and tray calls | default -> `async-io`, `blocking-api` | Runtime, tray. Keep async executor support. `zbus::block_on` is used, but this does not by itself prove the separate generated blocking API feature is required; its trimming remains unverified. |

## Primary recommended changes, bounded order

1. **Trim `glutin` default platform features.** Current code explicitly constructs EGL and Wayland handles, and imports only EGL/Wayland paths in both render implementations. Test `default-features = false, features = ["egl", "wayland"]` on intended Linux build. This is lowest-risk reduction in default build graph, but validate user scope before dropping X11/WGL compilation.
2. **Stop compiling GPU spike modules into production library.** `src/gpu/` is referenced only by `src/bin/gpu_spike.rs`, except shared crate module declarations; there are no `crate::gpu` production callers. Put spike behind a package feature and mark `hayu-gpu-spike` `required-features`, or split into a separate package. First option minimal diff; keep the `wgpu` feature and its compile check. Be careful `hayu-gpu-spike` is currently an existing binary shipped/built by normal Cargo commands; feature-gating changes that behavior and should be explicit. This can also isolate `libc` and non-renderer spike SCTK/Wayland references.
3. **Isolate `tiny-skia` and its CPU reference backend.** Current production bar/launcher rendering is GLES (`render::gl::GlPresenter`); `render::rasterize` is called only in tests, while `gpu::cpu` is spike-only and `launcher::write_ppm` is test-only. Once spike modules are gated, gate CPU/reference APIs/test helpers as appropriate. Use `tiny-skia = { version = "0.12", default-features = false, features = ["std", "simd"] }` if tests still need it: removes its unnecessary `png-format` edge while retaining CPU pixmap APIs. Do not call CPU path “production fallback”; it is not selected by bar runtime.

These three moves are separable but steps 2 and 3 share boundary work. First proposed change: glutin feature trim only, with `cargo check --offline`, `cargo test --offline`, and live GL bar/spike validation. No measured binary-size, compile-time, memory, or runtime savings claimed.

## Feature and boundary notes

- Default features for `glutin` bring irrelevant X11/GLX/WGL code into an EGL Wayland program. This is concrete dependency graph overhead, not a measured cost.
- `tiny-skia` default feature `png-format` activates a second route to `png` and `flate2`; direct `png` remains required for tray icons. Disable only tiny-skia `png-format` if retained.
- `wgpu` is correctly opt-in and excludes non-Vulkan backends in `Cargo.toml`; do not expand its defaults.
- GPU spike imports `hayu::gpu` through the same library crate as the shell. The `wgpu` feature only guards WGPU implementation, not CPU/GL spike or the rest of `src/gpu`; default compilation still includes CPU+GL spike modules.
- `Cargo.lock` currently includes optional WGPU dependencies in lock graph; presence in lockfile is not proof that default executable links/compiles them.
- `cargo tree -d` is not a cost metric. Version forks in font stack, zbus/toml macro graph, and compression chain should not be overridden without upstream compatibility evidence.

## Obvious resource findings

- `src/tray.rs::run` publishes snapshots by cloning `Item` title/service/path and cloning `Icon` RGBA bytes into each `TrayItem` (`src/tray.rs`, `Event::Items`). `bar.rs` copies those icons again into render input; `render` builds a new tray strip vector when drawing. `Frame::image.version` gates GPU upload, but this does **not** eliminate these CPU copies on each changed item snapshot/frame. Tray icon sizes are not capped for PNG theme files; pixmap selection restricts height `<=64` but not width, and validates `data.len() == (*w * *h * 4) as usize` using signed multiplication, risking overflow on malicious/bad remote dimensions. The D-Bus item is local user-session input, still untrusted. Add bounded dimensions/checked multiplication, max decoded PNG dimensions/bytes, and avoid cloning icon RGBA across snapshot/render stages if profiling shows this path matters.
- `tray::theme_icon` recursively scans each icon root to depth 3 for every lookup and does not deduplicate roots; runs on registration, not each frame. No polling. It is a latency candidate only if many registrations or slow icon roots; no data yet.
- `gpu::cpu::render` reserves `frames` timing values and rasterizes a full pixmap per frame; retains only last pixel buffer. This is spike-only and expected benchmarking work, not production allocation behavior.
- `hayu-gpu-spike` stores one timing per requested frame; `--frames` parsing allows user-sized counts. Audit of CLI caps and pixel allocation safety advisable before calling spike input hardened.
- Existing render frame/atlas reuse and tray image upload version gating are sound intended optimizations, but only gate GPU texture upload; they do not imply no host-side allocation.

## Evidence paths

- `Cargo.toml`: direct dependency declarations and opt-in `wgpu` feature.
- `src/lib.rs`: all modules, including GPU, currently always exported.
- `src/bin/gpu_spike.rs`: only external caller of `hayu::gpu`.
- `src/render.rs`: `rasterize` implementation; test calls only.
- `src/launcher.rs`: `rasterize` and PPM helper calls reside in tests.
- `src/render/gl.rs`, `src/app.rs` (formerly `src/bar.rs`), `src/launcher.rs`: production GLES draw path.
- `src/gpu/cpu.rs`, `src/gpu/gl.rs`, `src/gpu/wgpu.rs`, `src/gpu/present.rs`: experiment implementations.
- `src/tray.rs`: icon decoding, pixel validation, cloned snapshots and icon lookup.
- `/tmp/opencode/hayu-tree.txt`, `/tmp/opencode/hayu-tree-features.txt`, `/tmp/opencode/hayu-tree-duplicates.txt`, `/tmp/opencode/hayu-depth1-features.txt`, `/tmp/opencode/hayu-direct-feature-inbound.txt`: raw Cargo graph outputs.
