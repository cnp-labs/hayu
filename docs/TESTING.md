# Testing strategy and release gate

Status: partial prototype suite, not V1 acceptance. November 7 refocus validation passes 61 default unit tests and 64 all-feature tests; see [REFOCUS_VALIDATION.md](REFOCUS_VALIDATION.md) for exact commands, warnings and current live coverage. Historical launcher, IME, scaling, output and reload evidence below remains scoped to its recorded revisions/environments; it is not automatically rerun coverage. Harnesses live under `../tools/live/`; renderer evidence remains in ADR-0006. No T-01–T-20 acceptance result or comprehensive screen-reader usability claim exists.

November 7 final live checks: input 15/15, reload 42/42, pointer 7/7 and tray 11/11 pass. A11y suite exits 134 because `/run/user/1000/at-spi/bus_0` is absent; cleanup restored its owned process and original AT setting. This is not an accessibility pass. See [REFOCUS_VALIDATION.md](REFOCUS_VALIDATION.md) and [REFOCUS_MEASUREMENTS.md](REFOCUS_MEASUREMENTS.md).

## November 8 consumer-owned lifecycle checks

Follow-up to ADR-0008; not V1 acceptance. `timeout 120 cargo test --offline hyprland::` passed 20 tests, including delayed-response calloop responsiveness, executed FIFO saturation, retained bounded output, connect-backlog deadlines/cancellation, slow-drip total deadlines, idle event connection stability and stale in-flight reconnect rejection. `timeout 180 cargo test --locked --lib -- tray::tests app::tests config::tests launcher::tests` passed 36 tests, including private-bus tray name release/restart, cancellation during pending property/activation calls, shared pixel storage, consumer predicates and launcher pending-map success/error/close rules. These focused counts overlap the full suite; do not add them together as a distinct test total.

Live harnesses now use private short runtime directories and PID-filtered layer observations, preserve the user's control socket, and restore deliberate focus/workspace changes. Run reload checks with `dbus-run-session -- tools/live/reload-suite.sh` so tray worker assertions exercise actual ownership rather than a competing watcher. Static syntax checks are not live passes. November 8 full validation passed 90 default tests and 93 all-feature tests, the default release build, touched-file formatting, key-injector build, `doctor` and example config validation. Strict all-target/all-feature Clippy exited 101: 29 distinct diagnostics point to unchanged code already present at `8688c8e`; no new diagnostic was identified on added lines. This is still a failed strict Clippy gate, not a clean lint result. Resumed isolated input and tray suites exited 0. Reload initially exited 1 after an output-removal protocol error (`wl_display: invalid object`). Deferring native surface destruction until after layer-role dispatch and explicitly releasing fractional/viewport children fixed teardown; the final private-bus rerun passed all 71 reload checks, including output removal/recreation, repeated launcher disable and exact tray worker counts. Pointer initially exited 1 because successful Lua dispatch returned a table rather than `ok`; state-based verification fixed the harness and its rerun exited 0. After the user manually restarted a stale AT-SPI service, the AT-SPI harness passed (exit 0) with private-runtime verification, query text, selection, close/reopen and exact `IsEnabled` restoration. Its initial process-environment readiness race was fixed before this pass. Orca is absent: this is accessible-tree evidence, not speech output or screen-reader usability acceptance. The user shell PID and control-socket inode were unchanged after these runs.

### Tray shutdown coverage limit

The private-bus cancellation tests cover pending D-Bus property/activation calls, not synchronous theme-icon lookup, filesystem reads or PNG decoding in `read_item`. `Service::stop()` joins that worker; a blocked filesystem operation can therefore hold up the UI caller during reload/shutdown. File-size, dimension and traversal caps bound accepted data/work but do not bound filesystem latency. No slow-filesystem shutdown test or end-to-end stop deadline is established. Follow-up must distinguish cooperative cancellation between operations from interruption of an in-flight blocking syscall; detaching a worker would not meet consumer-owned teardown semantics.

## Tray texture hardware bound (November 8, 2026)

