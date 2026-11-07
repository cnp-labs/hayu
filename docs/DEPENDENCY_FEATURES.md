# Hayu dependency feature appendix

Snapshot: dependency isolation changes now recorded in `281fb65`, November 7, 2026. Subsequent zbus/Wayland feature-selector cleanup is described in [DEPENDENCIES.md](DEPENDENCIES.md); this table preserves the audited intermediate graph. This appendix supersedes feature statements in the first audit where they differ.

## Commands and status

- `cargo metadata --format-version 1 --offline` — exit 0. Resolved: 225 packages/nodes. `Cargo.lock` has 271 `[[package]]` entries.
- `cargo tree --offline` — exit 0; 449 lines.
- `cargo tree -e features --offline` — exit 0; 1187 lines.
- `cargo tree -e normal --depth 1 --offline` — exit 0. Default direct normal edges have no `libc`, `pollster`, `tiny-skia`, or `wgpu`; libc remains transitively required.
- `cargo tree -e dev --depth 1 --offline` — exit 0. Only direct dev edge: `tiny-skia v0.12.0`.
- `cargo tree -e features -i zbus --offline` — exit 0. `blocking-api` comes only from `zbus feature "default"` via Hayu.
- Feature lists below come from the default `cargo metadata` resolve graph, not `--all-features`. They include dev-target feature unification and target-conditional features (for example Windows-only features); they are not Linux compile artifacts.
- No `cargo check`, `cargo test`, live run, or proposed-manifest compile was performed for this appendix. Proposed trims remain unvalidated by build.

Raw current outputs:

- `/tmp/opencode/hayu-tree-current.txt`
- `/tmp/opencode/hayu-tree-features-current.txt`
- `/tmp/opencode/hayu-metadata-full-current.json`

## Direct dependencies: exact resolved active features

| Direct dependency | Version | Declaration state | Exact active features in default resolve | Role | Classification |
|---|---|---|---|---|---|
| `accesskit` | 0.25.2 | normal | none declared/active | `src/a11y.rs` accessibility tree | runtime |
| `accesskit_unix` | 0.24.1 | normal | `async-io`, `default` | `src/a11y.rs` AT-SPI adapter | runtime |
| `async-channel` | 2.5.0 | normal | `default`, `std` | `src/tray.rs` worker commands | runtime |
| `calloop` | 0.14.5 | normal, requested `signals` | `nix`, `signals` | event loop, Wayland source, signals, channels | runtime |
| `chrono` | 0.4.45 | normal, no defaults, requested `clock` | `alloc`, `clock`, `iana-time-zone`, `now`, `std`, `winapi`, `windows-link` | clock rendering | runtime; two features target-conditional |
| `cosmic-text` | 0.19.0 | normal, defaults | `default`, `fontconfig`, `std`, `swash`, `sys-locale` | text shaping, font discovery, glyph rasterization | runtime |
| `futures-lite` | 2.6.1 | normal | `alloc`, `default`, `fastrand`, `futures-io`, `parking`, `race`, `std` | tray async stream/future | runtime |
| `glow` | 0.18.0 | normal | none declared/active | production GLES and spike GL | runtime + experiment |
| `glutin` | 0.32.3 | normal, no defaults, requested `egl`, `wayland` | `egl`, `glutin_egl_sys`, `libloading`, `wayland`, `wayland-sys` | production EGL and spike EGL | runtime + experiment |
| `libc` | 0.2.191 | optional, `gpu-spike` enables | absent in default resolve | spike signal handling | experiment |
| `png` | 0.18.1 | normal | none declared/active | tray theme PNG decode | runtime |
| `pollster` | 1.0.1 | optional, `wgpu` enables | absent in default resolve | spike WGPU setup | experiment |
| `raw-window-handle` | 0.6.2 | normal | none declared/active | EGL/Wayland raw handles | runtime + experiment |
| `serde` | 1.0.229 | normal, requested `derive` | `default`, `derive`, `serde_derive`, `std` | config/IPC/Hyprland derives | runtime |
| `serde_json` | 1.0.154 | normal | `default`, `std` | IPC, Hyprland, spike JSON | runtime + experiment |
| `smithay-client-toolkit` | 0.21.1 | normal, requested `calloop` | `bytemuck`, `calloop`, `calloop-wayland-source`, `default`, `pkg-config`, `xkbcommon` | Wayland shell, seats, keyboard, layer surfaces | runtime + experiment presenter |
| `tiny-skia` | 0.12.0 | optional normal (`gpu-spike`) plus dev-dependency | `simd`, `std` via dev edge in default resolve; absent from default normal build | test reference rasterizer; spike CPU backend | test + experiment |
| `toml` | 1.2.0+spec-1.1.0 | normal | `default`, `display`, `parse`, `serde`, `std` | config parse/validation | runtime |
| `wayland-backend` | 0.3.17 | normal, requested `client_system` | `client_system` | selects libwayland system backend; no direct source import | runtime feature selector |
| `wayland-client` | 0.31.15 | normal | none declared/active | Wayland connection/proxies | runtime + experiment |
| `wayland-protocols` | 0.32.13 | normal, requested `client`, `staging` | `client`, `staging`, `unstable`, `wayland-client` | fractional scale, text-input, viewporter | runtime; `unstable` activated transitively by SCTK |
| `wgpu` | 30.0.1 | optional, `wgpu` feature enables; no defaults, requested `std`, `vulkan`, `wgsl` | absent in default resolve | spike WGPU backend | experiment |
| `xkbcommon` | 0.8.0 | normal | `default`, `memmap2`, `wayland` | keysyms in launcher/a11y/tests | runtime + test |
| `zbus` | 5.19.0 | normal, defaults | `async-executor`, `async-fs`, `async-io`, `async-lock`, `async-process`, `async-task`, `blocking`, `blocking-api`, `default` | tray D-Bus worker | runtime |

