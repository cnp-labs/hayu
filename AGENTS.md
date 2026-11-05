# AGENTS.md — Hayu

Hayu is a Rust Wayland desktop shell for Hyprland. **This repository is a
documentation-only planning baseline: there is no source code, `Cargo.toml`,
build script, CI, or test suite yet.** Everything shipped so far is Markdown
plus one example config.

## Do not invent an implementation

- Do not run `cargo build`, `cargo test`, `cargo fmt`, or any linter; there is
  nothing to build and no toolchain is pinned.
- Do not scaffold crates, CI, or packaging unless asked. M0 (ROADMAP.md) must
  first settle the open decisions O-01..O-05 (renderer, event loop, text stack,
  AT-SPI bridge, minimum Hyprland version).
- Never write that something is implemented, tested, benchmarked, installable,
  or supported. README, TESTING.md, and PERFORMANCE.md explicitly record that
  no implementation, hardware test, or measurement exists. All CLI names,
  socket paths, protocol usage, and performance figures are proposals.

## Documentation is normative — preserve its status labels

- `DECISIONS.md` is the source of truth for status: **D-01..D-09 user-decided**,
  **A-01..A-09 proposals pending evaluation**, **O-01..O-08 open decisions**.
  Never promote a proposal or an open question to a fact.
- Requirements are `HAYU-001..016` (PRD.md), tests are `T-01..T-20`
  (TESTING.md). Reference these IDs instead of inventing new numbering.
- "First release" means full V1 under M6, never the M0 prototype or a
  bar-only alpha.
- Foundational changes — UI foundation, config format, renderer, privacy/state
  model — require a new ADR in `docs/adr/` (next number: 0006). Feature
  changes require a PRD ID plus matching SCOPE, TESTING, and budget updates.
- Inspiration from Noctalia creates no feature-parity obligation, and no
  Noctalia code, assets, or palette may be copied.

## Keep the docs consistent

A single change usually touches several files; update all of them in the same
commit:

- Config key added or changed → `CONFIGURATION.md` schema table **and**
  `examples/config.toml` (the example must cover every documented table).
- UI behavior or component → `DESIGN.md` plus the relevant `ARCHITECTURE.md`
  surface/focus rules.
- Capability or fallback → `COMPATIBILITY.md`; new measurable target →
  `PERFORMANCE.md` (label it a hypothesis until measured).
- Milestone or sequencing change → `ROADMAP.md` and `DECISIONS.md`.

Locked boundaries: Hyprlock stays the user's locker (no internal lock screen),
no compositor/window management, no Lua/runtime scripting, no plugin
marketplace, no non-Arch distribution support in V1.

## Language and commit workflow (user-mandated)

- All Markdown files are **English**, even when the conversation is in
  Indonesian. Code identifiers and CLI names stay verbatim.
- Commit every change. **Split commits by logical unit** — one file group and
  one intent per commit, with a detailed English message (subject + bullets
  explaining what changed and which caveats still apply). Never one big commit.
- **Never `git push` and never open a PR without explicit approval.** Local
  commits only until the user says otherwise.

## Where to start reading

`README.md` (index and user decisions) → `PRD.md` (requirements) →
`SCOPE.md` (boundaries) → `DECISIONS.md` (decision status) →
`ROADMAP.md` (what comes next). Use `ARCHITECTURE.md` for module
responsibilities and `docs/adr/` for the reasoning behind locked choices.