The GLES presenter queries `GL_MAX_TEXTURE_SIZE` once with its context current. Fitting strips borrow source bytes; oversized strips are nearest-centre downsampled independently per axis before upload. Normalized UVs preserve icon domains, draw geometry and hit-testing. The CPU strip still retains source dimensions; this fix bounds GPU uploads, not CPU allocation. Six focused tests cover exact-limit borrowing, independent axis limits, all 64 cells from a 32,768×512 source at simulated limits 2,048/4,096, malformed inputs, zero limit and version retention on upload failure. These are simulated-limit tests, not a low-limit hardware run.

Validation: `cargo test --locked` passed 96 tests; `cargo test --all-features --locked` passed 99. Default release and `gpu-spike` release builds, touched-file formatting and private-bus live tray checks (13/13) passed. `cargo clippy --all-targets --all-features --locked` exited 0 with existing warnings; this was not a strict `-D warnings` pass. No renderer-GL warning appeared. Live tray used ordinary fcitx5 icons, not the synthetic maximum-size fixture.

## Testing layers

- Unit: schema/config diffs, state reducers, queue caps, geometry/layout, hit testing, clock, desktop entry rules, app identity, history eviction.
- Contract: socket/event parsers with partial reads, malformed payloads, unknown events, reconnect; D-Bus mocks with owner/property/capability changes.
- Render: shape/text/clip golden scenes with controlled font/scale and antialiasing tolerance; snapshots do not replace interaction tests.
- Integration: nested Hyprland when suitable, and actual hardware for GPU, hotplug, input, backlight, IME, and screen readers.
- Soak/performance: follow [PERFORMANCE.md](PERFORMANCE.md); do not write tests that merely mirror implementation.

## Launcher live input check (November 5, 2026)

Run at commit `10fbb4c` on the target machine (Hyprland 0.57.0, one output, scale 1). Keys were sent as real Wayland input through a throwaway `zwp_virtual_keyboard_v1` client; the shell was observed through `HAYU_DEBUG_KEYS=1` and `HAYU_DEBUG_SCAN=1` traces plus `hyprctl layers -j`, because no screenshot tool is installed. That run used a throwaway build of the
same approach; the harness now lives in `../tools/live/` and the suite was re-run
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

- Harness: `../tools/live/keyin`, a small client that binds
  `zwp_virtual_keyboard_v1` on the Hyprland seat, uploads an xkb keymap and
  sends press/release pairs 20 ms apart (one held press for the repeat check).
  `../tools/live/input-suite.sh` drives it and reads the trace. Both are in the
  repository now; see `../tools/live/README.md` for the commands and its limits.
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
| A widget-list change still exits 0 and prints `restart required for: bar.left` | Superseded by *Bar module areas* (November 7, 2026) |
| Theme/accent and clock-format change | Applied, no pixel diff |

- Method: `../tools/live/reload-suite.sh` (41 checks at `310a7c7`) writes small
  config files with `printf`, calls `hayu reload` between steps, and reads
  `h`/`y` of the `hayu` layer plus the `hayu-launcher` layer count. It starts
  its own shell against a temporary `XDG_CONFIG_HOME`, creates a headless
  output for the filter check through `hyprctl`, and removes every headless
  output it made on exit.
- Limits: one real output and integral scale; the filter check uses a
  temporary headless output, not a second monitor. No screenshot tool, so a
  theme or clock change is only known to have been applied to the state, not
  confirmed on screen. Widget lists needed a restart at this commit; they are
  read at draw time since November 7, 2026.
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
`fcitx5-remote -n` reported `anthy`. Keys came from `../tools/live/keyin` and the
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
| I8 | Missing IME falls back cleanly | Pass: with fcitx5 stopped, `../tools/live/input-suite.sh` passed 15/15 and reported no protocol error |
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
  3. Type `nihon` with `../tools/live/keyin`; the preedit appears and the result
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

### Launcher AT-SPI spike (November 6, 2026)

