# Runtime measurement probe

Run from the repository with a live Hyprland session:

```sh
cargo build --release
HAYU_SKIP_BUILD=1 tools/perf/measure.sh
```

Needs `hyprctl`, `jq`, `python3`, D-Bus tools and a built `tools/live/keyin/target/release/keyin` for typing checks. Uses a temporary config/private session bus; does not load or overwrite the user's configuration. Maps a real bar and launcher briefly, then stops its owned shell. Do not run alongside another Hayu on the same session control socket.

Artifacts default to `/tmp/opencode/hayu-baseline/measure-<timestamp>/`. Set `HAYU_BASELINE_ART` to select a directory, `HAYU_BIN` for a preserved binary and `HAYU_BASELINE_HEAD` for its source revision. Without `HAYU_SKIP_BUILD=1`, the probe preserves the existing default binary then performs a warm build. `HAYU_TRAY_SMOKE=1` opts into the private-bus tray behavior suite, not a resource measurement.

Three 10-second idle samples measure process CPU ticks, RSS/PSS, threads and main-thread context switches. Context switches are not wakeups. Layer polling gives a coarse observation upper bound, not presentation latency; debug draw timings are not input latency. Report skipped/failed scenarios, shared-machine load and binary hashes. Recorded failures return nonzero; unavailable probes may be explicitly skipped. This is a short prototype probe, not the full V1 performance method in [PERFORMANCE.md](../../docs/PERFORMANCE.md).
