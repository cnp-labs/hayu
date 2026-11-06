# AGENTS.md — Hayu

Hayu is a Rust Wayland desktop shell for Hyprland. The repository holds the V1
planning baseline (Markdown is the specification) plus an **M0 prototype**
in `src/`: one layer-shell bar, a keyboard launcher reachable over a local
control socket, `hayu config check`, `hayu doctor`, and a renderer spike binary.
There is no V1 feature beyond the bar and launcher, and no packaging or CI. The
renderer is decided: GLES/EGL with glow, CPU path for tests only (ADR-0006);
both surfaces still draw through tiny-skia until M1.

## Verified commands

```
cargo build --release
cargo test                                 # 41 unit tests
./target/release/hayu doctor               # works without a shell running
./target/release/hayu config check examples/config.toml
./target/release/hayu run                  # needs a live Hyprland session
./target/release/hayu launcher toggle      # needs `hayu run`; also open|close
./target/release/hayu quit                 # stops the running shell
./target/release/hayu reload               # re-reads config; needs `hayu run`
cargo build --release --offline --manifest-path tools/live/keyin/Cargo.toml
tools/live/input-suite.sh                  # live launcher suite; needs Hyprland
tools/live/reload-suite.sh                 # live reload suite; needs Hyprland
./target/release/hayu-gpu-spike --backend gl --mode offscreen --frames 300 --json
cargo build --release --features wgpu      # opt-in backend for the spike only
```

`hayu run` maps a real bar over the user's desktop and blocks. Use a timeout for
smoke checks and stop it with SIGTERM; do not leave it running. `hayu run` reads
only `$XDG_CONFIG_HOME/hayu/config.toml` (no `--config` flag yet). Client
commands talk to `$XDG_RUNTIME_DIR/hayu/<session-id>/control.sock`; without a
running shell they exit non-zero with a readable message.

`hayu reload` applies the theme, clock format, bar height/position,
`bar.enabled`, `bar.outputs` and `launcher.enabled`; the widget lists and
every unread section still need a restart, and a change to one of them
prints `restart required for: ...` while still exiting 0. An invalid file
exits non-zero and leaves the running state untouched.

## Do not overclaim

- Never write that a V1 capability is implemented, tested, benchmarked, or
  supported. Only the bar, launcher slice, config validation, and `doctor` exist; TESTING.md and
  PERFORMANCE.md record that no T-01–T-20 result and no V1 profile measurement
  exists.
- `DECISIONS.md` is the authority on status: **D-01..D-09 user-decided**,
  **A-01..A-09 proposals**, **O-02..O-08 open**; **O-01 is closed** by ADR-0006.
  Never promote a proposal or an open question to a fact, and do not reopen the
  renderer without new measured evidence.
- Requirements are `HAYU-001..016` (PRD.md), tests `T-01..T-20` (TESTING.md).
  Reference these IDs instead of inventing numbering.
- "First release" means full V1 at M6, never the M0 bar.
- Foundational changes (UI foundation, config format, renderer, privacy/state
  model) require a new ADR in `docs/adr/`; next number is **0008**. Feature
  changes need a PRD ID plus matching SCOPE, TESTING, and budget updates.
- Noctalia creates no feature-parity obligation; do not copy its code, assets,
  or palette.

## Keep docs and behavior in sync

Update all affected files in the same commit:

- Config key added or changed → `CONFIGURATION.md` schema table and
  `examples/config.toml`; `src/config.rs` must keep rejecting unknown keys.
- New runtime behavior → the relevant `ARCHITECTURE.md` rules, and
  `DESIGN.md` if it is visible UI.
- Capability or fallback → `COMPATIBILITY.md`; new measurable number →
  `PERFORMANCE.md`, labelled a hypothesis unless actually measured.
- Milestone progress → `ROADMAP.md`, and `DECISIONS.md` if a gate closes.
- Any claim that changes reality (new command, new surface, benchmark) → fix
  the status lines in README/TESTING/PERFORMANCE that say it does not exist.

Locked boundaries: Hyprlock stays the user's locker; no compositor/window
management, no Lua/runtime scripting, no plugin marketplace, no non-Arch
support in V1.

## Code gotchas learned the hard way