Run at `c8017ee` on the same session (Hyprland 0.57.0, one 1366×768 output at
scale 1) with `accesskit 0.25.2`, `accesskit_unix 0.24.1` and `at-spi2-core`.
Orca and accerciser were not installed, so this records the AT-SPI tree, not
speech. The session had `org.a11y.Status IsEnabled=false` and no assistive
client; `../tools/live/a11y-suite.py` sets it true for the run and restores the
original value. Hayu ran from a temporary `XDG_CONFIG_HOME`, `XDG_DATA_HOME`
and `XDG_RUNTIME_DIR` with three synthetic entries (`A11y Birch`, `A11y Cedar`,
`A11y Maple`), so the user's shell and desktop entries were untouched. The
runtime directory has to stay short: the control socket path must fit
`SUN_LEN`.

```
gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
  --method org.freedesktop.DBus.Properties.Set org.a11y.Status IsEnabled '<true>'
/usr/bin/python3 tools/live/a11y-suite.py    # exits 0; log in /tmp/opencode
```

| ID | Criterion | Result |
| --- | --- | --- |
| A1 | Launcher detected by AT-SPI | Pass: application `hayu`, window `Hayu launcher` with role `frame` while the overlay is open |
| A2 | Search field has a name, role and value | Pass: one node with role `entry` and name `Search applications`; `Text.GetText` returned `""` before typing, `A11y` after injecting `type:A11y`, and `A11y B` after appending `type: B` |
| A3 | Results readable | Pass: one `list` named `Results`; one `list item` per filtered entry, named after the desktop entry; the three fixtures matched `A11y`, and appending ` B` left exactly `A11y Birch` |
| A4 | Active selection readable | Pass: the initial row carried `selected=true`; `Down` moved it from `A11y Birch` to `A11y Cedar` |
| A5 | Close and reopen stay consistent | Pass: after close the window node remained but `showing=false` with no `entry` or `list` children; reopening restored all three entries with an empty query value and the first row selected |
| A6 | The check does not disturb the session | Pass: only the shell started by the suite was quit, `IsEnabled` was restored to `false`, and the temporary runtime and config directories were removed |

- The check drives plain `wl_keyboard` input, which needs no bound input
  method; it does not exercise the text-input path, and the IME I4 exception
  above stays open.
- Limits: no screen reader was installed, so announcement order, focus events,
  braille output and interruption behaviour were not tested, and T-17 remains
  unrun. The tree is read-only: the action handler ignores requests, so
  activating a result from a screen reader does nothing. Caret and selection
  ranges are absent, IME preedit is not exposed at all, and there are no
  bounds/geometry nodes. Only the launcher is covered; the bar has no semantic
  tree. A closed launcher is a hidden frame node because AccessKit has no API
  to remove a tree root. All filtered results are published, with no row cap
  or scroll position. One output at scale 1 was used, and `IsEnabled` was set
  by the check rather than by a client.

### GLES/EGL render path (November 6, 2026)
Run at `f105f22` on the same session (Hyprland 0.57.0, one 1366×768 output at
scale 1, RADV OLAND). The bar and the launcher now build a `render::Frame` and
present it through one EGL context and window surface per layer surface; the
tiny-skia path survives only as `render::rasterize`, which the unit tests call.
There is no shm buffer and no `wl_shm` bind on the shell path any more.

```
cargo test                                   # 46 tests
tools/live/input-suite.sh                    # 15 checks
tools/live/reload-suite.sh                   # 41 checks
/usr/bin/python3 tools/live/a11y-suite.py    # exits 0
```

| Check | Result |
| --- | --- |
| Bar presents through EGL | Pass: the bar surface is created, then painted by `swap_buffers`; startup, workspace/title events and the minute tick all reached it |
| Launcher presents through EGL | Pass: all 15 input checks pass, including type, repeat, arrows, Enter, Escape, `Terminal=true` and the failing-Exec error row |
| Reload keeps the renderer consistent | Pass: all 41 checks, including 32px top bar → 40px bottom bar at y=728, `launcher.enabled=false`, and `bar.outputs` reconciliation |
| Accessibility is unaffected | Pass: the AT-SPI suite exits 0 with the same names, roles, query value, selection and hidden root as before |
| Glyph reuse | Pass: `frame_reuses_glyphs_from_the_atlas` shows a second identical frame adds no atlas change |
| Reference pixels | Pass: the launcher test rasterizes the same frame at scale 2 and finds the accent colour |
| Protocol health | Pass: no protocol error in either live log; no `hayu run` left behind |

