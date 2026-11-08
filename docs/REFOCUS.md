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

## Approved scope and lifecycle responsibilities

The follow-up user decision approves this division; it supersedes the earlier proposal that placed the launcher in core.

- **Core:** bar with workspaces, clock and window title; configuration, hot reload and control IPC.
- **Mandatory foundations:** keyboard and pointer input, IME, accessibility, integer/fractional scaling, and surface/output lifecycle including hotplug. Foundations serve existing consumers; disabling an optional surface does not cancel these correctness requirements.
- **Optional modules:** launcher, tray, notifications/history, control center, media/OSD, wallpaper picker, dock and clipboard history. All V1 categories remain required by the agreed release gates; optional means enablement and resource ownership, not cancellation or a smaller release. Clipboard remains opt-in.
- **Lifecycle contract:** a disabled optional module starts no dedicated worker, timer, subscription, service connection or module resource load. An enabled flag alone is not an active service consumer. Shared resources remain while another real consumer needs them; output loss, reload and shutdown remove consumers. Future modules must define start/stop/restart and stale-result rejection before implementation, without a generic service framework.
- **Roadmap:** stabilize existing slices and measure them first, then implement service-backed modules independently and close full V1 acceptance/packaging gates. No roadmap modules are implemented solely to satisfy this division.

## Engineering policy

Keep standard protocol/parser/text/accessibility libraries: replacing shaping, Unicode, Wayland, D-Bus or AT-SPI with partial implementations costs correctness and development time. Narrow Hayu layout and hit testing stay internal. Crate count is not runtime cost: optional lockfile entries do not load into default binaries. Internal code saves dependencies only when its maintenance, validation and edge cases are genuinely smaller.

Use explicit dirty state and bounded queues/resources before adding generic services or widget abstractions. Shared GPU contexts/resources across outputs require lifecycle evidence; do not trade proven make-current correctness for speculative memory savings. Avoid timer-driven animation or redraw while hidden. Measure before claiming resource gains.

See [REFOCUS_VALIDATION.md](REFOCUS_VALIDATION.md) for executed checks and limitations, [DEPENDENCIES.md](DEPENDENCIES.md) for the complete direct-dependency inventory, [AMANE.md](AMANE.md) for pinned technical references, and [PERFORMANCE.md](PERFORMANCE.md) for methods/results. Amane contributes reference patterns only; no source, assets or dependency are reused.

## Structure after refocus

Root keeps `README.md`, `AGENTS.md`, `LICENSE`, `Cargo.toml` and `Cargo.lock`. `docs/` holds specification, audits, methods and retained `docs/adr/` history. `src/app.rs` owns lifecycle/input/service orchestration; `src/bar.rs` owns bar surfaces and drawing, `src/launcher.rs` launcher state/surface, `src/render.rs` and `src/render/gl.rs` production frame/GLES code, and `src/gpu/` opt-in spike code. Existing config, desktop, Hyprland, IPC, tray and accessibility modules keep their bounded responsibilities. `tools/live/` remains local to its harness. No workspace or generic service/widget framework is introduced.

## Remaining risks

The lifecycle follow-up implements owned Hyprland/tray workers and actual-consumer reconciliation under ADR-0008; validation evidence must establish cancellation, response ordering and reload behavior before these are considered verified. Tray snapshots/render inputs share icon pixels; the combined upload strip still copies pixels, and upload versioning is not a CPU allocation cache. Tray uploads now query the actual context's `GL_MAX_TEXTURE_SIZE` and downsample oversized strips before upload; per-icon caps alone were insufficient. Focused tests cover synthetic limits and all 64 icon cells, not a hardware compatibility matrix. Icon refresh/theme inheritance, live fractional/hotplug coverage, AT-SPI action completeness and long-duration whole-process measurements require explicit evidence. Do not infer these from passing ordinary unit tests or short smoke runs.
