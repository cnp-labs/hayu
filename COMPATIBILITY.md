# Compatibility targets

Status: testing plan. One M0 prototype record exists for the developer machine; no V1 capability, release, or additional hardware has been declared tested.

## Initial targets

| Dimension | Target | Status |
| --- | --- | --- |
| OS/distribution | Arch Linux or its derivatives | User-decided; derivatives still require verification |
| Compositor | Hyprland | User-decided |
| CPU architecture | Candidate x86_64 | Proposed; user hardware not recorded |
| Hyprland version | Establish a minimum and one tested rolling version | Open in M0 |
| GPU/driver | One iGPU and one additional configuration if available | Validation hardware not yet available |
| Monitors | Single and dual; scales 1/1.25/1.5/2 | Planned |
| Locker | User-managed Hyprlock | Hayu does not implement locking |

Do not infer hardware from unrelated infrastructure projects. Test inventories record CPU/GPU model, RAM, driver, kernel, outputs, scale, Rust toolchain, and Hayu commit.

## Capabilities and fallbacks

| Area | Requirement | If unavailable |
| --- | --- | --- |
| Bar/panel/wallpaper | Wayland layer-shell and output lifecycle | UI startup fails with clear diagnostics; no silent ordinary-window fallback |
| Workspaces/windows | Hyprland request/event IPC | Degraded mode, marked stale state, retry/resnapshot, disabled actions |
| Fractional scaling | Preferred fractional scale + viewporter when available | Tested integer fallback; report limitations |
| Launcher input | Keyboard and selected text-input/IME stack | IME must be tested before release; record unavailability |
| Clipboard history | Compositor-advertised data-control | Module unavailable; other features remain active; no global capture without a protocol |
| Notifications | Session bus and available notification name | Conflict/unavailable state without forcibly taking another owner's name |
| Tray | StatusNotifier and DBusMenu | Isolate item/menu failures; report watcher conflicts |
| Audio | Tested backend for PipeWire/WirePlumber | Controls unavailable rather than a false 0% |
| Network | NetworkManager and policy permissions | Unavailable/permission-denied state; shell continues |
| Battery | UPower or a selected tested adapter | Hide/show unknown on desktops without a battery |
| Media | MPRIS player | Empty state without polling nonexistent players |
| Brightness | Controllable backlight | Hide/disable control; external monitor DDC/CI deferred |
| Accessibility | Tested AT-SPI bridge | Accessibility release blocker, not a false support claim |

Discover exact protocol names/revisions from globals on the target machine and record them in M0 results. Library support does not establish compositor or Hayu support.

## Services and packaging

Finalize dependencies after selecting the renderer/libraries. Candidates include Wayland, text/font, GPU backend, D-Bus, audio, and image-decoder libraries. Do not publish a PKGBUILD with guessed dependencies.

M6 target: an Arch recipe tested in a clean chroot; correct install/uninstall prefixes; installed example config and documentation; separated runtime and build dependencies. AUR publication and official binaries remain open. Do not automatically change autostart, bindings, or Hyprlock configuration. Write final installation commands only after testing them.

Do not make the entire session depend on a particular systemd user service without documentation; startup follows the compositor session. Provide one official startup path first and prevent duplicate instances.

## Compatibility result template

| Date | Hayu commit | Hyprland | Distribution/kernel | GPU/driver | Monitors/scale | Modules/results | Sanitized logs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-11-05 | 02a7687 | 0.57.0 | Arch Linux, kernel not recorded | AMD Radeon R7 240 (Oland PRO), driver not recorded | 1 × HDMI-A-1, 1366×768, scale 1 | M0 bar only: layer maps, namespaces `hayu`, workspaces/clock/title render; `doctor` and `config check` work; SIGTERM exits 0 | PSS 5.6 MiB; idle CPU 0.07% of one core over 30 s |

| 2026-11-05 | df68feb | 0.57.0 | Arch Linux, kernel 7.2.14-zen1 | AMD Radeon R7 200 (RADV OLAND, Mesa 26.2.6), Vulkan 1.3.354 | 1 × HDMI-A-1 1366×768 scale 1, plus a temporary headless output | Renderer spike: GLES/EGL and wgpu present the same scene on a layer surface, survive output add/remove, exit 0 on SIGTERM; GL also runs on llvmpipe; wgpu has no software Vulkan here | Offscreen GL/wgpu frames bit-identical; MAD vs CPU 0.0127% at scale 1, 0.0086% at scale 2; live scale change not exercised |

| 2026-11-05 | 1785746 | 0.57.0 | Arch Linux, kernel 7.2.14-zen1 | AMD Radeon R7 200 (RADV OLAND, Mesa 26.2.6) | 1 × HDMI-A-1 1366×768 scale 1 | Launcher slice: `hayu-launcher` overlay opens and closes through the control socket, `quit` exits 0, a second instance refuses to bind, a missing shell reports a clear error, and a scale-2 frame renders offscreen | Desktop-entry scan and key handling covered by unit tests; live typing not exercised (no wtype/ydotool), IME and key repeat not verified |