- Limits of that run: no screenshot tool was used, so the check rested on the
  shared frame data, the rasterizer test, and the live suites. The readback
  section below inspects the renderer's own output; it does not cover the
  composited desktop.
- `cargo test` covers the frame/atlas code with the system font database; on a
  machine without fonts the text assertions are skipped the same way the old
  render tests were.
### Visual and idle check on the GL renderer (November 7, 2026)
Run at `729ba82` on Hyprland 0.57.1 (session restarted since the last entry),
with two active outputs at once: a 1366×768 output at scale 1 and a temporary
`HEADLESS-1` at 1920×1080 scale 2. No screencopy client is installed, and a
non-interactive `org.freedesktop.portal.Screenshot` request was refused with
response code `2`, so the composited screen was never captured. Instead a
temporary, uncommitted patch read the EGL back buffer with `glReadPixels`
before `eglSwapBuffers` and wrote one PPM per render attempt; it was removed
after the run. `magick` converted those files to PNG for inspection. This is
the renderer's output before the swap, not a screenshot of the desktop. PPM has
no alpha channel, so alpha in a later RGBA conversion is synthetic.
```
HAYU_DEBUG_DUMP=/tmp/.../frame ./target/release/hayu run   # temporary patch
```
| Check | Observed |
| --- | --- |
| Bar content, scale 1 (1366×32) | Workspace pills `1` (accent fill `#8AB4F8`, dark label) and `2` (foreground label), clock `14:44` centred, muted title `hayu — ARCHITECTURE.md`, one pixel divider absent as designed |
| Colours | Sampled RGB exactly `(30,30,46)` background, `(138,180,248)` accent, `(205,214,244)` foreground, `(108,112,134)` muted; 875 pixels in the bar and 9813 in the launcher differ from those exact values, consistent with anti-aliased coverage. Framebuffer alpha was not measured: the PPM dropped it, and `alpha=255` after converting to RGBA came from the converter |
| Launcher, scale 1 (560×420) | Query `hayu` with cursor, divider, five fixture rows, footer `Up/Down select   Enter launch   Esc close`; `Down` moved the highlight to row two: accent rect from x=10 to x=550, label and desktop id switched to the background colour |
| Clipping and ellipsis | The over-long fixture rendered as `Hayu Extremely Long Application Nam…` with `org.example.very.long.desktop.id.for.cli…` right-aligned, both inside the 560 px frame; no glyph crossed the surface edge |
| Scale 2 bar and launcher | Bar buffer 1920×64 for a 960×32 logical surface, launcher 1120×840 for 560×420: glyph metrics, pill, clock centring, divider and highlight all doubled; the images were inspected, not pixel-diffed |
| Reload height and edge | Renderer buffers: scale-1 bar 1366×32 → 1366×40 → 1366×32, scale-2 bar 1920×64 → 1920×80 → 1920×64. HDMI y moved 0 → 728 (768−40) per `hyprctl layers`. On the reverse reload the scale-2 surface box still read 960×40 while the buffer was already back to 960×32 logical; no composited capture shows it catching up before the next minute tick |
| No idle presents | A separate run with `WAYLAND_DEBUG=1` (local 14:59:35–15:02:03, 147 s; log timestamps are UTC) shows the bar surface `wl_surface#10` taking exactly 4 `attach` + `commit` pairs from `{mesa egl surface queue}`: 07:59:36.05 (startup), 08:00:00.02, 08:01:00.02 and 08:02:00.02 — one per minute boundary, nothing between. One earlier `commit` at 07:59:35.95 carried no buffer (the initial layer-surface commit). In the dump run that measured render attempts, 3 files in 126 s matched the same pattern |
| Cleanup | `hayu quit` left no process, the AT-SPI `IsEnabled` stayed `false`, and `HEADLESS-1` was removed with the real output back at 1366×768 scale 1 |
- What was not observed: the composited screen, so overlap with other layers, focus rings and cursor rendering were not checked; framebuffer alpha; any pixel comparison against `render::rasterize` (the images were inspected, not diffed); and fractional scale. Both integral scales ran at once on separate outputs, so that combination was exercised, but one output switching between scales was not.
- The dump was a temporary local patch, not a shipped feature. It ran synchronously on the UI thread, stalling the render, and wrote the whole surface including the visible launcher query, so an opt-in dump of this kind is not a privacy-safe diagnostic. It was removed after the run rather than kept.
- The idle evidence now counts `wl_surface.attach`/`commit` pairs emitted by Mesa's EGL surface queue, i.e. the buffer attaches that follow a successful `eglSwapBuffers` on the GL path. The earlier `wl_shm`-era count is superseded. It still counts published frames, not the compositor's own present timing.

