# ADR-0001 — Internal Rust-based UI foundation

Date: November 2, 2026. Status: **user-decided**.

## Context and decision

The user chose an internal UI foundation after discussing toolkits and Noctalia. Build layout, components, interaction, focus, semantics, and renderer integration as an internal Rust system. GTK4/Iced are not the primary widget foundation. Wayland protocol, text shaping, image decoding, GPU, and D-Bus libraries remain permissible.

## Consequences

Hayu maintains text, scaling, input/IME, accessibility, caching, and resource lifetimes. Greater control does not automatically lower resource use. The internal API follows shell needs, not a promise of a public UI framework.

## Validation

M0 proves surfaces/rendering; M1 proves TextInput/IME, focus, and a screen reader spike. The renderer is decided through ADR-0004. Resolve foundational failures before expanding features.
