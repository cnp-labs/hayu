# Hayu

**Lightweight desktop shell.**

Hayu is a Rust-based Wayland desktop shell for Hyprland users, with an internal UI foundation and TOML configuration. Noctalia is a reference for an integrated desktop experience; Hayu's requirements and implementation are defined independently.

**Status: M0 prototype, November 5, 2026 (Asia/Jakarta).** The planning documents below still define V1. The repository now also contains an M0 code slice: a layer-shell bar with workspaces, clock, and window title, a keyboard-driven application launcher reachable through a local control socket, `hayu config check`, and `hayu doctor`. The renderer is decided (GLES/EGL, ADR-0006) but both surfaces still draw through tiny-skia. No other V1 feature exists and no benchmark of the V1 profiles has been run. “First release” means the first product release that meets the entire V1 scope, rather than this M0 prototype.

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
| [PRD.md](PRD.md) | Users, requirements, acceptance criteria, and release criteria |
| [SCOPE.md](SCOPE.md) | V1 boundaries and scope expansion rules |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Modules, lifecycle, events, rendering, data, and integrations |
| [DESIGN.md](DESIGN.md) | Component system and UI interactions |
| [COMPATIBILITY.md](COMPATIBILITY.md) | Protocols, services, and testing targets |
| [CONFIGURATION.md](CONFIGURATION.md) | Configuration design and hot reload |
| [PERFORMANCE.md](PERFORMANCE.md) | Budget hypotheses and benchmark methodology |
| [ROADMAP.md](ROADMAP.md) | Milestones and exit criteria |
| [TESTING.md](TESTING.md) | Testing strategy and acceptance matrix |
| [DECISIONS.md](DECISIONS.md) | Decisions, assumptions, and open questions |
| [SOURCES.md](SOURCES.md) | Technical references and inspiration |
| [docs/adr/0001-custom-ui.md](docs/adr/0001-custom-ui.md) | Internal UI decision |
| [docs/adr/0002-hyprland-first.md](docs/adr/0002-hyprland-first.md) | Initial compositor and distribution targets |
| [docs/adr/0003-toml.md](docs/adr/0003-toml.md) | TOML configuration decision |
| [docs/adr/0004-renderer-spike.md](docs/adr/0004-renderer-spike.md) | Open renderer evaluation |
| [docs/adr/0005-local-state.md](docs/adr/0005-local-state.md) | Proposed state and privacy model |

## Starting implementation

Start with M0 in [ROADMAP.md](ROADMAP.md). The first deliverable is a prototype covering surfaces, text, input, and scaling in Hyprland; then implement modules incrementally until all V1 requirements are met. This stage does not require a marketplace or plugin runtime.

[examples/config.toml](examples/config.toml) is a **proposed schema example**: `hayu config check` validates it, but most keys describe modules that do not exist yet. All CLI names, IPC service names, module paths, and design tokens in these documents remain proposals until implementation stabilizes them. The M0 slice builds and runs in a Hyprland session:

```
cargo build --release
cargo test
./target/release/hayu doctor
./target/release/hayu config check examples/config.toml
./target/release/hayu run
./target/release/hayu launcher toggle   # with `hayu run` active
./target/release/hayu quit
```

## Decision status

“User-decided” identifies an explicit decision made in discussion. “Initial proposal” identifies a design that can be evaluated during implementation. “Open” requires prototype evidence or a maintainer decision. Performance figures are hypotheses, not marketing claims. The license, Hayu repository URL, Hyprland version, reference hardware, and final visual identity have not been selected.
