# ADR-0008: Consumer-owned services and asynchronous Hyprland requests

Date: November 7, 2026; verification updated November 8
Status: prototype implementation verified; product ownership division user-approved (D-12). Not full V1 acceptance.
References: HAYU-001, HAYU-003, HAYU-004, HAYU-014, HAYU-015; [REFOCUS.md](../REFOCUS.md)

## Context

The prototype used a detached Hyprland event listener but executed initial snapshots, resyncs and workspace actions synchronously from application callbacks. A slow request could block startup, input or drawing. Tray worker lifetime followed an enabled flag rather than an actual bar consumer and could not be stopped on reload. Disabled launcher still initialized its accessibility adapter. The approved scope distinguishes mandatory foundations from optional module resource ownership, while retaining every V1 category.

## Decision

Keep Rust, calloop channels, Hyprland adapter ownership and GLES/EGL. Use two owned Hyprland workers: an event reader invalidates state and requests coalesced refresh; a request worker alone publishes snapshots and action errors. UI callbacks submit work, never wait for a socket reply. Snapshot results carry service generation, connection epoch and event revision; stale results are rejected both before publication and on delivery. Discovery failures are reported only after an actual attempt, allowing a pending launcher to use a Wayland-output fallback without racing initial monitor discovery.

Workspace actions use a FIFO capped at 16 pending entries; overflow/stopped service is explicit. Refresh is a coalesced bit, alternated with actions to avoid starvation. The result channel holds 64 entries plus one retained pending result; draining wakes the publisher. Stop cancels registered sockets, wakes backoff/output waits and joins workers. Request exchanges have an absolute two-second deadline and a 4 MiB response cap. Connection retry slices exist only during bounded connection setup, not steady idle polling. Reconnect requires a fresh snapshot. Legacy workspace dispatch precedes Lua fallback; transport failure does not trigger a potentially duplicate dispatch.

Tray service runs only while an actual bar consumes an enabled tray. Its command queue holds 64 entries; D-Bus calls have a one-second timeout and stop cancels pending calls before joining. Watcher ownership remains non-replacing. Icon pixels share `Arc<[u8]>` across snapshots/render inputs; the combined upload strip still copies pixels. Item/icon/upload limits remain. Clock scheduling requires an actual bar with a clock widget; disabled launcher owns no accessibility adapter, catalogue or surface. Surface release destroys fractional/viewport children and queues native surface destruction until after dispatch so SCTK's temporary layer-role references cannot invert teardown order.

No generic service registry, plugin framework or async runtime is added. `rustix` 1.1, already present transitively, is a direct dependency with `event` and `net` features for safe nonblocking sockets and deadline-aware polling. Full AF_UNIX backlogs require cancellable connect retries rather than treating writability and `SO_ERROR == 0` as proof of connection.

## Consequences and verification limits

A fixed number of workers trades memory for responsive dispatch. Snapshots are not atomic compositor transactions. No V1 acceptance follows from this implementation.

November 8 validation passed 90 default tests and 93 all-feature tests, including delayed sockets, bounded queues, reconnect/stale results, total deadlines, shutdown and private-bus tray lifecycle. Isolated input/tray suites, pointer rerun, accessible-tree rerun and 71 reload checks passed. Hotplug exposed the teardown-order defect fixed above. AT-SPI required user repair of a stale bus; Orca was absent, so speech output remains unverified. Strict Clippy remains failed on 29 pre-existing diagnostics. Fresh IME/fractional-scale evidence and full V1 profile measurements are not established by these results. See [TESTING.md](../TESTING.md) and [PERFORMANCE.md](../PERFORMANCE.md); historical evidence remains scoped to its recorded revisions.
