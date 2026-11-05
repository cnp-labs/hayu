# ADR-0005 — Bounded local state and session-only history

Date: November 2, 2026. Status: **initial proposal**, not explicitly selected by the user.

## Proposal

Separate TOML config, versioned JSON state, and regenerable cache. Write pinning/favorites, DND, and wallpaper selections atomically. Clipboard history is opt-in and text-only; clipboard and notification history remain in session memory with count/byte caps.

## Reasons and consequences

A history feature category does not automatically require disk persistence. Session-only storage reduces personal-content retention and recovery complexity, but history disappears after restart and must be explained. Sensitive content cannot always be detected; pause/clear must be easy. Clearing Hayu does not guarantee that other applications' clipboards are cleared. Without a reliable lock signal, do not claim automatic protection.

## Validation

M3/M5 check that payloads do not enter disk/logs, plus eviction, pause, clear, and teardown. Future persistence requires an ADR covering retention, deletion, threat model, and UX.
