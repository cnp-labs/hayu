# Testing strategy and release gate

Status: plan with a partial M0 suite. `cargo test` runs 36 unit tests covering config validation, Hyprland event parsing, clock formatting, title ellipsization, desktop-entry parsing/dedup/filtering, Exec tokenization, control-socket framing and permissions, key mapping, and a rendered launcher frame. The renderer spike produced offscreen pixel comparisons (ADR-0006) and live surface checks, and the launcher was exercised live through the control socket. No T-01–T-20 acceptance result exists, and IME, screen-reader, soak, and multi-monitor tests have not been run.

## Testing layers

- Unit: schema/config diffs, state reducers, queue caps, geometry/layout, hit testing, clock, desktop entry rules, app identity, history eviction.
- Contract: socket/event parsers with partial reads, malformed payloads, unknown events, reconnect; D-Bus mocks with owner/property/capability changes.
- Render: shape/text/clip golden scenes with controlled font/scale and antialiasing tolerance; snapshots do not replace interaction tests.
- Integration: nested Hyprland when suitable, and actual hardware for GPU, hotplug, input, backlight, IME, and screen readers.
- Soak/performance: follow [PERFORMANCE.md](PERFORMANCE.md); do not write tests that merely mirror implementation.

## Acceptance matrix

The PRD column refers to HAYU-xxx in [PRD.md](PRD.md).

| Test ID | PRD | Scenario | Minimum result |
| --- | --- | --- | --- |
| T-01 | 001,015 | Narrow bar, long title, minute/second clock, theme/scale | No overflow; correct tick precision; redraw only as needed |
| T-02 | 002,016 | Workspace/monitor focus, special workspace, event disconnect | Correct target, recovered resync, responsive UI |
| T-03 | 003 | Unicode, Hidden/NoDisplay, duplicate desktop IDs, Exec codes, D-Bus activation | Correct visibility/arguments; visible launch errors |
| T-04 | 003,015 | IME composition, selection, graphemes, RTL, keyboard | Correct text/focus; no UTF-8 corruption |
| T-05 | 004 | Tray/menu restart, missing icon, DBusMenu error, watcher conflict | No duplicates/crashes; isolated failures |
| T-06 | 005 | Replace/close/action, timeout/urgency, DND, payload/history caps | Correct protocol lifecycle; bounded resources |
| T-07 | 006–010 | External volume/backlight changes, audio output loss, service restart | Actual values, reconnect, correct pending/error and OSD |
| T-08 | 007–009 | No battery/backlight/NetworkManager, permission denied | Clear unknown/unavailable; other features remain active |
| T-09 | 008 | Saved connection, radio off, failed connection, new credentials | Correct state; external editor; no logged secrets |
| T-10 | 010 | Multiple players, missing capabilities, player exit | Correct selection/fallback and disabled actions |
| T-11 | 011 | Corrupt/huge images, empty directory, scale/hotplug | Bounded decoding, fallback, workers do not block UI |
| T-12 | 012 | Multiple windows, app_id mismatch, removed pinned app, restart | Correct chooser/fallback and state |
| T-13 | 013 | Opt-in/off/pause/clear, duplicates, caps, selection owner changes | Capture follows choice; FD transfers finish; no disk/log payload |
| T-14 | 014 | Typos/invalid TOML, newer schema, atomic rename, 100 reloads | Last-known-good; no duplicate listeners; correct state precedence |
| T-15 | 014,016 | Two instances, socket permissions, malformed client, timeout/quit | Session isolation, bounded errors, cleaned resources |
| T-16 | 015 | Scales 1/1.25/1.5/2, mixed scale, unplug output with popup | Correct geometry/hit targets; closed popup; no crash |
| T-17 | 015 | Tab/Shift+Tab, focus ring, screen reader roles/names/actions | Usable controls; semantics match visuals |
| T-18 | 016 | Disable every consumer, then enable again | Dedicated resources released; shared services follow consumer count |
| T-19 | 013,016 | External Hyprlock, pause hook if used, suspend/resume | Lock unaffected; clear privacy limits; reconnect |
| T-20 | All | All features active, clean package/install/uninstall, long session | Gates pass; dotfiles unchanged; resources reported |

Each requirement needs relevant tests or manual evidence; this matrix is not proof of passing. Fixtures use synthetic data, not personal clipboard/notification contents.

## Robustness

Fuzz config/event/IPC parsers and image metadata where paths are complex. Check size limits before large allocations. Test FD ownership, EAGAIN, cancellation, and clipboard error paths. Activate applications through desktop entry semantics, without `sh -c` constructed from queries/IPC. Test FFI/GPU resources with relevant tools.

## CI, hardware, and soak

Once source exists: formatting/linting, unit/contract tests, and release builds use a pinned toolchain. Render/headless checks may be added, but real Hyprland hardware, IME, and screen reader tests remain separately recorded. Test PKGBUILD in a clean chroot. Final CI commands follow actual Cargo/dependency setup.

Candidate soak: 8 hours of all-module use, 100 reload/panel cycles, each backend reconnecting, suspend/resume, and hotplug. Compare quiescent resources with a warm baseline. This plan has not been executed.

## Release gate and evidence

Every HAYU-001–016 requirement needs commit/version/hardware evidence; no crash, data-loss, input/focus, or privacy blockers remain; screen reader/IME/scaling/multi-monitor testing passes; Core/V1 normal/stress benchmarks are available; packages and license inventories match implementation.

Evidence template: test ID, date, commit, environment, steps, expected/actual, pass/fail, sanitized attachment, issue. All results at document preparation time: **not run**.
