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

These rows cover surface creation, text drawing, Hyprland snapshot/event handling, renderer presentation, launcher control and keyboard mapping, and clean shutdown on a single output. They do not cover IME, fractional scaling, multi-monitor, hotplug, tray, audio, notifications, or accessibility; those were not exercised.

Releases list tested version ranges, known issues, protocol gaps, and optional dependencies. Other distributions may be tried on a best-effort basis, but must not be labeled supported before testing.
