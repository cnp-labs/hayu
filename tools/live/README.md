# Live check harness

Two scripts that drive the real `hayu` binary on a real Hyprland session and
record what happened. They exist so the live evidence in
[TESTING.md](../../TESTING.md) can be reproduced instead of trusted.

```
tools/live/input-suite.sh     # launcher: typing, selection, launch, terminal, errors
tools/live/reload-suite.sh    # config reload: geometry, invalid file, launcher gate, notice
```

Both scripts:

- build `target/release/hayu` if it is missing, and the injector if needed;
- start their own shell against a temporary `XDG_CONFIG_HOME` and
  `XDG_DATA_HOME`, so your real config and applications are not read or
  written;
- print one `PASS`/`FAIL` line per check and exit non-zero on any failure;
- stop the shell they started, even on failure.

Requirements: a running Hyprland session (the scripts use `hyprctl layers -j`),
`libxkbcommon` with its data files, `python3` for the layer query, and `foot`
for the `Terminal=true` check (that check is skipped when foot is missing).

The shell runs with `HAYU_DEBUG_KEYS=1` and `HAYU_DEBUG_SCAN=1`, so every check
sees the real key path, query, selection and result count in the log instead of
inferring them from the config.

## `keyin`

`tools/live/keyin` is a throwaway client that binds
`zwp_virtual_keyboard_v1` on the first seat, uploads an xkb keymap and sends
raw evdev keycodes. It is a separate Cargo project, so `cargo build --release`
at the repository root does not build it.

```
cargo build --release --offline --manifest-path tools/live/keyin/Cargo.toml
tools/live/keyin/target/release/keyin type:hayu test
tools/live/keyin/target/release/keyin press:Down press:Return
tools/live/keyin/target/release/keyin hold:BackSpace
tools/live/keyin/target/release/keyin raw:44
```

- `type:` uses a fixed `us` keycode table, so the typed text does not depend on
  the caller's layout.
- `press:`/`hold:` accept `Return`, `Escape`, `BackSpace`, `Tab`, `Delete`,
  `Up`, `Down`, `Left`, `Right`, `Home`, `End`. `hold:` keeps the key down for
  900 ms so the compositor's own repeat produces several events.
- `raw:` sends an evdev keycode directly; the character then comes from the
  uploaded keymap. `KEYIN_LAYOUT=de` plus `raw:44` is how the runtime keymap
  switch was checked.
- `KEYIN_STEP_MS` changes the delay between keys (default 20).

## Fixtures

`data/applications/` holds the desktop entries the input suite copies into its
temporary `XDG_DATA_HOME`:

| Entry | Purpose |
| --- | --- |
| `Hayu Test One/Two/Three` | query `hayu test` returns three results for navigation |
| `Hayu Terminal Touch` | `Terminal=true`; the launched `touch` proves the terminal ran it |
| `Hayu Failing Launch` | `Exec` points at a missing binary, so the error path is exercised |

Launched commands only touch files under `/tmp/hayu-live`, which the input
suite recreates at start. Do not point them at anything else.

## AT-SPI launcher check

`a11y-suite.py` starts an isolated Hayu with three synthetic desktop entries and
walks the AT-SPI tree while it types, moves the selection, closes and reopens
the launcher. It needs `/usr/bin/python3` with PyGObject's `Atspi` typelib,
`gdbus` and `at-spi2-core`. It refuses to run when another Hayu shell or control
socket is present, sets `org.a11y.Status IsEnabled` only for the run and
restores it, and keeps its log under `/tmp/opencode`.

```
/usr/bin/python3 tools/live/a11y-suite.py
```

It checks bus-visible names, roles, text values and the selected flag; it is
read-only and does not prove screen-reader speech.

## Limits

These are single-session checks, not a test suite: one output, integral scale,
no screenshot comparison (there is no screenshot tool on the reference machine),
and IME, fractional scaling, mixed per-output scales and screen-reader speech
are not covered. They also depend on the compositor keeping
`zwp_virtual_keyboard_manager_v1` available to ordinary clients.