- cosmic-text: calling `FontSystem::db_mut()` per frame invalidates its font
  caches and turned one bar draw from 0.8 ms into 36 ms. Use `db()` for
  read-only checks; only mutate the database at startup.
- Redraw must stay event-driven. The bar draws on Hyprland events, configure,
  and the exact minute tick; do not add a frame loop or per-widget polling.
- The Hyprland adapter owns both sockets; widgets must not talk to them
  directly. Reconnects request a resync before trusting state again.
- The control socket is not the Hyprland socket: `src/ipc.rs` owns
  `$XDG_RUNTIME_DIR/hayu/<session>/control.sock` (0700 dir, 0600 socket,
  protocol v1, 4 KiB request cap, 1 s timeout). It runs as a calloop source,
  reads on a worker thread, and answers on the UI thread; do not block the UI
  thread on accept or file IO.
- Launcher keyboard interactivity is `Exclusive` only while the overlay is open
  and `None` for the bar. Only one redraw path exists; do not add timers or
  per-keystroke desktop scans (entries are rescanned once per open).
- Desktop-entry rules live in `src/desktop.rs`: `[Desktop Entry]` only,
  `Type=Application`, honour `Hidden`/`NoDisplay`/`OnlyShowIn`/`TryExec`, strip
  the allowed `%` field codes and reject unknown ones, and spawn an argument
  vector and never a shell. The catalogue is rescanned on every launcher open
  (a few ms), so installed entries appear without a restart.
- `Terminal=true` entries run in a terminal found from `$TERMINAL` or a short
  fallback list, and the argument form is per emulator: foot takes the command
  positionally, alacritty/ghostty/konsole/xterm use `-e`, kitty uses `--`, and
  wezterm needs `start --`. A known emulator in `$TERMINAL` keeps its form; an
  unknown one is assumed to accept `-e`. A missing emulator is an error shown
  in the footer, not a silent no-op.
- `HAYU_DEBUG_KEYS=1` traces keysyms, query, selection and per-frame draw time;
  `HAYU_DEBUG_SCAN=1` traces the scan duration. Both are off by default and are
  what the live input check asserts on. Do not log query text without them.
- Keysyms come from the seat's current xkb keymap, which the keyboard handler
  rebuilds on `wl_keyboard.keymap`; never cache key-to-character mappings. A
  runtime layout switch must keep working without a restart.
- The launcher opens on the focused monitor and falls back to the first bar; a
  removed output drops its surface without ending the process.
- The launcher surface is reused across opens: closing detaches the buffer and
  clears `configured`, opening re-requests a configure and paints only from
  that event. Attaching a buffer before a fresh configure makes the compositor
  error out and the overlay never maps again.
- `src/config.rs` is the only schema consumer today. Adding a key there is not
  implementation work; the module that uses it still has to exist.
- GL backend: every output owns an EGL context, so `make_current` before each
  draw is mandatory; skipping it crashed the second output with
  "argument does not name a valid surface" only after a hotplug.
- The wgpu backend is compiled out unless `--features wgpu`; default builds must
  keep working with the `gl` feature only.
- Renderer evidence lives in ADR-0006; do not restate its PSS/latency numbers as
  V1 profile results.

## Language and commit workflow (user-mandated)

- All Markdown files are **English**, even when the conversation is in
  Indonesian. Code identifiers and CLI names stay verbatim.
- Commit every change. **Split commits by logical unit** — one file group and
  one intent per commit, with a detailed English message (subject plus bullets
  naming what changed and which caveats remain). Never one big commit. A new
  module plus the wiring that makes it usable is one unit; independent modules
  are separate commits, and each commit should build.
- **Never `git push` and never open a PR without explicit approval.** Local
  commits only until the user says otherwise.

## Where to start reading

`README.md` → `PRD.md` → `SCOPE.md` → `DECISIONS.md` → `ROADMAP.md` for intent;
`src/main.rs` → `src/bar.rs` → `src/hyprland.rs` for the code,
`src/launcher.rs` → `src/desktop.rs` → `src/ipc.rs` for the launcher slice,
`src/gpu/` for the renderer spike; `docs/adr/` for the reasoning behind locked
choices.