- `cargo test` covers the frame/atlas code with the system font database; on a
  machine without fonts the text assertions are skipped the same way the old
  render tests were.

### Fractional and mixed scale (November 7, 2026)
Run at `4f8327d` on Hyprland 0.57.1 with `HDMI-A-1` (1366×768, scale 1) and a
temporary `HEADLESS-2` (1920×1080) set to scale 1.5 in one run and scale 2 in
another. Both `wp_fractional_scale_v1` and `wp_viewporter` were offered, so the
fractional path is what ran; no run removed either global. Buffers again came
from a temporary, uncommitted `glReadPixels` dump written before
`eglSwapBuffers` and removed after the run, so they show the renderer's output,
not the composited desktop.
```
WAYLAND_DEBUG=1 XDG_CONFIG_HOME=/tmp/... ./target/release/hayu run
```
| Check | Observed |
| --- | --- |
| Protocol use | The log repeats `wl_surface.set_buffer_scale(1)` and `wp_viewport.set_destination(...)` for each surface as it draws: `(1366, 32)` and `(1280, 32)` for the two bars and `(560, 420)` for the launcher |
| Bar buffer at 1.5 | Logical surface 1280×32, readback 1920×48; the scale-1 output painted 1366×32 at the same time, so mixed scales ran together |
| Launcher buffer at 1.5 | Logical 560×420, readback 840×630; the image showed query with cursor, divider, six fixture rows, the accent highlight with dark label and desktop id, the ellipsised long entry and the footer |
| Live scale change | Setting the same output to scale 2 without a restart moved the bar buffer 1440×48 → 1920×64 and the destination 1280 → 960 while the scale-1 output stayed 1366×32; reopening the launcher then produced 1120×840 for 560×420 |
| Rounding | `cargo test` 48/48, including `fractional_scale_sizes_the_frame_in_pixels`: `1366×1.25 → 1708`, `32×1.5 → 48`, `1×1.5 → 2`, and an 840×630 frame at 1.5 |
| Scale-1 regression | `input-suite.sh` 15/15, `reload-suite.sh` 41/41, `a11y-suite.py` exit 0, `doctor` exit 0, `config check` exit 0 — at scale 1, which now also draws through the viewport path |
- What was not observed: a live 1.25 scale (unit test only); the input and reload suites at 1.5 (they ran at scale 1); a composited screenshot; and the integer fallback, because both globals were present.
- The compositor restarted during this work: `coredumpctl` records a Hyprland 0.57.1 SIGSEGV inside its own cleanup path (`CCompositor::cleanup` → monitor and protocol destructors), its log has no hayu or viewport protocol error, and the suite results above were re-run on the new instance with the new `HYPRLAND_INSTANCE_SIGNATURE`. What requested that shutdown is not visible in the available logs.
### Bar module areas (November 7, 2026)
Unit checks in `../src/render.rs` cover the layout: `bar_areas_follow_the_configured_modules` puts `clock` in the left area and `workspaces` in the right and asserts their drawn positions, and `crowded_bar_stays_inside_the_frame` draws twelve workspaces plus a long title into a 320-pixel frame and asserts that no rect or glyph leaves the frame. `unknown_bar_widget_is_a_config_error` rejects `netwrk` in `bar.right` with the area path in the message.
Live check at this commit: `cargo test` 51/51, `../tools/live/reload-suite.sh` 42/42. The suite changes `bar.center` and asserts that the reload prints no bar-area restart notice while a `[tray]` change is still reported, and it keeps the geometry, filter and `bar.enabled` checks.
- What was not observed: a composited screenshot of the new layout, the areas at fractional scale, and pointer or workspace-click behaviour — the bar has no input regions yet.
### Bar workspace click (November 7, 2026)
Run at `f266ba9` on Hyprland 0.57.1 with one 1366×768 output at scale 1. The
bar was started from a temporary `XDG_CONFIG_HOME` with only `workspaces` in
`bar.left`, two workspaces existed on the focused monitor, and clicks came
from `zwlr_virtual_pointer_v1` through the `keyin` injector. Pill geometry came
from a temporary `HAYU_DEBUG_POINTER=1` trace that prints the drawn pills in
logical coordinates; the click point was the pill's centre.
| Check | Result |
| --- | --- |
| A left click on the pill for workspace 2 focuses workspace 2 | Live pass |
| A left click on the pill for workspace 1 focuses workspace 1 again | Live pass |
| A left click on empty bar space leaves the active workspace unchanged | Live pass |
| The shell survives the clicks and `hayu quit` exits 0 | Live pass |
- Method: `../tools/live/pointer-suite.sh` (7 checks) reads
  `hyprctl -j activeworkspace` before and after each click, and derives the
  click point from the drawn pill geometry rather than a hard-coded x.
