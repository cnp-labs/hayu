# Scope — Hayu V1

Status: planning baseline 0.1, November 2, 2026. See numbered requirements in [PRD.md](PRD.md).

## In scope

All user-selected features: bar/workspaces/clock/window title, launcher, tray, notifications with history, audio/brightness/network/battery control center, media/OSD, wallpaper picker, dock, and clipboard history. Supporting foundations include TOML, hot reload, IPC, light/dark themes, keyboard, IME, accessibility, multiple monitors, scaling, reconnect, and Arch packaging.

Features are implemented incrementally but remain obligations for the first release. A bar-only alpha may be used for testing, but must not be labeled a complete V1.

## Initial feature boundaries

| Area | Proposed V1 minimum | Deferred |
| --- | --- | --- |
| Wallpaper | Local static images, picker, thumbnails, per-output fit/fill | Video, shader wallpapers, online downloads |
| Clipboard | Session-only text history, opt-in, pause/clear/search | Images, device synchronization, disk persistence |
| Network | Status, radio toggles, saved connections, external editor link | Built-in Wi-Fi secret agent and enterprise connection editor |
| Media | MPRIS and player selection | Equalizer, streaming, visualizer |
| Dock | Pinning, running state, launch/focus, window chooser | Magnification, complex drag-and-drop, auto-hide |
| Appearance | Light/dark themes and configurable tokens | Visual theme editor and application templates |
| Bar | Horizontal top/bottom bars and widget lists | Vertical bars and custom script widgets |
| Notifications | Toasts, actions, DND, bounded session history | Disk persistence and online service integrations |

These refinements can be expanded through documented changes. User-selected feature categories must not be removed merely to reduce implementation effort.
Status: only the bar, launcher search, config validation and `doctor` exist. The bar draws `workspaces`, `clock`, `active_window` and tray items; the launcher, media, audio, network, battery, notifications and control_center IDs are validated and draw nothing. The tray has no menus yet, so HAYU-001 and HAYU-004 remain incomplete.


## Outside V1

- Internal locker, PAM authentication, and replacing Hyprlock.
- Window management, tiling, window rules, compositor monitor configuration, and display manager.
- Lua runtime, native/dynamic plugins, marketplace, and script widget execution.
- Bluetooth, weather, screenshots/recording, desktop widgets, and power/session menu.
- Official support for GNOME, KDE, Sway, Niri, X11, Windows, or macOS.
- Installers that rewrite user dotfiles, a separate networking service, and an updater independent of distribution packages.

## Feature admission rules

Describe the user problem, interaction path, additional dependencies, CPU/memory/energy impact, failure modes, testing, and maintenance owner. Features that open a new area belong on the roadmap or require a PRD update. Noctalia inspiration does not create a feature-parity obligation.

Every addition affecting privacy or active resource use must define defaults and disabled behavior. Hiding a widget does not always stop its service: shared services may remain active if other consumers need them.
