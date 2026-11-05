# Hayu

**Lightweight desktop shell.**

Hayu is a Rust-based Wayland desktop shell for Hyprland users, with an internal UI foundation and TOML configuration. Noctalia is a reference for an integrated desktop experience; Hayu's requirements and implementation are defined independently.

**Status: planning documentation v0.1, November 2, 2026 (Asia/Jakarta). This package contains no verified implementation or benchmark results.** “First release” means the first product release that meets the entire V1 scope, rather than the M0 prototype.

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

[examples/config.toml](examples/config.toml) is a **proposed schema example**, not configuration for an available binary. All CLI names, IPC service names, module paths, and design tokens in these documents are proposals until implementation stabilizes them. No installation commands are claimed to work already.

## Decision status

“User-decided” identifies an explicit decision made in discussion. “Initial proposal” identifies a design that can be evaluated during implementation. “Open” requires prototype evidence or a maintainer decision. Performance figures are hypotheses, not marketing claims. The license, Hayu repository URL, Hyprland version, reference hardware, and final visual identity have not been selected.
