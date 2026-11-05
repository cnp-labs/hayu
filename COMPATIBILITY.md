# Compatibility targets

Status: testing plan; **no hardware/version combination has been declared tested**.

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
| Not tested | — | — | — | — | — | — | — |

Releases list tested version ranges, known issues, protocol gaps, and optional dependencies. Other distributions may be tried on a best-effort basis, but must not be labeled supported before testing.
