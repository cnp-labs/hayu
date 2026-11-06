# Testing strategy and release gate

Status: plan with a partial M0 suite. `cargo test` runs 43 unit tests covering config validation, Hyprland event parsing, clock formatting, title ellipsization, desktop-entry parsing/dedup/filtering, Exec tokenization, terminal wrapping, control-socket framing and permissions, key mapping, and a rendered launcher frame. The launcher flow was also driven with real key events on the live session (see below). The renderer spike produced offscreen pixel comparisons and live surface checks (ADR-0006). No T-01–T-20 acceptance result exists. Launcher input, IME, scale 2, two outputs and config reload were exercised live (see below); the launcher and reload harnesses are checked in under `tools/live/`, and the IME run used fcitx5 plus fcitx5-anthy. Screen-reader, soak, fractional scale and mixed per-output scales have not been run.

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
| IME | Live pass with one exception, see the fcitx5 section below |

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
  idle sample and no fractional scale; IME is covered by its own section. The
  CLI round trip ends when the control socket answers, so surface-map latency
  is not isolated, and no GPU metric was collected for the launcher.

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
| Mixed per-output scales, fractional scale | Not tested (IME has its own section) |

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

### Idle redraw check (November 6, 2026)
Two runs on the same session, one 32px top bar, `%H:%M` clock, no input. Each
run used two independent logs: `WAYLAND_DEBUG=1` counted `wl_surface.commit`
calls, and a separate client read the Hyprland event socket with a timestamp per
line.

Run 1 at `72ffbcc` covered 40 seconds from 14:19:07. The bar committed 10 times;
every commit sat in the same burst as a Hyprland event (`windowtitle`/
`activewindow` at 0.0 s, 12.44 s, 18.21 s and 30.92 s, plus the startup draws
before the window). No commit happened in any gap between events. That window
ended before the next minute boundary, so it did not cover the tick.

Run 2 at `19b6f8b` covered 80 seconds, 14:50:49 to 14:52:09, and crossed two
minute boundaries. The bar committed 19 times: 17 in the startup burst at
14:50:46-49 (each within ~1 ms of `openlayer`, `activewindow`, `workspace`,
`createworkspace` or `destroyworkspace`), and 2 at 14:51:00.021 and
14:52:00.022. Those two are 21 ms and 22 ms after the minute changed, and no
Hyprland event was logged between 14:50:49.06 and 14:52:09.6, so they can only
be the clock tick.

| Check | Result |
| --- | --- |
| Commits outside a Hyprland event burst (run 2) | only the two minute ticks |
| Exactly one draw per minute boundary | Live pass: commits at :00.021 and :00.022, 60.0 s apart, nothing between |
| Idle CPU over the 80 s window | 0 ticks in `/proc/<pid>/stat`, under 0.02% of one logical core; 6 voluntary context switches |
| PSS while idle and closed | 7.0 MiB |
| Redraw after a title or workspace change | one commit ~1 ms after the event burst |

- Limits: one output, integral scale, a busy session; the runs prove there was
  no periodic redraw and that the tick fires once per minute, not that every
  future idle period stays silent. Only the bar was measured; the bar and
  launcher were closed for the CPU and PSS samples.
### Text input with fcitx5 and anthy (November 6, 2026)
Run at `4de4112` on the same session (Hyprland 0.57.0, one 1366×768 output at
scale 1) with `fcitx5 5.1.23` and `fcitx5-anthy 5.1.11`. Anthy was picked
because it composes visible Japanese from Latin keys, so a wrong commit is
obvious without a candidate UI. The input method ran with `fcitx5 --replace -d`
against a temporary `XDG_CONFIG_HOME` holding both `hayu/config.toml` and an
`fcitx5/profile` with one anthy group, and `fcitx5-remote -o` activated it;
`fcitx5-remote -n` reported `anthy`. Keys came from `tools/live/keyin` and the
launcher was driven through the control socket. `HAYU_DEBUG_IME=1` and
`WAYLAND_DEBUG=1` recorded event kinds and lengths, and `HAYU_DEBUG_KEYS=1`
printed the query only for the checks that need it.

