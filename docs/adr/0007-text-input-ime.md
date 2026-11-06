# ADR-0007 — Text input and IME through text-input-v3
Date: November 6, 2026. Status: **accepted and implemented** in `4de4112`.
I1-I10 were run with fcitx5 and fcitx5-anthy (results in TESTING.md); I4 is the
only criterion that did not pass, because the input method consumes the second
Escape after a cancelled composition.

## Context
The launcher is a layer-shell surface with an exclusive keyboard grab while it
is open. Today it turns `wl_keyboard` keysyms into text itself, which covers
ASCII but cannot produce composed text. M1 requires proven IME support before
the launcher can be called usable for non-Latin input, and O-03 in
DECISIONS.md still leaves the text stack and protocol libraries open.

Hyprland 0.57.0 advertises `zwp_text_input_manager_v3` and
`zwp_input_method_manager_v2`. The client side of the first is the only role
Hayu needs: the input method (for example fcitx5) is a separate process that
binds the second one. Binding the input-method protocol ourselves would make
Hayu an IME, which it is not.

There is no existing text input helper in the toolkit dependency, so this ADR
fixes the protocol and the ownership boundary before code exists, not after a
particular implementation turns out easy.

## Decision
Use `zwp_text_input_v3` from `wayland-protocols` for composed text in the
launcher. Keep the current `wl_keyboard` path for navigation and for plain
typing when no input method is composing. Do not add a configuration switch:
the protocol is used when the compositor offers it, and its absence is silent
apart from one startup diagnostic.

Candidate lists, the input method's own popup, on-screen keyboards, and global
text input for future V1 fields are out of scope.

## Scope
In scope for the M1 slice:

- one `zwp_text_input_v3` object per seat, created once the seat exists;
- enabling it only while the launcher surface has text-input focus;
- inline preedit in the launcher, committed text inserted at the query cursor,
  and `delete_surrounding_text` applied to the query;
- cursor rectangle, surrounding text, change cause and content type kept in
  sync with the query;
- clean disable, reset and destruction on close, reload, output removal and
  quit;
- keyboard-only operation when the global is missing or an input method is not
  running.

Out of scope: candidate windows, IME popup positioning, multi-seat layouts,
text input on the bar, rich text, selection ranges inside the query, and any
V1 text field other than the launcher.

## Integration with input
- The bar never enables text input. Only the launcher surface requests it, and
  only while the overlay is open and has keyboard focus.
- `wl_keyboard` stays authoritative for `Up`, `Down`, `Left`, `Right`, `Home`,
  `End`, `Return`, `Escape`, `Tab` and `BackSpace`. Printable input that the
  input method consumed arrives as `commit_string`; it must not be inserted a
  second time from a keysym.
- The launcher query has one caret, always at the end. `set_surrounding_text`
  sends the whole query with the cursor at its byte length, so composed text
  can replace or delete around that caret.
- `set_content_type` declares a single-line, no-spellcheck search field:
  purpose `normal`, hint `none`. The slice does not request completion,
  auto-capitalization or a hidden password field.
- `set_text_change_cause` reports `other` for launcher-driven edits and the
  protocol cause for IME-driven edits; `set_cursor_rectangle` reports the
  caret rectangle in surface coordinates.
- Changes from the input method are applied on `done`, in the protocol's
  order: replace preedit, apply `delete_surrounding_text`, insert
  `commit_string`, then store the new preedit. After that the client sets
  surrounding text, cursor rectangle and content type again and calls `commit`.
- A `done` serial that does not match the number of issued commits is still
  applied to the query but must not overwrite the pending client state, as the
  protocol requires.

## Lifecycle
1. Startup: bind `zwp_text_input_manager_v3` if present and create one object
   for the seat. A missing global is recorded once; no retry loop.
2. Surface focus: on `enter` for the launcher surface, call `enable`,
   `set_content_type`, `set_surrounding_text`, `set_cursor_rectangle` and
   `commit`. If the launcher is not open, ignore the event.
3. Close or focus loss: clear the stored preedit, call `disable` and `commit`,
   and reset all pending state. Opening again starts from an empty query and
   must not show stale preedit.
