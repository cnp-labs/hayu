# Testing strategy and release gate

Status: plan with a partial M0 suite. `cargo test` runs 41 unit tests covering config validation, Hyprland event parsing, clock formatting, title ellipsization, desktop-entry parsing/dedup/filtering, Exec tokenization, terminal wrapping, control-socket framing and permissions, key mapping, and a rendered launcher frame. The launcher flow was also driven with real key events on the live session (see below). The renderer spike produced offscreen pixel comparisons and live surface checks (ADR-0006). No T-01–T-20 acceptance result exists. Launcher input, scale 2, two outputs and config reload were exercised live (see below); the launcher and reload harnesses are checked in under `tools/live/`; IME, screen-reader, soak, fractional scale and mixed per-output scales have not been run.

## Testing layers

- Unit: schema/config diffs, state reducers, queue caps, geometry/layout, hit testing, clock, desktop entry rules, app identity, history eviction.
- Contract: socket/event parsers with partial reads, malformed payloads, unknown events, reconnect; D-Bus mocks with owner/property/capability changes.
- Render: shape/text/clip golden scenes with controlled font/scale and antialiasing tolerance; snapshots do not replace interaction tests.
- Integration: nested Hyprland when suitable, and actual hardware for GPU, hotplug, input, backlight, IME, and screen readers.
- Soak/performance: follow [PERFORMANCE.md](PERFORMANCE.md); do not write tests that merely mirror implementation.

## Launcher live input check (November 5, 2026)

Run at commit `10fbb4c` on the target machine (Hyprland 0.57.0, one output, scale 1). Keys were sent as real Wayland input through a throwaway `zwp_virtual_keyboard_v1` client; the shell was observed through `HAYU_DEBUG_KEYS=1` and `HAYU_DEBUG_SCAN=1` traces plus `hyprctl layers -j`, because no screenshot tool is installed. That run used a throwaway build of the
same approach; the harness now lives in `tools/live/` and the suite was re-run
at `d3781f7` (15 checks, all pass).

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
| IME | Not tested |

Unit tests still own the parts the live run cannot isolate: Exec tokenization and field-code rejection, desktop-ID deduplication, filter ordering, terminal resolution, socket framing and permissions, and key mapping.

### Method and limits

- Harness: `tools/live/keyin`, a small client that binds
  `zwp_virtual_keyboard_v1` on the Hyprland seat, uploads an xkb keymap and
  sends press/release pairs 20 ms apart (one held press for the repeat check).
  `tools/live/input-suite.sh` drives it and reads the trace. Both are in the
  repository now; see `tools/live/README.md` for the commands and its limits.
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
  idle sample; no fractional scale or IME. The CLI round trip ends when the
  control socket answers, so surface-map latency is not isolated, and no GPU
  metric was collected for the launcher.

### Scale 2, two outputs and a runtime keymap switch (November 5, 2026)

Run at commit `9e0a4d3` with the same harness and observation setup. This
Hyprland uses the Lua config provider, where `hyprctl keyword` answers
`unknown request`, so both outputs were configured with
`hyprctl eval 'hl.monitor({ ... })'`; `HEADLESS-1` (1920×1080) was created with
`hyprctl output create headless` and focused with
`hl.dispatch(hl.dsp.focus({monitor = "HEADLESS-1"}))`.

| Check | Result |
| --- | --- |
| Full 20-check launcher suite on one output at scale 2 | Live pass |
| One bar per output at its own logical size (683×32 and 960×32) | Live pass |
| Launcher opens on the focused output (`HEADLESS-1`) at 560×420 logical | Live pass |
| Typing, selection and launch on that output | Live pass |
| Removing `HEADLESS-1` while the launcher is open: shell survives, layer disappears | Live pass |
| Next open lands on the remaining output and accepts keys | Live pass |
| Keymap `us`→`de`: evdev keycode 44 yields `z` before and `y` after, no restart | Live pass |
| Mixed per-output scales, fractional scale, IME | Not tested |

- Scale: the launcher read the new scale from the output info and drew a
  1120×840 buffer for its 560×420 logical surface; the exact pixels are covered
  by the offscreen scale-2 render test, because no screenshot tool is installed
  for a live diff.
- Unplug: a headless output was removed rather than a physical cable, so the
  `wl_output` removal path was exercised but a real hotplug was not.
- Keymap: the change came from a second `zwp_virtual_keyboard_v1` client
  uploading a `de` keymap, which makes the compositor send a fresh
  `wl_keyboard.keymap` to the focused surface. The harness then sent raw evdev
  keycode 44 instead of a keysym, so the character can only come from the
  client's current keymap. A `switchxkblayout` on a physical device was not
  used.
