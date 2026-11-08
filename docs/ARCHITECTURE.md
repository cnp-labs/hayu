# Hayu architecture

Status: initial design 0.2. The user decided on internal UI/Rust/Hyprland/TOML; the module organization and process model below remain proposals. `src/` implements only an M0 slice: Wayland surfaces, the Hyprland adapter, the GLES/EGL render path, the bar, config validation, and `doctor`.

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

One UI thread owns Wayland dispatch, the UI tree, and renderer resources. Large file operations, thumbnails, decoding, indexing, and blocking work run on workers. The prototype uses calloop with worker channels. ADR-0008 moves runtime Hyprland discovery, snapshots, reconnect refresh and workspace actions to owned adapter workers; the UI submits requests and applies generation-tagged results. Launcher catalogue scans still run when opening. General worker/backpressure rules below remain contracts for future modules, not evidence that every file operation is asynchronous.

Queues have explicit limits and policies. Volume/workspace state updates may coalesce to the latest value; user actions and surface lifecycle events must not be silently dropped. Do not hold state locks while awaiting IO. Worker results carry generation IDs so stale results after reload cannot replace newer data. Shutdown cancels work, closes channels, and releases handles.

## 4. UI pipeline

Frame phases: apply events/update → measure/layout if geometry changed → render if visuals changed → commit/frame callback. Do not mutate the tree or run layout during rendering. Components request invalidation instead of rendering directly inside callbacks.

Each output tracks logical size, scale, transform, buffer size, and resource generation. Hit testing uses logical coordinates; visual clips match hit regions. Scale comes from `wp_fractional_scale_v1` when it and `wp_viewporter` are both offered: the fractional object and a `wp_viewport` are created before the surface's first commit, the surface keeps `wl_surface.buffer_scale` at 1, the viewport destination is the logical size, and the buffer is `round(logical × preferred_scale/120)`. Without them the output's integer scale sets `buffer_scale` and no viewport is used, and the shell logs that limitation once. Integer output-scale events are ignored while a fractional object owns the surface. Scale changes invalidate related text/geometry caches. Hidden surfaces without animation needs do not continuously request frames.

The renderer is GLES/EGL with glow ([ADR-0006](adr/0006-renderer-criteria.md)). `src/render.rs` owns the backend-neutral interface: a `Frame` is a clear colour plus solid rects, RGBA sprite quads and glyph quads, and an `Atlas` rasterizes a glyph the first time it is drawn and reuses it afterwards. Text is shaped and rasterized during frame construction, never by a backend.
`src/render/gl.rs` presents a frame on one layer surface: one EGL context and window surface per output, `make_current` before every draw because outputs do not share a context, a dynamic vertex buffer rebuilt from the frame, a full atlas upload only when its version changed, and `swap_buffers` as the surface commit. The EGL window surface follows the frame's pixel size, so a fractional preferred scale only changes that size and the viewport destination. Surfaces own their atlas and presenter, and the presenter drops before the layer surface whose `wl_surface` it wraps.
`render::build` lays the bar out from `bar.left`, `bar.center` and `bar.right` at draw time, so a reload re-applies the module lists without a restart. The centre area is measured first and starts at the frame's centre; the side areas get the room left over. A module that does not fit is dropped rather than allowed to overflow: workspaces stop at the first pill that does not fit, the title is ellipsized, and a clock or title that still does not fit is skipped. Config validation rejects IDs outside `BAR_MODULES`; IDs whose module does not exist yet draw nothing.
`render::rasterize` is the reference implementation for tests: it consumes the same frame and atlas and returns a pixmap. It is not a user-selectable fallback; a second production backend is not promised for V1. `src/gpu/` and the spike binary require `gpu-spike` (or `wgpu`, which implies it); tiny-skia is available only to tests/spike. No frame loop is added. `src/app.rs` owns application lifecycle, input and service integration; `src/bar.rs` owns per-output bar surface resources and drawing. Bar events mark dirty state, flushed once after a calloop dispatch cycle, so multiple events in that cycle share a draw. This is not compositor frame-callback pacing. Clock timers align to the next second for unescaped `%S`, otherwise the next minute; no clock timer is installed without an actual bar surface consuming the clock module, including unmatched output filters or all outputs removed. Reload rebuilds the timer cadence immediately. Launcher drawing still follows input/configure events and closing detaches its buffer.

### Launcher accessibility spike

`src/a11y.rs` publishes a read-only AccessKit tree through `accesskit_unix`
(AT-SPI). The UI thread copies launcher state after draws and on close or
surface removal; a shared snapshot supplies the initial tree when accessibility
activates later. Adapter callbacks run on its worker thread and do not access
Wayland or renderer resources. The adapter exists only while the optional launcher is enabled; disabling it closes the surface and drops accessibility state. No polling or frame timer is added.

The open tree contains a window, a named search field with committed query
value, and named result items with the active selection flag. Closing removes
children and hides the root. This is a launcher-only spike, not the proposed
component semantics system: actions, text editing/caret APIs, IME preedit,
geometry and bar semantics are not covered. Screen reader acceptance remains
separate from AT-SPI bus inspection; see [TESTING.md](TESTING.md).

## 5. Surfaces and focus

The bar uses layer-shell and an exclusive zone matching its calculated position/height. The dock must not reserve space that conflicts with the bar. Panels/launcher/toasts/OSD have separate output and focus policies; do not create a full-screen overlay that captures all input by default.

