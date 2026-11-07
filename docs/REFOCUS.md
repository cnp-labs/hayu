# Hayu product refocus — November 7, 2026

## Authority and limits

The user's refocus request supersedes conflicting goals: Hayu is a simple, minimalist, lightweight and responsive desktop shell, not a general-purpose UI framework. Evaluate actual CPU, memory, GPU activity and background work together with usability. Rust, Hyprland-first, TOML, existing IME/accessibility and GLES/EGL remain. Historical ADRs remain evidence, not files to overwrite. No working feature or agreed V1 category is removed by this refocus.

Starting revision: `ab9e678204ca4224468368c5d1e044cf649b1413`; working tree was clean. Subsequent documentation moves were agent changes, not pre-existing user edits. Changes are local commits only; no push or PR is authorized by this request.

## Actual implementation versus intent

| Area | Current evidence and boundary |
| --- | --- |
| Bar | Workspaces, title, clock, configurable areas, workspace left click, fractional-scale protocol and output lifecycle exist. Other accepted widget IDs are not proof of implementation. |
| Launcher | Desktop-entry search, keyboard input, terminal execution, control socket and reused layer surface exist. IME protocol and AccessKit tree exist; neither proves comprehensive screen-reader compatibility. |
| Tray | Registration/removal, ARGB pixmap or PNG icon, left-click Activate and non-displacing watcher ownership exist. Menus, scroll, live icon/title refresh and full icon-theme resolution remain absent. |
| Integrations | Hyprland owns command/event sockets; local IPC and reload exist. Some Hyprland queries/dispatch remain synchronous on UI paths. |
| Renderer | Production GLES/EGL and glyph atlas are implemented. CPU reference and alternative wgpu renderer are experiments/testing, not alternative shipping renderers. |
| V1 | Notification history, control center, media/OSD, wallpaper picker, dock and clipboard are requirements/plans, not working modules. Packaging and CI remain absent. |

## Scope recommendation — product approval still required

- **Core delivery priority:** bar, launcher, configuration/IPC, keyboard/pointer, IME, accessibility, output/scaling and service-failure handling. These are daily entry points or correctness foundations. This is a prioritization proposal, not a newly approved smaller V1 release.
- **Optional modules:** tray, notifications/history, audio/network/brightness/battery controls, media/OSD, wallpaper, dock and clipboard. Keep existing V1 categories; propose explicit enablement and no dedicated worker/subscription without a consumer. Clipboard privacy remains opt-in. Accessibility is not an expendable optional visual feature.
- **Roadmap:** stabilize existing slices and measure them first; implement service-backed modules independently; then close full V1 acceptance, packaging and distribution gates. A smaller first-release scope would require explicit approval changing D-06 and release gates.

## Engineering policy

Keep standard protocol/parser/text/accessibility libraries: replacing shaping, Unicode, Wayland, D-Bus or AT-SPI with partial implementations costs correctness and development time. Narrow Hayu layout and hit testing stay internal. Crate count is not runtime cost: optional lockfile entries do not load into default binaries. Internal code saves dependencies only when its maintenance, validation and edge cases are genuinely smaller.

Use explicit dirty state and bounded queues/resources before adding generic services or widget abstractions. Shared GPU contexts/resources across outputs require lifecycle evidence; do not trade proven make-current correctness for speculative memory savings. Avoid timer-driven animation or redraw while hidden. Measure before claiming resource gains.

See [REFOCUS_VALIDATION.md](REFOCUS_VALIDATION.md) for executed checks and limitations, [DEPENDENCIES.md](DEPENDENCIES.md) for the complete direct-dependency inventory, [AMANE.md](AMANE.md) for pinned technical references, and [PERFORMANCE.md](PERFORMANCE.md) for methods/results. Amane contributes reference patterns only; no source, assets or dependency are reused.

## Structure after refocus

Root keeps `README.md`, `AGENTS.md`, `LICENSE`, `Cargo.toml` and `Cargo.lock`. `docs/` holds specification, audits, methods and retained `docs/adr/` history. `src/app.rs` owns lifecycle/input/service orchestration; `src/bar.rs` owns bar surfaces and drawing, `src/launcher.rs` launcher state/surface, `src/render.rs` and `src/render/gl.rs` production frame/GLES code, and `src/gpu/` opt-in spike code. Existing config, desktop, Hyprland, IPC, tray and accessibility modules keep their bounded responsibilities. `tools/live/` remains local to its harness. No workspace or generic service/widget framework is introduced.

## Remaining risks

Hyprland synchronous request paths can stall input during service failure; moving them to the existing adapter worker is follow-up work, not claimed fixed here. `tray.enabled=false` avoids starting its worker, but an enabled tray without a visible tray widget can still subscribe; tying services to dynamic consumer counts needs explicit reload/lifecycle handling. Tray RGBA snapshots/render inputs and strips still copy data; upload versioning is not a CPU allocation cache. Icon refresh/theme inheritance, a tray strip that respects actual `GL_MAX_TEXTURE_SIZE` (per-icon caps alone do not guarantee this), live fractional/hotplug coverage, AT-SPI action completeness and long-duration whole-process measurements require explicit evidence. Do not infer these from passing ordinary unit tests or short smoke runs.
