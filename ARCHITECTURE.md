# Hayu architecture

Status: initial design 0.1, not a description of an existing implementation. The user decided on internal UI/Rust/Hyprland/TOML; the following module organization and process model are proposals.

## 1. System boundaries and modules

The baseline is one modular shell process, with a lightweight CLI as an IPC client. Additional process separation is considered if isolation or profiling requires it. These are logical module names; they do not all need to be separate crates initially.

| Module | Responsibility | Boundary |
| --- | --- | --- |
| platform/wayland | Registry, capabilities, outputs, surfaces, input lifecycle | No widget presentation policy |
| platform/hyprland | Workspace/window/output snapshots, event stream, compositor actions | One central adapter; no raw IPC calls from widgets |
| render | Backend, buffers/textures, shapes, clipping, text, GPU resources | GPU handles do not leak into shell components |
| ui | Component tree, layout, focus, hit testing, semantics | No synchronous service IO |
| services | Audio, battery, network, MPRIS, tray, notification server, clipboard, wallpaper | Typed state; modeled errors and reconnection |
| shell | Bar, launcher, control center, dock, toast/history, OSD, pickers | Uses UI components and service actions |
| config | Deserialization, schema, validation, diff, reload | Does not partially activate invalid configuration |
| ipc | CLI commands, request/response, readiness | Not a general shell-command execution API |
| state | Favorites, pinning, wallpaper selection, cache metadata | No sensitive clipboard/history persistence by default |

## 2. Data model and event flow

Wayland/Hyprland/D-Bus/file-watcher events enter adapters; adapters produce typed events; state owners update snapshots; only affected consumers become dirty; the scheduler requests the necessary update/layout/redraw. UI actions go to service owners, and actual results return through events.

Service state uses `unavailable`, `connecting`, `ready`, `degraded`, and `error`, with reasons. Unknown values are not shown as zero. The UI may show pending actions and restore actual values when a backend rejects a request.

## 3. Threads, event loop, and backpressure

One UI thread owns Wayland dispatch, the UI tree, and renderer resources. Large file operations, thumbnails, decoding, indexing, and blocking work run on workers. Choose the final executor/event loop in M0; evaluate candidate calloop/async integration by wakeups and lifecycle rather than framework count.

Queues have explicit limits and policies. Volume/workspace state updates may coalesce to the latest value; user actions and surface lifecycle events must not be silently dropped. Do not hold state locks while awaiting IO. Worker results carry generation IDs so stale results after reload cannot replace newer data. Shutdown cancels work, closes channels, and releases handles.

## 4. UI pipeline

Frame phases: apply events/update → measure/layout if geometry changed → render if visuals changed → commit/frame callback. Do not mutate the tree or run layout during rendering. Components request invalidation instead of rendering directly inside callbacks.

Each output tracks logical size, scale, transform, buffer size, and resource generation. Hit testing uses logical coordinates; visual clips match hit regions. Check preferred scale and fractional-scale/viewporter support; document integer-scale fallbacks. Scale changes invalidate related text/geometry caches. Hidden surfaces without animation needs do not continuously request frames.

The renderer is not selected: see [ADR-0004](docs/adr/0004-renderer-spike.md). Define a minimal interface for shapes, text runs, images, clips, and presentation. A replaceable backend is not a promise to support multiple production renderers in V1.

## 5. Surfaces and focus

The bar uses layer-shell and an exclusive zone matching its calculated position/height. The dock must not reserve space that conflicts with the bar. Panels/launcher/toasts/OSD have separate output and focus policies; do not create a full-screen overlay that captures all input by default.

Interactive launchers/panels obtain keyboard focus when opened; the bar, wallpaper, toasts, and OSD do not steal it. Escape/dismiss closes panels and releases keyboard interactivity. Test outside-click and popup dismissal against actual surfaces/input regions. Do not use session-lock protocols for ordinary overlays.

When a monitor disappears, cancel its surfaces/targets before later callbacks can use them. Close interactive panels and destroy that output's bar/dock. Subsequent panels follow the focused output by default. Configuration uses output names, which may require adjustment when device names change.

## 6. Hyprland adapter

Use the request socket for snapshots/actions and the event socket for updates. Open the event stream, buffer events while obtaining the snapshot, reconcile state, then process buffered events. Do not assume a snapshot+stream transaction exists: detect ambiguous state and perform bounded resynchronization. Connection ordering alone does not establish race freedom.

Requests use short-lived connections, timeouts, bounded response sizes, and closure after completion. The event parser handles partial reads, multiple lines in one read, unknown payloads, and disconnects. Reconnect uses bounded backoff; obtain a new snapshot after connection. The UI may retain last-known state with a stale indicator. Do not poll `hyprctl` per widget.

## 7. Integration contracts

| Service | Design |
| --- | --- |
| Audio | Choose a backend adapter in M0/M1 for PipeWire/WirePlumber installations; evaluate native or PulseAudio-compatible interfaces rather than treating them as identical |
| Network/battery/media | NetworkManager, UPower, and MPRIS over D-Bus; candidate Rust bindings: zbus; subscribe and resnapshot after owner changes |
| Tray | StatusNotifier watcher/host and DBusMenu; diagnose ownership conflicts, async menus, isolated item failures |
| Notifications | Own `org.freedesktop.Notifications` only when available; do not forcibly replace another owner; expose implemented capabilities only |
| Brightness | Discover backlights; helpers/permissions follow the selected distribution mechanism; no global sysfs chmod or privileged main shell |
| Clipboard | Use the data-control capability available in the tested Hyprland version; bounded async FD reads/writes; opt-in capture, paused on session lock when a signal is available |
| Wallpaper | Per-output background layer, worker decoding, bounded texture cache; invalid files fall back to a base color |

Automatic Hyprlock integration is not required in V1. If a reliable lock state is unavailable, diagnostics explain the limitation; users may invoke Hayu pause/clear actions from their own lock workflow. Do not claim sensitive clipboard data is always detectable; clearing Hayu history also does not guarantee removal of other applications' clipboard data.

## 8. Configuration, state, and IPC

Proposed locations follow XDG: TOML config, versioned JSON state, regenerable caches, and a socket in a private runtime directory. See [CONFIGURATION.md](CONFIGURATION.md). Read a new configuration into a candidate, validate it, prepare required resources, and swap; failures preserve the previous operational state.

Proposed CLI: `hayu run`, `hayu doctor`, `hayu config check`, `hayu config reload`, `hayu launcher toggle`, `hayu panel toggle control-center`, `hayu clipboard pause`, `hayu clipboard clear`, `hayu quit`. This is a planned contract, not an available executable. Local IPC is versioned and size-bounded, with success/error responses and correlation IDs; restrict access to the socket owner, with directory mode 0700 and socket mode 0600.

## 9. Reliability, privacy, and observability

Modules have start/stop/reconnect lifecycles; shared services stay alive while consumers exist. Disabling the last consumer releases dedicated subscriptions, timers, and resources. Rust memory safety does not remove input/FFI validation or GPU lifecycle requirements.

Structured logs contain module, error class, and duration without clipboard/notification contents, secrets, network credentials, or personal data. `doctor` reports capabilities/dependencies rather than sensitive payload dumps. Rate-limit logs and retries; cache/history caps are in CONFIGURATION. Crash reports and debug traces are not uploaded automatically.
