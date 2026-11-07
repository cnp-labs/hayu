# Hayu design system and UI behavior

Status: initial design proposal 0.1, November 2, 2026. The final logo, palette, and UI language have not been selected. These tokens are prototype starting points subject to evaluation.

## Principles

Readable information, reachable controls, visible keyboard focus, and brief optional motion. Consistency covers the bar, launcher, dock, control center, and pickers. Visual completeness must not depend on compositor blur; solid backgrounds must remain fully usable.

## Prototype tokens

| Token      | Dark    | Light   |
| ---------- | ------- | ------- |
| background | #111318 | #F7F8FA |
| surface    | #1C2028 | #FFFFFF |
| text       | #F2F4F8 | #17202B |
| muted      | #AAB2C0 | #536173 |
| accent     | #8AB4F8 | #245BB2 |
| border     | #363D49 | #CED5DF |
| error      | #FFB4AB | #A51C30 |

Verify contrast for each foreground/background pair; never communicate status by color alone. Candidate spacing: 4/8/12/16/24 logical pixels; radii: 6/10/14; bar height: 32; control targets: approximately 32–40 logical pixels. Final sizes follow readability and input testing.

Font: a system sans-serif family with fallback; final font and redistribution remain open. Icons use assets with recorded licenses and text fallbacks. Basic functionality must not require a particular Nerd Font.

## Internal component contracts

Minimum components: Box/Row/Column, Label, Icon, Button/IconButton, Toggle, Slider, TextInput, ScrollList, Menu, Tooltip, Toast, and Dialog. Each has normal/hover/pressed/focused/disabled states; async controls also have pending/error states. Distinguish decorative and interactive components in the semantics tree.

Contracts cover measure constraints, layout bounds, clip, input bounds, focusability, accessible role/name/value, actions, and invalidation. Component identity stays stable across updates to preserve focus/selection. V1 does not build a general-purpose public UI framework; the internal API follows real shell needs.

## Surface behavior

| Surface          | Behavior                                                                                                                                       |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| Bar              | Top/bottom, left/center/right areas; ellipsized titles; labeled indicators/tooltips; missing items do not break layout                         |
| Launcher         | Focus input on open; reset query on reopening by default; arrows select, Enter activates, Escape closes; clear empty results and launch errors. M0 slice: 560×420 overlay, eight rows, keyboard interactivity only while open; icons, IME, and favorites are not implemented |
| Control center   | Group audio, brightness, network, and battery; show pending/permissions; keyboard reaches every control                                        |
| Tray/menu        | Keep menus inside output bounds, scroll long menus, consistent keyboard navigation/dismissal                                                   |
| Notifications    | Toasts do not steal focus; default to focused output; hover may hold timeout; history panel offers clear/DND                                   |
| Dock             | Distinguish pinned/running applications; clicking a multiwindow indicator opens a chooser; missing icons have fallbacks                        |
| Wallpaper picker | Virtualized grid, async thumbnails, selection preview, explicit target monitor; errors do not remove a valid selection                         |
| Clipboard        | Opt-in/pause banner; bounded text previews; accessible clear action; do not interpret contents as markup                                       |
| OSD              | No focus; one slot per category; updates reset timeout; rapid volume changes do not stack                                                      |

By default, popups follow the focused monitor, unless invoked from a specific bar/dock control, in which case they use the originating monitor. Close popups when that monitor disappears. This proposed policy is tested on multiple monitors.

## Input, text, and accessibility

Tab/Shift+Tab move focus; arrows navigate lists/menus; Space/Enter follow the control type; Escape closes the topmost surface. Focus rings survive animation and relayout. Users configure compositor shortcuts; Hayu documents CLI actions only.

TextInput supports selection, cursor, copy/paste, grapheme clusters, and IME composition. Test Unicode shaping, font fallback, right-to-left text, and long titles from the foundation stage. Never truncate input arbitrarily by UTF-8 bytes.

V1 must have a semantics tree and an accessibility bridge tested with a screen reader in the target environment. AT-SPI implementation details remain open, but must not be deferred without a recorded risk and explicit gate change.

## Scaling and motion

Layout uses logical pixels. Buffers/textures follow output scale; text caches are keyed by scale/font generation. Test 100%, 125%, 150%, 200%, and moves across differently scaled monitors. Multiplying integer coordinates alone does not solve fractional scaling.

Short animations run only during transitions; reduce-motion is available without changing functionality. Candidate duration: 100–160 ms; no default idle animations. Transparency is optional; blur and advanced decoration are not V1 requirements.

## Visual evaluation

Capture light/dark, long-title, empty-list, unavailable-service, small-panel, and mixed-scale scenarios. Review readability and clipping on small outputs before adding decoration. Pair visual snapshots with actual input tests: images alone do not prove focus or accessibility.