| ID | Criterion | Result |
| --- | --- | --- |
| I1 | Bound once on the launcher seat | Pass: one `zwp_text_input_manager_v3` bind and one `get_text_input` for the seat; no text-input object is created for a bar |
| I2 | Inline preedit, no early filtering | Pass: the preedit went `n` → `に` → `にh` → `にほ` → `にほn` while `query_len` stayed 0 |
| I3 | Commit inserts once and launches | Pass: Space turned the preedit into `日本`, Return sent exactly one `commit_string("日本")`, `query_len` became 6 and surrounding text `日本` 6/6; the next Return launched the test entry and closed the overlay |
| I4 | Cancel leaves the launcher usable | **Exception**: the first Escape cleared the preedit (`key -> Changed`, `preedit=0`) and left the overlay mapped, but the second Escape never reached the client — it produced no `wl_keyboard.key` at all — and only the third closed it. With no composition, one Escape still closes normally |
| I5 | Editing stays inside the protocol | Pass: BackSpace during the preedit took `にほn` to `にほ`; `query_len` stayed 0 |
| I6 | Surrounding text stays correct | Pass: after the commit the client sent `日本` 6/6; BackSpace then removed one character and sent `日` 3/3, leaving one matching entry |
| I7 | Close and reopen reset the field | Pass: closing with a preedit logged `ime disabled`; reopening logged `ime enabled` and `set_surrounding_text("", 0, 0)`, and the next preedit started from `f`, not from the old text |
| I8 | Missing IME falls back cleanly | Pass: with fcitx5 stopped, `tools/live/input-suite.sh` passed 15/15 and reported no protocol error |
| I9 | Teardown is clean | Pass: `hayu quit` with a preedit exited 0 and sent `disable` + `commit` before destroying the layers; reloading `launcher.enabled=false` exited 0, closed the overlay and logged `ime disabled`; removing the focused headless output with a preedit left the shell alive, removed the launcher layer and logged `ime disabled` |
| I10 | Text is not logged by default | Pass: with `HAYU_DEBUG_IME=1` and no `HAYU_DEBUG_KEYS=1`, the trace held only counts and lengths, and a search for `にほ`, `日本` and `nihon` in the log found nothing |

- No double insert: while the input method composed, no `wl_keyboard.key`
  reached the surface, so the keysym path never inserted the same letters a
  second time; the committed text appeared once in the query.
- The I4 exception comes from fcitx5-anthy keeping its own composition after
  the client drops the preedit; the protocol has no cancel request, so the
  client cannot clear it. It is recorded as an open interaction, not as
  working behaviour, and the rest of the IME run does not close it.

  Reproduce it with the same setup as this section:

  1. Start the input method (`fcitx5 --replace -d` against the temporary
     `XDG_CONFIG_HOME`), then run `hayu launcher open`.
  2. Activate anthy: `fcitx5-remote -o`; `fcitx5-remote` must answer `2` and
     `fcitx5-remote -n` must answer `anthy`.
  3. Type `nihon` with `tools/live/keyin`; the preedit appears and the result
     list stays unchanged.
  4. Press Escape once: the preedit disappears and `hyprctl layers -j` still
     lists `hayu-launcher`.
  5. Press Escape again: with `WAYLAND_DEBUG=1`, no `wl_keyboard.key` arrives
     and the overlay stays mapped.
  6. Press Escape a third time: the overlay closes.
- Limits: no screenshot tool, so preedit rendering and the input method's own
  candidate popup were not inspected pixel by pixel; I1-I8 used the real output
  only; I9's output removal used a headless output, not a cable; preedit,
  commit and delete were the only text-input events exercised.

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