The bar keeps `KeyboardInteractivity::None` but accepts pointer input: the seat's `wl_pointer` is bound, and a left press is hit-tested against the workspace pills recorded in the frame that was drawn. Drawing and hit testing therefore read one list; a click outside those pills is ignored, and the bar has no keyboard focus. Workspace switching queues an ordered action on the owned Hyprland service. Its adapter tries legacy `dispatch workspace <id>` first and falls back to Hyprland 0.57's Lua `repl` form only after an explicit rejection. Transport timeout/disconnect is not grounds to retry a possibly executed action.
Interactive launchers/panels obtain keyboard focus when opened; the bar, wallpaper, toasts, and OSD do not steal it. Escape/dismiss closes panels and releases keyboard interactivity. Test outside-click and popup dismissal against actual surfaces/input regions. Do not use session-lock protocols for ordinary overlays.

When a monitor disappears, cancel its surfaces/targets before later callbacks can use them. Close interactive panels and destroy that output's bar/dock. The prototype explicitly destroys fractional-scale and viewport children when releasing a surface. Native `wl_surface.destroy` is queued until the calloop after-dispatch drain: SCTK can retain a temporary layer-role reference across `closed`, so destroying the native surface inside that callback would invert role/surface teardown order. Subsequent panels follow the focused output by default. A launcher opened before the current Hyprland discovery answer remains pending without blocking IPC; a current snapshot selects its monitor, while a current failed attempt permits Wayland-output fallback. Closing or disabling cancels pending mapping. Configuration uses output names, which may require adjustment when device names change.

### Tray
`src/tray.rs` owns a minimal StatusNotifierItem host on a worker thread. It
requests `org.kde.StatusNotifierWatcher` with `DoNotQueue`, so when another
host already owns the name the shell logs it, draws nothing and keeps running.
Items register through the watcher interface; the worker reads `Title`,
`IconPixmap` and `IconName` once at registration, drops the item when its bus
name disappears (`NameOwnerChanged`), and sends the list to the UI through a
calloop channel. Nothing is polled. The service exists only while an actual bar consumes an enabled tray; reload/output loss stops the last consumer and releases watcher ownership. Stop cancels pending D-Bus calls; normal method calls have a one-second timeout. Theme-icon search, filesystem reads and PNG decoding remain synchronous on the worker. Because stop joins that worker, slow filesystem IO can delay reload/shutdown on the caller; D-Bus cancellation tests establish no total stop-time bound. Generation tags reject messages from stopped lifecycles. Icon pixels use shared `Arc<[u8]>` ownership through snapshots and render inputs; the combined RGBA strip still copies pixels per draw and
are uploaded only when the strip version changes. Each GLES presenter caches its context's `GL_MAX_TEXTURE_SIZE`; oversized image strips are nearest-centre downsampled independently per axis before upload, while fitting strips borrow their pixels unchanged. Normalized UVs retain icon placement and hit-testing; upload errors propagate without marking the image version current. This bounds GPU dimensions, not the existing bounded CPU strip allocation. `Frame::tray_hits` records
the rects that were drawn, and a left click queues `Activate(0,0)` on the owned
tray worker. Saturation reports an error instead of spawning a thread per click. Context menus, scroll and `NewIcon`/`NewTitle` refresh are out of this
slice. Retained tray items and the registration command queue are capped at 64;
icons are at most 512×512 (1 MiB decoded RGBA), and PNG files at most 4 MiB.
Invalid/oversized icons are ignored, and a full command queue drops new
registrations with a diagnostic. These bounds do not cap zbus's incoming
message allocation or the calloop snapshot queue. Canonical bus/path identity
keeps distinct path-form registrations separate and targets Activate correctly.

## 6. Hyprland adapter

Use the request socket for snapshots/actions and the event socket for invalidation. The event connection is authoritative for the active compositor session. Relevant events coalesce a refresh instead of publishing deltas that could race an in-flight snapshot. Connection epochs and event revisions invalidate snapshots taken before a disconnect or newer event; the UI rechecks them at delivery. Do not assume separate snapshot queries form an atomic compositor transaction. This conservative full-refresh policy replaces incremental application for now; add finer-grained updates only with measured need and ordering evidence.

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

Proposed CLI: `hayu run`, `hayu doctor`, `hayu config check`, `hayu config reload`, `hayu launcher toggle`, `hayu panel toggle control-center`, `hayu clipboard pause`, `hayu clipboard clear`, `hayu quit`. In the M0 slice only `hayu run`, `hayu doctor`, `hayu config check`, `hayu launcher toggle|open|close`, `hayu reload`, and `hayu quit` exist; the rest is still a planned contract. `hayu reload` is the small version of the swap rule above: it rebuilds theme and clock strings, re-applies bar height/edge through a new layer-shell configure, closes the launcher if it was just disabled, and returns the parse error without touching the running state. Local IPC is versioned (v1) and size-bounded (4 KiB, 1 s timeout), with success/error responses and correlation IDs; access is restricted to the socket owner, with directory mode 0700 and socket mode 0600.

## 9. Reliability, privacy, and observability

D-12 and ADR-0008 require consumer-owned start/stop/reconnect lifecycles. A disabled optional module starts no dedicated worker, timer, subscription, connection or resource load. An enabled flag without a surface/widget consumer does not justify background services. Shared services stay alive while other actual consumers exist; disabling the last consumer releases dedicated work and invalidates old-generation results. Current concrete owners are the Hyprland and tray handles, bar-consumer clock timer and optional launcher accessibility adapter; future modules must define equivalent contracts, not inherit a generic framework. Rust memory safety does not remove input/FFI validation or GPU lifecycle requirements.

Structured logs contain module, error class, and duration without clipboard/notification contents, secrets, network credentials, or personal data. `doctor` reports capabilities/dependencies rather than sensitive payload dumps. Rate-limit logs and retries; cache/history caps are in CONFIGURATION. Crash reports and debug traces are not uploaded automatically.
