# Hayu refocus verification plan

Starting commit: ab9e678204ca4224468368c5d1e044cf649b1413. Initial working tree clean. No push or PR authorized for this task.

## Risk and evidence

- Preserve existing launcher, tray, workspace left click, IME, AT-SPI tree, output lifecycle and preferred fractional scaling. No renderer replacement or product-feature removal.
- Documentation move: resolve every local Markdown target, search old paths, preserve ADR history; no build needed for prose alone.
- Dependency trimming/spike isolation: inspect active features and actual imports first. Build default release and retained CPU/GL/wgpu tooling explicitly. Test CPU reference callers before changing their gate. Lockfile stays tracked; no unrelated upgrades.
- Runtime structural changes: full unit tests after reconciled changes (foundation); compile all retained targets, Clippy with explicit feature selection, rustfmt only touched Rust files (never whole-crate cargo fmt). Formatting-only drift outside the intended lane was reverted during reconciliation; final checks ran after that cleanup. Executed results: [REFOCUS_VALIDATION.md](REFOCUS_VALIDATION.md).
- Runtime behavior changes: one deterministic regression test per scheduling/caching behavior. Reuse relevant existing live input, reload, pointer, tray and accessibility suites after final writers reconcile; capture commands, status and logs. IME and fractional live coverage must be separately described, never inferred from ordinary suites.
- Measurements: baseline and after from same script/config/environment with repeated runs, no user config mutations and SIGTERM cleanup. CPU ticks/context switches do not measure hardware wakeups. Debug frame time excludes compositor/input delivery. Warm build time is not clean build time. Missing GPU counters remain unavailable, not zero.

Do not repeat passed checks without new changes invalidating them. Nonblocking missing hardware/measurement coverage is documented backlog.