- After the run, `HDMI-A-1` was returned to scale 1 and the full suite passed
  again.

### Config reload (November 6, 2026)
Run at commit `310a7c7` on the same session (Hyprland 0.57.0, one 1366×768
output at scale 1). A real bar was started from a temporary `XDG_CONFIG_HOME`,
the config file was rewritten between steps, and each `hayu reload` went through
the control socket. Geometry came from `hyprctl layers -j`; nothing here was
inferred from the config file alone.

| Check | Result |
| --- | --- |
| Initial 32px top bar maps at y=0 | Live pass |
| Reload to 40px bottom exits 0 and the bar maps at y=728, h=40 | Live pass |
| Launcher still opens and closes after a geometry reload | Live pass |
| Invalid file (`bar.height = 500`) exits 1 and names `bar.height` | Live pass |
| After the rejected reload the bar is still 40px bottom and the launcher works | Live pass |
| `launcher.enabled=false` makes `launcher toggle` and `launcher open` exit 1 | Live pass |
| Reloading `enabled=false` while the overlay is open closes it | Live pass |
| Re-enabling maps the launcher again; `quit` exits 0 | Live pass |
| An applied-only reload prints no notice | Live pass |
| `bar.outputs = ["NOPE-1"]` applies without a restart, prints no notice, and removes the bar | Live pass |
| The launcher still opens with no bar, because it falls back to the output list | Live pass |
| `bar.enabled=false` applies without a restart and removes the bar | Live pass |
| The launcher still opens with the bar disabled | Live pass |
| Re-enabling the bar maps it again at the configured height | Live pass |
| A named filter moves the bar onto a matching hotplugged headless output | Live pass |
| Removing that output removes its bar; restoring `*` maps the bar on the real output | Live pass |
| A widget-list change still exits 0 and prints `restart required for: bar.left` | Live pass |
| Theme/accent and clock-format change | Applied, no pixel diff |

- Method: `tools/live/reload-suite.sh` (41 checks at `310a7c7`) writes small
  config files with `printf`, calls `hayu reload` between steps, and reads
  `h`/`y` of the `hayu` layer plus the `hayu-launcher` layer count. It starts
  its own shell against a temporary `XDG_CONFIG_HOME`, creates a headless
  output for the filter check through `hyprctl`, and removes every headless
  output it made on exit.
- Limits: one real output and integral scale; the filter check uses a
  temporary headless output, not a second monitor. No screenshot tool, so a
  theme or clock change is only known to have been applied to the state, not
  confirmed on screen. Widget lists still need a restart.
- Unit tests cannot reach this path without a Wayland session; the IPC parser
  change (`reload`) is pinned by `parses_and_serializes_requests`.

### Terminal fallback arguments (November 5, 2026)

The fallback list is not one invocation: each emulator has its own way to run a
program. Verified at the current commit by pointing the shell's `PATH` at a
directory holding one recording stub named after each emulator, launching the
`Terminal=true` test entry through the real launcher, and reading the recorded
argv. This exercises selection and spawn; it does not prove that a real binary
of that emulator accepts the form.

| Emulator | Invocation | Evidence |
| --- | --- | --- |
| foot | `foot <program> <args>` (`-e` is documented as ignored) | Live with the real foot binary; stub argv matches |
| alacritty | `alacritty -e <program> <args>` | Stub argv live; `-e, --command` from `alacritty(1)` |
| kitty | `kitty -- <program> <args>` | Stub argv live; `--` terminator in kitty's CLI parser |
| wezterm | `wezterm start -- <program> <args>` | Stub argv live; `start` from `wezterm start --help` |
| ghostty | `ghostty -e <program> <args>` | Stub argv live; `-e` special case in `src/cli/ghostty.zig` |
| konsole | `konsole -e <program> <args>` | Stub argv live; `-e` in the Konsole manual |
| xterm | `xterm -e <program> <args>` | Stub argv live; `-e program [arguments]` in `xterm(1)` |
| `$TERMINAL` | a known emulator keeps its row's form, anything else gets `-e` | Stub argv live for `kitty` by name and by absolute path (both `--`); every known name is unit-tested |

- Only foot is installed on this machine. The other six forms were checked
  against their upstream documentation plus the recorded spawn: no real window
  was opened for them, and a future upstream change could invalidate a form.
- The unit test `each_fallback_uses_its_own_program_arguments` pins the same
  table without spawning.

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
