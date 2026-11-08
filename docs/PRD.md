# Product Requirements Document — Hayu

Document version: 0.1 • Date: November 2, 2026 • Status: planning baseline, not implemented.

## 1. Summary and problem

Hyprland users need everyday desktop components with consistent visuals and behavior. Combining separate applications can require additional configuration and integration. Hayu brings together the bar, launcher, control panels, notifications, media, wallpaper, dock, and clipboard through a single internal Rust-based UI foundation.

Product direction updated November 7, 2026: simple, minimalist, lightweight and responsive desktop shell; not a general-purpose UI framework. D-12 approves core bar/configuration/reload/IPC, mandatory input/IME/accessibility/scaling/output foundations, and optional ownership for launcher and other feature modules. All existing V1 categories and release criteria remain requirements. Disabled optional modules own no dedicated work or resources; shared services require actual consumers (HAYU-016). See [REFOCUS.md](REFOCUS.md) for the approved division and implementation boundaries.

Motto: **Lightweight desktop shell.** “Lightweight” means measurable resource use, limited idle work, and disabled modules releasing their dedicated work and resources. Rust alone is not evidence of performance.

## 2. Users and goals

Primary users: Hyprland users on Arch Linux or its derivatives who want to configure their desktop through files. This initial persona is a proposal based on the user's environment, not a market research finding.

Goals: consistent daily desktop workflows; good keyboard navigation; predictable TOML configuration; service failures that do not bring down the UI; traceable overhead. Success is measured through the acceptance criteria below, compatibility testing, and performance reports. No user-count target or calendar deadline has been set.

## 3. V1 requirements

Every row below is required for the first product release. Milestones divide implementation work rather than removing features.

| ID | Requirement | Minimum acceptance criteria |
| --- | --- | --- |
| HAYU-001 | Modular bar | TOML-defined left/center/right areas; clock, window title, launcher, and system indicators; no layout overflow on small outputs; clock refresh matches format precision. |
| HAYU-002 | Hyprland workspaces | Show active, occupied/empty, and special workspaces; clicks target the appropriate output; IPC focus/workspace changes appear without routine polling. |
| HAYU-003 | Launcher | Search application names and desktop IDs; fuzzy search, favorites, arrows/Enter/Escape; respect desktop entry visibility and activation rules; visible launch errors; tested IME and Unicode. |
| HAYU-004 | Tray | Support StatusNotifierItem and DBusMenu; icons, status changes, and menus; restarting tray applications produces no duplicate entries; one application's failure does not break the tray. |
| HAYU-005 | Notifications and history | Desktop Notifications D-Bus; replacement IDs, close, actions, timeouts, and urgency; DND; bounded, clearable history; advertised capabilities match implementation. |
| HAYU-006 | Audio control center | Output volume, mute, and active output selection; external changes update state; backend failures are visible and recover after reconnection. |
| HAYU-007 | Brightness | Control available backlights with clear value limits; unsupported differs from 0%; the main shell must not require running as root. |
| HAYU-008 | Network | Connection status, Wi-Fi/radio toggles when authorized, saved connection listing/activation, and disconnect; networks requiring new credentials open an external connection editor. |
| HAYU-009 | Battery | Charging status and percentage; estimates only when available; the widget can be hidden without a battery; updates come from the backend. |
| HAYU-010 | Media and OSD | MPRIS play/pause/previous/next according to capabilities; player selection; title; volume/brightness OSD with timeout and updates that do not stack. |
| HAYU-011 | Wallpaper picker | Select local static images; worker-generated thumbnails; apply per monitor; fit/fill and base color; persist selection; unreadable images produce a fallback. |
| HAYU-012 | Dock | Pin/unpin and launch applications, running indicators, and focus existing windows; choose among multiple windows; application identity mapping has an explicit fallback. |
| HAYU-013 | Clipboard history | Opt-in; capture/reselect text; deduplication, search, individual/all deletion, and pause; count/size limits; session-only data by default. |
| HAYU-014 | TOML and IPC | Schema validation; transactional hot reload; invalid config preserves the last valid config; CLI for panels/launcher/reload/diagnostics; structured results and errors. |
| HAYU-015 | UI foundation | Light/dark themes, visible keyboard focus, Unicode text and IME, scales 1/1.25/1.5/2; hotplug; the bar does not normally take keyboard focus. |
| HAYU-016 | Lifecycle and failure handling | One instance per session; shutdown cleans resources; service restarts recover; disabled modules stop dedicated work; nonfatal errors appear in diagnostics. |

The minimum network, text clipboard, media, and wallpaper details above are **proposed V1 refinements**: the user selected feature categories without specifying every control. Refinements are recorded in the PRD and decision log before the relevant implementation, without requiring another clarification round for this documentation task.

## 4. Primary workflows

1. Startup: read configuration and capabilities, create bars on matching outputs, connect services, and report readiness.
2. Launch an application: a compositor keybinding opens the launcher, input receives focus, the user selects a result, the application launches, and shell focus is released.
3. Quick controls: open the panel from the bar, change volume/brightness, show pending/error if the backend is slow, and synchronize the actual result.
4. Wallpaper/dock: select an image or favorite; write state atomically; changes appear immediately and survive restart.
5. Clipboard: enable history; bounded capture runs during the session; pause stops capture; clearing also removes Hayu-managed metadata.

## 5. Nonfunctional requirements

- No global redraw loop while all surfaces are static. Bound queues, thumbnails, icons, notifications, and clipboard data.
- File operations, service IPC, expensive search, and image decoding must not block the UI.
- Accessibility is part of the foundation: model semantics, roles, focus, and labels from the start. A screen-reader-usable AT-SPI bridge is a release gate, not an existing capability.
- Clipboard and notification contents never enter logs. No default V1 telemetry or HTTP requests.
- A full settings GUI is optional; all settings are documented through TOML.
- Measure minimal core and the V1 feature profile separately; candidate budgets are in [PERFORMANCE.md](PERFORMANCE.md).

## 6. Boundaries and release criteria

V1 does not implement a compositor, locker, display manager, Lua runtime, plugin marketplace, or support for other distributions. Users manage Hyprlock. Bluetooth, screenshots, weather, and animated wallpapers are post-V1 roadmap items.

V1 passes when HAYU-001 through HAYU-016 meet [TESTING.md](TESTING.md), no crash/data-loss/input-access/privacy blockers remain, dependencies and packaging are reproducible, and performance profiles are reported with actual versions and hardware. Confirm budget hypotheses or revise them with reasons and maintainer approval before the release gate; do not silently remove them after a failure.

## 7. Main risks

Internal UI expands text, IME, and accessibility work; V1 has a broad scope; compositor IPC/protocols may change; desktop ID/app_id mapping is not always exact; hardware permissions and services vary by installation. Mitigations: M0/M1 spikes, separate adapters, bounded state, reconnect tests, real application fixtures, and a dependency/capability matrix.

## 8. Open decisions

Renderer and event loop, text libraries, accessibility bridge, minimum Hyprland version, reference machine, architectures beyond candidate x86_64, branding, and license. Follow-up owner: Hayu maintainer; resolution follows [ROADMAP.md](ROADMAP.md) gates.
