# Refocus validation — November 7, 2026

Source revision: `740569bdac141f68269df6620c5b0148159b9396`. Earlier logical commits: documentation navigation `4059abe`, experiment isolation `281fb65`, tray bounds/identity `5fa0940`, zbus/Wayland features `23f7016`. Starting tree at `ab9e678` was clean. Local commits only; no push/PR.

## Build and deterministic checks

| Command | Exit | Evidence |
| --- | --- | --- |
| `cargo test --offline --all-features` | 0 | 64 passed, 0 failed; `/tmp/opencode/hayu-final-checks/test-all-features.log` |
| `cargo clippy --offline --all-targets --all-features` | 0 | 22 library warnings, 29 library-test warnings including duplicates, 1 spike-bin warning; no errors or suppressions |
| `rustfmt --check --edition 2024 --config skip_children=true src/app.rs src/bar.rs src/launcher.rs src/lib.rs src/main.rs src/tray.rs` | 0 | Touched-file formatting clean |
| `rustfmt --check --edition 2024 --config skip_children=true src/render.rs` | 1 | Pre-existing formatting drift; no crate-wide reformat applied |
| `cargo build --release --offline --features gpu-spike --bin hayu-gpu-spike` | 0 | 71.6 s, 6,474,120 bytes |
| `cargo build --release --offline --features wgpu --bin hayu-gpu-spike` | 0 | 158.7 s, 12,885,840 bytes |
| `cargo build --release --offline` | 0 | 120.9 s after feature switching, 13,373,488-byte main binary, no warnings |

Default tests had already passed 61/61 before final all-feature run. Three extra gated tests explain 64, not a lost test. Full build logs, command statuses and reports are under `/tmp/opencode/hayu-final-checks/` on the measurement host. Paths are local evidence, not portable CI artifacts.

CPU, GL and wgpu offscreen spike runs each used `--mode offscreen --frames 10 --json` and exited 0. Respective p50 times: 0.532, 0.154 and 0.197 ms. These short runs establish retained tooling functionality only, not renderer reselection or V1 performance superiority.

Clippy warnings include pre-existing style/API suggestions and touched tray/bar style suggestions; not all are pre-existing. They remain visible rather than triggering broad unrelated edits. `render.rs`'s new gates are narrow; its old formatting drift is recorded separately from touched-file formatter success.

`./target/release/hayu doctor` and `./target/release/hayu config check examples/config.toml` both exit 0. Local Markdown link resolution and `git diff --check` pass after documentation relocation.

## Live checks

Each suite ran once against the final default release with the current Hyprland signature, under a timeout. Exact logs/statuses: `/tmp/opencode/hayu-comparison/live/`.

| Command | Exit | Result |
| --- | --- | --- |
| `tools/live/input-suite.sh` | 0 | 15/15 |
| `tools/live/reload-suite.sh` | 0 | 42/42 |
| `tools/live/pointer-suite.sh` | 0 | 7/7 |
| `tools/live/tray-suite.sh` | 0 | 11/11, private bus and real fcitx5 |
| `python3 tools/live/a11y-suite.py` | 134 | Failed: missing `/run/user/1000/at-spi/bus_0`; AT-SPI client aborted |

The a11y failure is an environment limitation, not a passing tree check or evidence of a Hayu regression. Its abort bypassed harness cleanup; the owned shell was terminated and `org.a11y.Status IsEnabled` restored to its original `false`. Final state: no Hayu/fcitx5 process left and workspace 1 restored. No repair/retry was used to hide this failure.

The retained measurement probe ran before/after successfully; see [REFOCUS_MEASUREMENTS.md](REFOCUS_MEASUREMENTS.md). Its final automation status-exit addition passed `bash -n` plus focused success/failure/skip assertions; these do not invalidate runtime measurements because scenarios were unchanged.

## Coverage boundary

Unit/build success and ordinary live suites do not establish fractional-scale input, physical hotplug, IME composition or screen-reader usability. These were not rerun. Clock `%S`/reload cadence is unit-tested but not separately traced live. Existing tests must not be restated as a completed T-01–T-20 acceptance result or milestone closure.