- Rounding: `cargo test` 52/52, including `workspace_hit_test_uses_the_drawn_pills`
  (half-open ranges and logical-to-buffer scaling) and the crowded-frame test
  now also asserting that no pill leaves the frame. `reload-suite.sh` 42/42
  still passes.
- Limits: left click and workspace pills only — scroll, right/middle click and
  the other widgets are not implemented. One output at integral scale, and the
  injector needs `zwlr_virtual_pointer_manager_v1` plus a pointer-capable seat.
  The Lua fallback path is what ran: the config in this session rejects the
  legacy `dispatch workspace <id>` string.
### Tray items, icon and activation (November 7, 2026)
Run at `2c440da` on Hyprland 0.57.1 with one 1366×768 output at scale 1. The
real session already has another watcher (noctalia), so the check ran inside
`dbus-run-session` on a private bus; fcitx5 5.1.23 was the tray application,
started with a private `XDG_RUNTIME_DIR` whose `wayland-1` symlinks the real
socket. `../tools/live/tray-suite.sh` also starts a second shell to prove it does
not take the name.
| Check | Result |
| --- | --- |
| fcitx5 starts on the private bus | Live pass |
| The shell owns `org.kde.StatusNotifierWatcher` | Live pass (`:1.16` in that run) |
| `RegisteredStatusNotifierItems` lists the fcitx5 item | Live pass (`as 1 ":1.18"`) |
| The item's icon is drawn with a hit rect | Live pass (`bar tray 0:1338.0:20.0` on a 1366px bar) |
| A left click on the icon calls `Activate` | Live pass, seen by `dbus-monitor` |
| A second shell refuses to take the watcher name and the owner stays | Live pass |
| Killing fcitx5 removes the item from the property | Live pass |
| The shell survives the removal and `hayu quit` exits 0 | Live pass |
- Related checks: `cargo test` 56/56 (service-name forms, ARGB32 conversion,
  malformed pixmaps, an absent theme icon, and the sprite draw/hit test);
  `pointer-suite.sh` 7/7 and `reload-suite.sh` 42/42 still pass.
- Limits: no context menu, scroll or secondary activation; `NewIcon`/
  `NewTitle` are not followed, so an icon that changes in place is stale.
  SVG-only icon names draw nothing unless the same name exists as a PNG with
  its `-symbolic` suffix removed. The icon strip keeps the item's source size
  and is scaled by the sampler. Only fcitx5 was used, on a private bus, so a
  second real-world tray app and the real session watcher were not exercised.
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
| ghostty | `ghostty -e <program> <args>` | Stub argv live; `-e` special case in `../src/cli/ghostty.zig` |
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