4. Reload that disables the launcher, output removal and quit: the same
   disable/reset path runs before the surface or object is destroyed. Object
   destruction happens after the disable commit is flushed.
5. Protocol error, a compositor without the global, or a failed bind: log once,
   destroy the object if it exists, and keep the keyboard path working. The
   launcher must remain open and usable.

## Fallback
The permanent fallback is the current keysym path: ASCII typing, filtering,
selection, launch, cancel and held-key repeat work exactly as the 15-check
input suite exercises. No config key enables or disables text input, and no
error dialog is shown when no IME is installed, because an absent IME is not a
Hayu failure. An IME that crashes or disconnects leaves the launcher usable;
the next open tries again.

## Acceptance criteria with a real IME
These criteria are the M1 exit evidence. They are run with
`fcitx5` plus `fcitx5-anthy` (both in Arch `extra`; not installed on the
reference machine at the time of this ADR), started with `fcitx5 --replace -d`
and toggled with `fcitx5-remote -o`. Anthy is chosen because it composes
visible Japanese text from Latin keys, so a failure is obvious without relying
on a candidate UI. The table exists in TESTING.md at `4de4112`. I4 stays open, so
this ADR supports a launcher IME claim only with that exception stated: after a
cancelled composition, one Escape is consumed by the input method before the
next one closes the overlay.

| ID | Criterion | Method |
| --- | --- | --- |
| I1 | Protocol bound once on the launcher seat | `HAYU_DEBUG_IME=1` logs the bind and the manager version; no object is created on the bar |
| I2 | Preedit is inline and does not filter early | Type `nihon` with fcitx5 active; the launcher shows the preedit and the result list stays unchanged until commit |
| I3 | Commit inserts exactly once and launches | Commit `日本` with Space/Enter, then press Return; the query shows `日本` once, filtering reflects it, and the selected entry runs |
| I4 | Cancel leaves the launcher usable | Escape during preedit clears the preedit without closing; a second Escape closes the overlay |
| I5 | Editing stays inside the protocol | BackSpace during preedit removes one preedit character and does not delete committed query text |
| I6 | Surrounding text stays correct | After a commit, surrounding text and cursor rectangle match the query; a following BackSpace removes the last committed character only |
| I7 | Close and reopen reset the field | Close while preedit is active, reopen, type ASCII; there is no stale preedit and no stale surround |
| I8 | Missing IME falls back cleanly | With fcitx5 stopped, the 15-check input suite still passes and the log has no protocol error |
| I9 | Teardown is clean | Quit, reload with the launcher disabled, and remove the focused output while preedit is active; each exits without a protocol error and nothing panics |
| I10 | Text is not logged by default | With `HAYU_DEBUG_IME=1` but without `HAYU_DEBUG_KEYS=1`, the trace records event kinds and lengths, never the query, preedit or commit text |

ASCII typing with fcitx5 active must also produce one character, not two: the
keyboard path and `commit_string` must not both insert the same letter.

## Alternatives considered
- **text-input-unstable-v1**: superseded, and Hyprland exposes v3. Rejected.
- **Bind `zwp_input_method_manager_v2` in Hayu**: that is the input method's
  role, not the application's. It would make Hayu an IME and still not give the
  launcher composed text. Rejected.
- **Toolkit text entry (GTK/Qt)**: Hayu draws its own surfaces and does not use
  a toolkit widget. Rejected.
- **Keyboard only**: fails HAYU-003/HAYU-015 and cannot type Japanese,
  Chinese or Korean. Rejected except as the fallback.
- **Custom IME protocol or private socket**: no interoperability and a larger
  security surface. Rejected.

## Consequences
- `wayland-protocols` already is a dependency with the `client` feature; no
  new crate is required.
- Hayu gains one more Wayland object whose lifecycle must follow the launcher
  surface, and a preedit string whose length must be bounded like the query.
- Composition is visible only inside the launcher; V1 text fields need the
  same protocol but are not covered by this ADR.
- O-03 remains open for shaping, font fallback and the rest of the text stack;
  this ADR only fixes the IME protocol and the keyboard fallback. DECISIONS.md
  records the decision as D-10.
- The existing evidence for keyboard-only use stays valid; an IME claim is
  added only when I1–I10 pass on a recorded commit.