There are no direct build-dependency crates. Proc-macro crates appear transitively.

Package features:
- `[features] default = []`
- `gpu-spike = ["dep:libc", "dep:tiny-skia"]`
- `wgpu = ["gpu-spike", "dep:wgpu", "dep:pollster"]`

Default resolve features for Hayu itself: only `default`.

`hayu-gpu-spike` has `required-features = ["gpu-spike"]`.

## Important transitive active features

| Transitive | Version | Active features | Note |
|---|---|---|---|
| `wayland-sys` | 0.31.11 | `client`, `dlopen`, `egl`, `once_cell` | activated by glutin and system backend |
| `wayland-client` | 0.31.15 | none declared | default feature set is empty |
| `wayland-backend` | 0.3.17 | `client_system` | libwayland system ABI |
| `calloop-wayland-source` | 0.4.1 | none declared | SCTK calloop bridge |
| `nix` | 0.31.3 | `process`, `signal` | calloop signals |
| `fontdb` | 0.23.0 | `fontconfig`, `fontconfig-parser`, `fs`, `memmap`, `memmap2`, `std` | cosmic-text font discovery |
| `swash` | 0.2.10 | `render`, `scale`, `std` | cosmic-text rasterization |
| `harfrust` | 0.5.2 | `std` | cosmic-text shaping |
| `skrifa` | 0.40.0 | `std` | cosmic-text font parsing |
| `skrifa` | 0.44.0 | `std` | swash font parsing |
| `read-fonts` | 0.37.0 | `libm`, `std` | harfrust + skrifa 0.40 |
| `read-fonts` | 0.41.0 | `std` | skrifa 0.44 |
| `font-types` | 0.11.3 | `bytemuck`, `std` | read-fonts 0.37 |
| `font-types` | 0.12.7 | `bytemuck`, `std` | read-fonts 0.41 |
| `png` | 0.18.1 | none declared | tray plus png internals |
| `flate2` | 1.1.10 | `any_impl`, `default`, `miniz_oxide`, `runtime_detection`, `rust_backend` | PNG DEFLATE |
| `miniz_oxide` | 0.8.9 | `default`, `simd`, `simd-adler32`, `with-alloc` | direct png edge |
| `miniz_oxide` | 0.9.1 | `default`, `simd`, `simd-adler32`, `with-alloc` | flate2 edge |
| `tiny-skia-path` | 0.12.0 | `std` | tiny-skia |
| `atspi` | 0.29.0 | `proxies` | accesskit_unix |
| `atspi-common` | 0.13.0 | `default`, `wrappers`, `zbus` | accesskit_unix |
| `zvariant` | 5.15.1 | `default`, `enumflags2` | zbus/accesskit |
| `rustix` | 1.1.6 | `alloc`, `default`, `event`, `fs`, `net`, `pipe`, `process`, `shm`, `std`, `time` | zbus async-io |
| `toml_edit` | 0.25.18+spec-1.1.0 | `parse` | toml/zbus macro graph |
| `serde_core` | 1.0.229 | `alloc`, `result`, `std` | serde |
| `uuid` | 1.29.0 | `default`, `serde`, `std` | accesskit |
| `zbus_macros` | 5.19.0 | `blocking-api`, `default` | enabling `blocking-api` changes generated proxy surface |
| `serde_derive` | 1.0.229 | `default` | serde derive |

