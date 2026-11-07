# Hayu implementation roadmap

Status: proposed work sequence without calendar deadlines. Every user-selected feature category remains required for V1. Contributor count has not been determined.

## November 7 refocus

Prioritize stabilization and measurement of existing bar/launcher/tray slices before expanding service integrations. Keep existing V1 feature commitments and milestone gates; proposed core/optional priorities require product approval ([REFOCUS.md](REFOCUS.md)). This refocus is not milestone closure.

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

## M0 status (November 5, 2026)
Implemented in `../src/`: layer-shell bar per output with hotplug, cosmic-text labels, Hyprland snapshot plus event resync, `hayu config check`, and `hayu doctor`. Renderer decided in ADR-0006 (GLES/EGL with glow, CPU rasterizer retained only for tests) and moved into the bar and launcher at `f105f22` during M1. M0 also has a launcher slice: an overlay surface, an XDG desktop-entry catalogue that is rescanned on open, fuzzy filtering, keyboard selection with compositor-driven repeat, `Terminal=true` wrapping in each emulator's documented form, and a local control socket (`hayu launcher toggle|open|close`, `hayu reload`, `hayu quit`) with owner-only permissions. The full keyboard flow was verified live with injected Wayland key events, on one output at scale 1 and scale 2, on a second output, and across a runtime keymap switch; removing the output the launcher was open on left the shell running and the next open landed on the remaining output. M0 left IME, fractional and mixed per-output scales, and the production render interface open; the render interface, the IME slice and fractional plus mixed per-output scaling have since landed in M1. The bar and launcher are prototypes, not V1 surfaces: no tray, panels, icons, favorites, or theming beyond the prototype tokens. `bar.left`, `bar.center` and `bar.right` are read at draw time, so a reload re-lays out the bar; only `workspaces`, `clock` and `active_window` have modules, and the other accepted IDs draw nothing. A left click on a drawn workspace pill switches workspace. A minimal tray hosts StatusNotifierItems when no other host owns the watcher, draws their icons and activates them on a left click; context menus, scroll and icon refresh are not wired yet.

M1 has started with TOML reload: `hayu reload` applies theme, clock format, bar height/position, `bar.enabled`, `bar.outputs` and `launcher.enabled`, reconciles the bar surfaces with the output filter, names the keys that still need a restart, and rejects an invalid file without disturbing the running shell (verified live, see [TESTING.md](TESTING.md). Two idle runs with independent logs show no redraw outside Hyprland events, and the second one crossed two minute boundaries with exactly one commit per minute, which is the evidence for the "no idle redraw loop" exit criterion. The IME slice is wired (`zwp_text_input_v3`, ADR-0007) and was run end to end with fcitx5 and fcitx5-anthy: preedit is inline, one commit inserts once, BackSpace works inside and after composition, and close, reload, quit and output removal tear it down cleanly. One interaction is open: after a cancelled composition the input method consumes the next Escape, so I4 in ADR-0007 is not met and IME acceptance is not complete. The launcher AT-SPI spike landed at `c8017ee`: the overlay is registered as a window, the search field exposes name/role/value, results appear as a list with the active row marked selected, close hides the tree, and reopen resets it. That is bus-level evidence only; no screen reader was installed, so T-17 and the screen-reader exit criterion remain open, and the spike is read-only, launcher-only and does not expose the IME preedit. The render interface moved to GLES/EGL at `f105f22`: frames carry rects and atlas-backed glyph quads, each bar and the launcher own an EGL presenter, and the live input, reload and AT-SPI checks still pass. Fractional scaling landed at `4f8327d`: each layer surface follows `wp_fractional_scale_v1` through a `wp_viewport` destination and falls back to an integer `buffer_scale` when either global is missing; a live run at 1.5 next to a scale-1 output presented 1920×48 and 1366×32 buffers, and changing that output to scale 2 resized the same surface without a restart. The remaining M1 exit criteria are still open.

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
