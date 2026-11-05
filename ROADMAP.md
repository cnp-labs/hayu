# Hayu implementation roadmap

Status: proposed work sequence without calendar deadlines. Every user-selected feature category remains required for V1. Contributor count has not been determined.

## Milestones and exit criteria

| Milestone | Deliverables | Exit criteria | Dependencies |
| --- | --- | --- | --- |
| M0 — Platform/rendering | Surfaces, shapes/text, outputs/input; renderer prototypes; Hyprland snapshots/events | Compare candidate renderers with the same scene or reject one with evidence; record resources; no hotplug crashes; record hardware/versions; resolve renderer ADR | Initial decisions |
| M1 — UI/config foundation | Layout, components, focus, TextInput/IME, semantics and AT-SPI spike, invalidation, TOML reload, IPC | Proven keyboard and IME support; successful screen reader spike; invalid reload preserves runtime; no idle redraw loop; reviewed budgets | M0 |
| M2 — Bar/launcher/tray | Workspaces, clock, title, application search, tray/menu | HAYU-001–004 and relevant 014–016 portions tested; real desktop entries; correct multi-monitor/focus behavior | M1 |
| M3 — Services/panels | Notifications/history, control center, audio/network/battery/brightness, MPRIS/OSD | HAYU-005–010 pass, including conflicts, permissions, and reconnect; tested history caps | M2 and adapters |
| M4 — Wallpaper/dock | Static wallpaper picker, thumbnail workers, pinning and window chooser | HAYU-011–012 pass; atomic state, bounded caches, clear identity fallback | M1/M2 |
| M5 — Clipboard/integration | Opt-in text history, search/pause/clear, data-control lifecycle | HAYU-013 passes; bounded FDs; no payload in logs/on disk; external Hyprlock workflow unaffected | M0/M1 |
| M6 — Hardening/release | Soak, full-feature performance, accessibility, Arch recipe, installation | All HAYU-001–016 pass; clean build/install/uninstall; selected license; actual reports available | M0–M5 |

M2/M3 alphas may be shared for testing with recorded limitations; only label V1 complete after M6. Milestones divide implementation work rather than reducing feature obligations.

## Risk priorities

1. Prove text/IME, fractional scaling, and accessibility before many surfaces depend on UI APIs.
2. Validate resource lifetimes when outputs disappear before adding decoration.
3. Establish reconnect, queue caps, and configuration transactions before adding many services.
4. Share the application identity resolver between launcher and dock.
5. Test all features together before publishing performance claims.

## After V1

Uncommitted candidates: Lua/scripting API, plugin lifecycle, Bluetooth, screenshots/recording, weather, animated wallpapers, dock auto-hide, vertical bars, theme generator, and other compositors/distributions. An internal locker is not automatically the next task.

## Plan changes

Feature changes refer to PRD IDs and update acceptance criteria, scope, tests, and budgets. Foundational UI/config/renderer/privacy changes require ADRs. Produce calendar estimates after M0 because internal UI effort has not been proven.
