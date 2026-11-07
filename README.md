# Hayu

**Lightweight desktop shell.**

Hayu is a simple, minimalist Rust Wayland desktop shell for Hyprland, configured with TOML. Fast input, readable UI, event-driven rendering and low background work matter more than framework flexibility or crate counts. GLES/EGL remains the production renderer. See [product refocus](docs/REFOCUS.md), [dependency audit](docs/DEPENDENCIES.md), and [Amane technical references](docs/AMANE.md).

**Status: prototype, November 7, 2026 (Asia/Jakarta).** Implemented slices: layer-shell bar with workspace left click, clock, title and configurable areas; keyboard application launcher over local IPC, including IME protocol and AccessKit tree; minimal tray registration, icons and left-click activation; reload, `hayu config check`, and `hayu doctor`. These are partial V1 requirements, not a completed milestone or full accessibility claim. Both surfaces use GLES/EGL with glow (ADR-0006). CPU reference and GPU spike remain test/experimental tooling. No full V1 profile benchmark exists. “First release” still means full agreed V1; proposed core/optional priorities do not silently shrink it.

## User decisions

- Rust is the primary language; Hyprland is the primary compositor; Arch Linux or its derivatives are the initial distribution target.
- Components, layout, interaction, and the UI system are developed internally.
- TOML is the primary configuration format; scripting is deferred.
- V1 includes the bar/workspaces/clock/window title, launcher, tray, notifications and history, audio/brightness/network/battery control center, media and OSD, wallpaper picker, dock, and clipboard history.
- An internal lock screen is outside V1; users continue using Hyprlock.
- All Markdown files and documentation use English, as subsequently requested by the user.

## Documentation guide

| Document | Purpose |
| --- | --- |
| [PRD.md](docs/PRD.md) | Users, requirements, acceptance criteria, and release criteria |
| [SCOPE.md](docs/SCOPE.md) | V1 boundaries and scope expansion rules |
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | Modules, lifecycle, events, rendering, data, and integrations |
| [DESIGN.md](docs/DESIGN.md) | Component system and UI interactions |
| [COMPATIBILITY.md](docs/COMPATIBILITY.md) | Protocols, services, and testing targets |
| [CONFIGURATION.md](docs/CONFIGURATION.md) | Configuration design and hot reload |
| [PERFORMANCE.md](docs/PERFORMANCE.md) | Budget hypotheses and benchmark methodology |
| [ROADMAP.md](docs/ROADMAP.md) | Milestones and exit criteria |
| [TESTING.md](docs/TESTING.md) | Testing strategy and acceptance matrix |
| [DECISIONS.md](docs/DECISIONS.md) | Decisions, assumptions, and open questions |
| [SOURCES.md](docs/SOURCES.md) | Technical references and inspiration |
| [docs/adr/0001-custom-ui.md](docs/adr/0001-custom-ui.md) | Internal UI decision |
| [docs/adr/0002-hyprland-first.md](docs/adr/0002-hyprland-first.md) | Initial compositor and distribution targets |
| [docs/adr/0003-toml.md](docs/adr/0003-toml.md) | TOML configuration decision |
| [docs/adr/0004-renderer-spike.md](docs/adr/0004-renderer-spike.md) | Open renderer evaluation |
| [docs/adr/0005-local-state.md](docs/adr/0005-local-state.md) | Proposed state and privacy model |

## Starting implementation

Start with M0 in [ROADMAP.md](docs/ROADMAP.md). The first deliverable is a prototype covering surfaces, text, input, and scaling in Hyprland; then implement modules incrementally until all V1 requirements are met. This stage does not require a marketplace or plugin runtime.

[examples/config.toml](examples/config.toml) is a **proposed schema example**: `hayu config check` validates it, but most keys describe modules that do not exist yet. All CLI names, IPC service names, module paths, and design tokens in these documents remain proposals until implementation stabilizes them. The M0 slice builds and runs in a Hyprland session:

```
cargo build --release
cargo test
# Optional renderer experiments (not needed to run Hayu):
# cargo build --release --features gpu-spike
# cargo build --release --features wgpu
./target/release/hayu doctor
./target/release/hayu config check examples/config.toml
./target/release/hayu run
./target/release/hayu launcher toggle   # with `hayu run` active
./target/release/hayu reload             # with `hayu run` active
./target/release/hayu quit
```

## Decision status

“User-decided” identifies an explicit decision made in discussion. “Initial proposal” identifies a design that can be evaluated during implementation. “Open” requires prototype evidence or a maintainer decision. Performance figures are hypotheses, not marketing claims. The repository is [cnp-labs/hayu](https://github.com/cnp-labs/hayu), licensed under MIT. Recorded compositor/hardware test environments are evidence for those runs, not a finalized support matrix. Final visual identity remains open.
