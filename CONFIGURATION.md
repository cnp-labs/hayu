# Configuration, state, and IPC

Status: **schema v1 design**. `hayu config check` validates the schema today; only bar/clock/appearance keys affect a running process, and no other module exists yet. TOML is user-decided; the keys/defaults below are baseline proposals. Full example: [examples/config.toml](examples/config.toml).

## Locations and precedence

Follow XDG Base Directory with standard fallbacks:

| Data | Proposed location |
| --- | --- |
| Configuration | `$XDG_CONFIG_HOME/hayu/config.toml`, fallback `~/.config/hayu/config.toml` |
| Versioned state | `$XDG_STATE_HOME/hayu/state.json`, fallback `~/.local/state/hayu/state.json` |
| Cache | `$XDG_CACHE_HOME/hayu/`, fallback `~/.cache/hayu/` |
| Runtime IPC | `$XDG_RUNTIME_DIR/hayu/<session-id>/control.sock` |

The runtime directory must be valid and user-owned; do not use a cross-user `/tmp/hayu.sock`. Derive the session ID from validated session identity rather than inserting raw environment strings into paths.

Initial precedence: built-in defaults → one TOML config → supported CLI arguments for that process. No includes, directory merges, environment interpolation, or hidden GUI overrides yet. Expand `~` only in documented path fields, in application code; TOML itself performs no expansion.

## Core schema

| Key/table | Proposed type and default | Validation |
| --- | --- | --- |
| schema_version | integer `1` | Reject newer versions; explicit migration |
| appearance.mode | `dark` | `dark` or `light` |
| appearance.accent | hex string `#8AB4F8` | Valid RGB color format |
| appearance.animations | bool `false` | Motion is not required for functionality |
| appearance.reduce_motion | bool `false` | Overrides animations when true |
| bar.enabled/position/height | true / `top` / 32 | `top` or `bottom`; height 24–96 logical pixels |
| bar.outputs | string array `["*"]` | All-output wildcard or exact output names |
| bar.left/center/right | widget ID arrays | Known IDs; validated service references |
| clock.format | string `%H:%M` | Contract uses a strftime subset; local session timezone |
| launcher.enabled/favorites | true / desktop ID array | Mark unknown favorites; never execute as commands |
| tray.enabled | true | Requires session bus |
| notifications.enabled/dnd | true / false | Do not forcibly take another owner's name |
| notifications.history_limit | integer `100` | Range 1–1000; session-only |
| control_center.audio/brightness/network/battery | bool true | Backend unavailability is a runtime status |
| media.enabled / osd.enabled | true / true | Action capabilities come from backends |
| wallpaper.enabled/directories/mode | true / `[]` / `fill` | Missing directories may warn; `fit`/`fill` |
| wallpaper.outputs | output → image path map | Runtime file errors use a fallback |
| dock.enabled/pinned | true / desktop ID array | Resolve desktop IDs; empty is valid |
| clipboard.enabled/max_entries | false / `100` | Opt-in; range 1–1000 |
| clipboard.max_item_bytes | integer `65536` | Range 1024–1048576; check before unbounded allocation/reads |

Unknown keys produce errors with paths and locations so typos are not ignored. Candidate config size cap: 1 MiB. Duplicate keys follow TOML parser rules. A single file simplifies errors and schema evolution.

## Hot reload and runtime changes

Watch the directory to detect editors' atomic renames; candidate debounce: 150 ms. Parse → schema validation → semantic validation → resource preparation → commit. On failure, keep the old config and publish errors to logs/diagnostics, not an unbounded stream of notifications.

Disabling a module stops its consumers and frees dedicated resources. Shared resources remain while other modules use them. Plan surface position/size/output changes so old exclusive zones are not left behind. Reload generations cancel stale worker results.

Panel interactions change runtime state rather than silently rewriting user configuration. Persist DND, pinning, favorites, and wallpaper selections as state. For favorites/pinning/wallpaper, config entries provide the baseline; state stores later interactive changes. If the related config field changes on reload, it replaces the corresponding state and discards the old override. Test and explain this precedence in the UI.

## State and privacy

Versioned JSON state stores only necessary desktop metadata: pinning/favorites, DND, and wallpaper paths. Write atomically within the same directory, with private permissions; quarantine corrupt state and start from defaults with a warning. State is not a password or clipboard-content store.

Clipboard and notification history remain in session memory, are bounded and clearable, and do not enter logs. Candidate total clipboard cap: 8 MiB; notification payload/history cap: 4 MiB, including application-managed metadata. Evict FIFO when count/byte caps are exceeded. Bound per-notification images/payloads before decoding. Sensitive-data persistence requires a new decision.

Candidate disk thumbnail/icon cache maximum: 128 MiB with LRU eviction; resident textures have a separate PERFORMANCE budget. These limits are proposals to measure during M0–M6.

## CLI and integration examples

Proposed contract: `hayu run`; `hayu doctor`; `hayu config check`; `hayu config reload`; `hayu launcher toggle`; `hayu panel toggle control-center`; `hayu clipboard pause`; `hayu clipboard clear`; `hayu quit`.

The CLI uses the correct session endpoint, exits nonzero on errors, and provides actionable messages. `doctor` remains useful when the daemon is not running. IPC has a protocol version, request ID, size limits, timeouts, and owner-only permissions. It does not expose arbitrary command execution.

Write final Hyprland keybinding/autostart examples after recording the target Hyprland configuration version; do not copy untested legacy syntax. In the M0 slice only `run`, `doctor`, `config check`, `launcher toggle|open|close`, and `quit` exist. The launcher honours `$TERMINAL` (then foot, alacritty, kitty, wezterm, ghostty, konsole, xterm) for `Terminal=true` entries and rescans XDG application directories each time it opens. The launcher commands go through `$XDG_RUNTIME_DIR/hayu/<session-id>/control.sock` with protocol v1, a 4 KiB request cap, a 1 s timeout, directory mode 0700, and socket mode 0600. Every other command here is a plan, not an installation step already proven to work.
