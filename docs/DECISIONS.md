# Decision log and assumptions

Baseline date: November 2, 2026. This document distinguishes user decisions from technical proposals.

## User-decided

| ID | Decision | Reference |
| --- | --- | --- |
| D-01 | Rust as the primary language | PRD; ADR-0001 |
| D-02 | Internal UI foundation | ADR-0001 |
| D-03 | Hyprland as the first compositor target | ADR-0002 |
| D-04 | Arch Linux or its derivatives as the initial target | ADR-0002 |
| D-05 | TOML; scripting deferred | ADR-0003 |
| D-06 | All selected feature categories included in V1 | PRD HAYU-001–016 |
| D-07 | Internal locker deferred; users use Hyprlock | SCOPE |
| D-08 | Motto “Lightweight desktop shell.” | README |
| D-09 | English for all Markdown files and documentation | User's subsequent language request |
| D-10 | Text input through `zwp_text_input_v3`, with the keysym path kept as the permanent fallback | ADR-0007; HAYU-003, HAYU-015 |

## November 7, 2026 refocus

D-11 (user-decided): prioritize a simple, minimalist, lightweight and responsive desktop shell, not a general-purpose UI framework. Evaluate CPU, RAM, GPU activity and background work in real use. Preserve Rust, Hyprland-first, TOML, GLES/EGL, working features, accessibility and IME. This supersedes conflicting broad-framework ambitions, not D-06's feature categories. Core/optional delivery priorities in [REFOCUS.md](REFOCUS.md) are proposals; no smaller V1 release is approved. Historical ADRs remain unchanged.

## Baseline proposals not explicitly selected

| ID | Proposal | Evaluation point |
| --- | --- | --- |
| A-01 | x86_64 as the first package architecture | M0/M1; adjust to actual hardware |
| A-02 | One modular shell process and a separate CLI | M0; revise if profiling/isolation requires it |
| A-03 | Opt-in session-only text clipboard; session-only notification history | M3/M5; see ADR-0005 |
| A-04 | SCOPE feature minimums, including an external network editor | Before implementing the relevant module |
| A-05 | Single-file configuration; separate JSON state and local cache | M1 |
| A-06 | Initial UI language English and strings ready for localization; documentation English as requested | M1; UI language is not a user decision |
| A-07 | Neutral initial theme with example DESIGN color tokens | M1; final branding remains open |
| A-08 | No default HTTP/telemetry | All milestones |
| A-09 | Documented history/cache limits and budget hypotheses | M0 through M6, based on measurement |

Documentation language update: the user subsequently requested **English for all Markdown files and documentation**. This supersedes the original Indonesian-documentation proposal in A-06; the initial UI-language proposal and localization plan are unchanged.

## Open decisions

| ID | Decision | Owner / gate | Required evidence |
| --- | --- | --- | --- |
| O-01 | **Resolved 2026-11-05: GLES/EGL with glow; CPU path for tests** ([ADR-0006](adr/0006-renderer-criteria.md)) | Closed / M0 | Spike measurements in ADR-0006; wgpu rejected on tie-break |
| O-02 | Event loop, executor, worker model | Maintainer / M0 | Wayland integration, wakeups, shutdown, async |
| O-03 | Text stack and protocol libraries | Maintainer / M1 | Shaping, Unicode, font fallback, IME, licensing |
| O-04 | AT-SPI bridge and semantics coverage | Maintainer / M1 | Screen reader/focus spike and dependencies |
| O-05 | Minimum/tested Hyprland versions and hardware | Maintainer / M0 | Actual machine inventory and protocol globals |
| O-06 | Code, asset, and font licenses | Project owner / before public distribution | Owner selection and dependency license inventory |
| O-07 | Organization/repository name, logo, final palette | Project owner / before release | Selected identity |
| O-08 | AUR publication and official binaries | Maintainer / M6 | Tested recipe, reproducible build, distribution decision |

No package version, hardware, benchmark, Hayu repository, or license is considered approved merely because it appears as a candidate. Record decision changes with the date, reason, impact, and replacement ADR; preserve decision history.
