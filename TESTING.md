# Testing strategy and release gate

Status: plan with a partial M0 suite. `cargo test` runs 38 unit tests covering config validation, Hyprland event parsing, clock formatting, title ellipsization, desktop-entry parsing/dedup/filtering, Exec tokenization, terminal wrapping, control-socket framing and permissions, key mapping, and a rendered launcher frame. The launcher flow was also driven with real key events on the live session (see below). The renderer spike produced offscreen pixel comparisons and live surface checks (ADR-0006). No T-01–T-20 acceptance result exists, and IME, screen-reader, soak, and multi-monitor tests have not been run.

## Testing layers

- Unit: schema/config diffs, state reducers, queue caps, geometry/layout, hit testing, clock, desktop entry rules, app identity, history eviction.
- Contract: socket/event parsers with partial reads, malformed payloads, unknown events, reconnect; D-Bus mocks with owner/property/capability changes.
- Render: shape/text/clip golden scenes with controlled font/scale and antialiasing tolerance; snapshots do not replace interaction tests.
- Integration: nested Hyprland when suitable, and actual hardware for GPU, hotplug, input, backlight, IME, and screen readers.
- Soak/performance: follow [PERFORMANCE.md](PERFORMANCE.md); do not write tests that merely mirror implementation.

## Launcher live input check (November 5, 2026)

Run at commit `10fbb4c` on the target machine (Hyprland 0.57.0, one output, scale 1). Keys were sent as real Wayland input through a throwaway `zwp_virtual_keyboard_v1` client; the shell was observed through `HAYU_DEBUG_KEYS=1` and `HAYU_DEBUG_SCAN=1` traces plus `hyprctl layers -j`, because no screenshot tool is installed. The driver is not in the repository.

| Check | Result |
| --- | --- |
| `launcher toggle` maps the overlay and the second toggle unmaps it | Live pass |
| Typed query `hayu test` reaches the launcher and filters to three entries | Live pass |
| Down changes the selection, Enter launches it and closes the overlay | Live pass |
| Launched command actually ran (file created by `Exec`) | Live pass |
| Reopening after a launch maps the surface again, no protocol error | Live pass |
| Empty results for an unmatched query | Live pass |
| Held Backspace repeats and deletes several characters from one press | Live pass |
| Escape closes; reopening resets the query | Live pass |
| `Terminal=true` entry runs inside foot and closes the overlay | Live pass |
| Failing `Exec` keeps the overlay open and reports the error | Live pass |
| `hayu quit` exits 0 and the socket is removed | Live pass |
| IME, key repeat after a real compositor keymap change, multi-monitor, scale 2 | Not tested |

Unit tests still own the parts the live run cannot isolate: Exec tokenization and field-code rejection, desktop-ID deduplication, filter ordering, terminal resolution, socket framing and permissions, and key mapping.

### Method and limits

- Harness: a throwaway client bound `zwp_virtual_keyboard_v1` on the Hyprland
  seat, uploaded a `us` keymap built with xkbcommon, and sent press/release
  pairs 20 ms apart (one held press for the repeat check). It lives outside the
  repository; no package was installed to run it.
- Observation: `HAYU_DEBUG_KEYS=1` printed keysym, UTF-8, query, selection,
  result count, first names and per-frame draw time; `HAYU_DEBUG_SCAN=1`
  printed the scan duration; `hyprctl layers -j` confirmed whether the overlay
  was mapped. This machine has no screenshot tool, so no pixel diff was taken.
- Isolation: test entries were written under a temporary `XDG_DATA_HOME` and
  launched commands only touched files in `/tmp`, so no user application was
  started by the check.
- Performance numbers come from the same live session: PSS from
  `/proc/<pid>/smaps_rollup`, idle CPU from `utime+stime` in `/proc/<pid>/stat`,
  wakeups from `/proc/<pid>/status`, frame times from the debug trace over 16
  frames including the cold first frame, and the CLI round trip measured around
  `hayu launcher toggle`.
- Limits: one 1366×768 output at scale 1, 46 desktop entries, one session, one
  idle sample; no multi-monitor, fractional scale or IME. The CLI round trip
  ends when the control socket answers, so surface-map latency is not isolated,
  and no GPU metric was collected for the launcher.

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