`async-io`, `async-executor`, and `polling` report no declared features while still being active. Their functionality is inherent.

## True multi-version packages

From the default metadata graph, six names have more than one version:

| Name | Versions | Immediate parents | Assessment |
|---|---|---|---|
| `font-types` | 0.11.3, 0.12.7 | 0.11.3 <- `read-fonts 0.37`; 0.12.7 <- `read-fonts 0.41` | cosmic-text dual font stack |
| `read-fonts` | 0.37.0, 0.41.0 | 0.37 <- `harfrust 0.5.2`, `skrifa 0.40`; 0.41 <- `skrifa 0.44` | same dual stack |
| `skrifa` | 0.40.0, 0.44.0 | 0.40 <- `cosmic-text 0.19`; 0.44 <- `swash 0.2.10` | same dual stack |
| `miniz_oxide` | 0.8.9, 0.9.1 | 0.8.9 <- `png 0.18.1`; 0.9.1 <- `flate2 1.1.10` | png pulls both direct miniz and flate2/miniz |
| `syn` | 2.0.119, 3.0.9 | 2.0 <- older proc macros incl. `enumflags2_derive`, `phf_macros`; 3.0 <- `serde_derive`, `zbus_macros`, `zvariant_derive`, `futures-macro` | host proc-macro split |
| `windows-sys` | 0.52.0, 0.61.2 | 0.52 <- `glutin_egl_sys 0.7.1`; 0.61 <- `async-io`, `polling`, `rustix`, `zbus`, etc. | target-conditional; not a Linux binary duplicate |

Not true version duplicates:
- `hashbrown 0.17.1` appears multiple times in `cargo tree -d` due separate graph paths, not two versions.
- `fastrand`, `memchr`, `phf_shared`, `toml_datetime`, `toml_parser` likewise appear through multiple inbound paths in duplicate output but have one version each.
- `cargo tree -d` output alone is not proof of multiple compiled copies or measurable cost.

No direct pin, downgrade, or patch is recommended for these splits without upstream compatibility data. They reflect dependency requirements, especially the cosmic-text/swash font stack.

## Claim verification

### 1. zbus blocking APIs used?

**Verdict: false. No `zbus::blocking` API is used in current source.**

Evidence:
- Entire direct zbus usage (`src/tray.rs`): `zbus::fdo::{NameOwnerChanged, RequestNameFlags, RequestNameReply}`, `zbus::message::Header`, `zbus::{Connection, MatchRule, Proxy, interface}`, `zbus::block_on`, `zbus::MessageStream`, `zbus::Message`.
- No source match for `zbus::blocking`, `use zbus::blocking`, or `blocking::`.
- zbus `block_on` is defined in `zbus-5.19.0/src/utils.rs` under `#[cfg(not(feature = "tokio"))]`; it calls `async_io::block_on`. It is not gated by `blocking-api`.
- `blocking-api` in zbus 5.19.0 adds `pub mod blocking` and a `From<blocking::Connection>` impl. It is not required for `Connection`, `Proxy`, `MessageStream`, `#[interface]`, or the generated async `NameOwnerChanged` proxy.
- `NameOwnerChanged` is the generated async fdo proxy. Hayu calls `NameOwnerChanged::from_message` and awaits no blocking proxy method.
- `zbus_macros` feature `blocking-api` is active only because zbus default enables it; it makes proxy macros generate optional blocking companion APIs. Those APIs are not referenced.
- `cargo tree -e features -i zbus` shows `blocking-api <- zbus feature "default" <- hayu`; no other crate requests it. `accesskit_unix` requests only `async-io`.

