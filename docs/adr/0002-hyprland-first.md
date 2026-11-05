# ADR-0002 — Hyprland and Arch Linux as initial targets

Date: November 2, 2026. Status: **user-decided** for compositor/distribution.

## Decision

Workspace/window integration and initial tests use Hyprland. The first packaging target is Arch Linux or its derivatives. A compositor adapter centralizes IPC differences; widgets do not directly access sockets.

## Consequences and boundaries

Do not claim universal Wayland support. Record minimum and tested rolling versions. Snapshot/event reconciliation and reconnect belong to the adapter. x86_64 remains a candidate, not a fact about the user's hardware. Do not manage tiling/dotfiles. Hyprlock stays in the user's workflow; no internal V1 locker.

## Validation

M0 records hardware, versions, and protocol globals; M2 tests workspaces/windows/outputs; M6 tests packages and compatibility reports. Other compositors require new evidence.