| 2026-11-05 | 10fbb4c | 0.57.0 | Arch Linux, kernel 7.2.14-zen1 | AMD Radeon R7 200 (RADV OLAND, Mesa 26.2.6) | 1 × HDMI-A-1 1366×768 scale 1 | Launcher driven with injected `zwp_virtual_keyboard_v1` events: open, type, navigate, launch, close, reopen, held-key repeat, empty results and launch errors all behaved; terminal entries run in foot; `quit` exits 0 | Keyboard repeat comes from the compositor's `repeat_info`; IME, runtime keymap changes, scale 2, and multi-monitor were not exercised |

| 2026-11-05 | 9e0a4d3 | 0.57.0 | Arch Linux, kernel 7.2.14-zen1 | AMD Radeon R7 200 (RADV OLAND, Mesa 26.2.6) | 1–2 outputs: HDMI-A-1 1366×768 scale 2, HEADLESS-1 1920×1080 scale 2 | Full launcher suite passes at scale 2; one bar per output at its own logical size; the launcher opens on the focused output and typing, selection and launch work there; removing that output while the launcher is open leaves the shell alive and the next open lands on the remaining output; a `us`→`de` keymap switch changes the character for the same key without a restart | Both outputs ran at scale 2; mixed per-output scales, fractional scaling and IME were not exercised; the keymap switch used a virtual keyboard's keymap rather than a physical layout switch, and the output removal was a headless output, not a cable |
| 2026-11-06 | d3781f7 | 0.57.0 | Arch Linux, kernel 7.2.14-zen1 | AMD Radeon R7 200 (RADV OLAND, Mesa 26.2.6) | 1 × HDMI-A-1 1366×768 scale 1 | Config reload from a temporary `XDG_CONFIG_HOME`: 32px top bar became 40px bottom at y=728, an invalid height exited 1 without changing it, `launcher.enabled=false` took effect and closed an open overlay, and a change to `bar.outputs` printed `restart required for: bar.outputs` while still exiting 0. The checked-in `tools/live/input-suite.sh` passed 15/15 and `reload-suite.sh` 28/28 | Single output at scale 1; theme/clock applied without a pixel diff; `bar.outputs` and widget lists still need a restart; the suite needs `zwp_virtual_keyboard_manager_v1` |
| 2026-11-06 | 310a7c7 | 0.57.0 | Arch Linux, kernel 7.2.14-zen1 | AMD Radeon R7 200 (RADV OLAND, Mesa 26.2.6) | 1 × HDMI-A-1 1366×768 scale 1, plus a temporary HEADLESS 1920×1080 | Reload applies `bar.outputs` and `bar.enabled` live: a filter with no match and a disabled bar both removed the bar without a restart, the launcher still opened through the output fallback, re-enabling restored the 40px bar, and a named filter moved the bar onto a hotplugged headless output whose removal removed it. The checked-in `tools/live/input-suite.sh` still passed 15/15, and `reload-suite.sh` passed 41/41 | One real output at scale 1; the filter check uses a headless output, not a second monitor; theme/clock applied without a pixel diff; widget lists still need a restart |
| 2026-11-06 | 4de4112 | 0.57.0 | Arch Linux, kernel 7.2.14-zen1 | AMD Radeon R7 200 (RADV OLAND, Mesa 26.2.6) | 1 × HDMI-A-1 1366×768 scale 1, plus a temporary HEADLESS 1920×1080 | Launcher text input through `zwp_text_input_v3` with fcitx5 5.1.23 and fcitx5-anthy 5.1.11: inline preedit, one `commit_string(日本)`, BackSpace inside and after composition, clean reset on close/reopen, and cleanup on quit, reload and output removal; `input-suite.sh` passed 15/15 with fcitx5 stopped | Preedit pixels and the input method's candidate popup were not checked; a second Escape after a cancelled composition is consumed by anthy, so I4 in ADR-0007 stays open; the removal check used a headless output |

Terminal wrapping was checked for every fallback with recording stubs on `PATH`; only foot is installed on this machine, so the other emulators' forms rest on their upstream documentation.
Config reload was checked on the live bar: a 32px top bar became a 40px bottom bar at y=728 on a 768px output, an invalid height exited 1 without changing it, and `launcher.enabled=false` took effect while the shell kept running. Theme and clock strings were applied by the same call but no pixel diff was taken. `bar.outputs` and `bar.enabled` were applied live, including on a hotplugged headless output; the widget lists still need a restart.

These rows cover surface creation, text drawing, Hyprland snapshot/event handling, renderer presentation, launcher control and keyboard mapping, and clean shutdown. They do not cover fractional scaling, mixed per-output scales, real display hotplug, tray, audio, notifications, or accessibility; those were not exercised.

Releases list tested version ranges, known issues, protocol gaps, and optional dependencies. Other distributions may be tried on a best-effort basis, but must not be labeled supported before testing.