Recommended trim, unvalidated:
```toml
zbus = { version = "5", default-features = false, features = ["async-io"] }
```
This keeps async I/O and zbus async APIs. Internal feature `blocking` remains enabled via `async-io` for async-process, but the `blocking-api` public surface and generated blocking proxy companions are dropped. Verify `cargo check` default and all targets before committing. No evidence says this saves runtime memory; only generated code/compile surface.

### 2. wayland-backend direct feature needed for EGL wl_display pointer?

**Verdict: the `client_system` backend feature is required; the direct `wayland-backend` dependency is not required to enable it.**

Evidence:
- Production passes raw libwayland pointers to glutin EGL:
  - `src/app.rs:115` — `conn.display().id().as_ptr() as *mut c_void`
  - `src/app.rs`/bar/launcher surface equivalents, for example `src/bar.rs:179`
  - spike: `src/gpu/present.rs:101`, `src/gpu/gl.rs:329`
- `wayland-backend 0.3.17` defines `ObjectId::as_ptr` in `src/sys/mod.rs` only under `#[cfg(any(test, feature = "client_system"))]`. Without `client_system`, that method is unavailable, so EGL display/surface pointer use does not compile.
- The method returns an actual `*mut wayland_sys::client::wl_proxy`; with the pure-Rust backend there is no libwayland proxy/display pointer to hand to EGL. The system backend is therefore semantically required, not incidental.
- No source imports `wayland_backend::*`. The direct edge exists only to activate the feature.
- `wayland-client 0.31.15` already exposes the identical feature selector:
  ```toml
  system = ["wayland-backend/client_system"]
  ```
  Enabling `wayland-client`'s `system` feature preserves `wayland-backend/client_system` globally through Cargo feature unification.

Recommended shape, unvalidated:
```toml
wayland-client = { version = "0.31", features = ["system"] }
# remove the direct wayland-backend declaration
```
This removes one direct declaration while retaining the required system backend and raw pointer API. Do not remove `client_system` itself. Verify with `cargo tree -e features -i wayland-backend` that `client_system` remains active, then compile and run the live Wayland/GL path. This assumes `wayland-client` remains a direct dependency, which it does.

## Further bounded recommendations

1. Validate the zbus trim above. It is a source-supported reduction, but compile validation is still pending. Do not touch accessibility's `async-io` need.
2. Validate moving `client_system` activation to `wayland-client`'s `system` feature and dropping the direct `wayland-backend` declaration. Keep the feature itself.
3. Leave the six true version splits alone; no safe direct-manifest fix exists. Removing `tiny-skia`'s `png-format` was already applied but cannot remove the `png`/`flate2`/two-miniz chain because `png` is a required tray dependency.
4. `glutin` is already trimmed to `egl` + `wayland`; documentation still describing its old default feature set is stale prose. Do not add X11/GLX/WGL back for this Wayland-only shell.
5. `tiny-skia` is correctly absent from the default normal graph and present only for dev tests plus `gpu-spike`. Its duplicate optional-normal/dev declaration is intentional feature plumbing, not a dependency duplication.
6. No measured binary size, build time, RSS, or latency claims are made. Graph and feature counts are not runtime costs.

## Snapshot versus final state

The inventory in [DEPENDENCIES.md](DEPENDENCIES.md) deliberately labels its table as the original baseline; its applied-changes section and final feature delta supersede baseline states. The two additional trims recommended here were subsequently applied and compiled successfully in `23f7016`. Measurements and final checks are recorded in [REFOCUS_VALIDATION.md](REFOCUS_VALIDATION.md) and [PERFORMANCE.md](PERFORMANCE.md), not inferred from this intermediate feature graph.
