# References and usage boundaries

Reviewed for the November 2, 2026 baseline. Rolling documentation URLs may change; pin versions/commits actually used by implementation in compatibility reports and lockfiles.

November 7 technical refocus also audits [Amane at pinned commit `544cbfc`](https://github.com/MystiaFin/amane/tree/544cbfc23af37f84ac237cd9bec53173981772b3). See [AMANE.md](AMANE.md) for source links, suitable/unsuitable patterns and license boundaries. No code/assets were reused and no dependency added.

## Inspiration and reviewed sources

| Source | Purpose |
| --- | --- |
| [Noctalia repository](https://github.com/noctalia-dev/noctalia) | Feature categories and separation between shell and compositor |
| [Noctalia README snapshot](https://github.com/noctalia-dev/noctalia/blob/30dbf43426035112bf1542f84a29fd7360091de6/README.md) | Repository snapshot pointer at preparation time; the main-branch README was also reviewed during discussion |
| [Noctalia architecture](https://github.com/noctalia-dev/noctalia/blob/main/ARCHITECTURE.md) | Event-driven design, component boundaries, rendering, lifecycle; not Hayu's specification |
| [Noctalia documentation](https://docs.noctalia.dev/noctalia/) | Desktop experience mapping and native v5 / legacy v4 separation |
| [Hyprland IPC](https://wiki.hypr.land/ipc/) | Request socket, event socket, event format, connection lifecycle |
| [Desktop Notifications](https://specifications.freedesktop.org/notification/latest/) | D-Bus notification semantics, capabilities, actions, lifecycle |
| [Desktop Entry](https://specifications.freedesktop.org/desktop-entry/latest/) | Discovery, visibility, Exec field codes, D-Bus activation |
| [TOML specification](https://toml.io/en/) | Declarative configuration syntax |
| [Smithay Client Toolkit](https://docs.rs/smithay-client-toolkit/latest/smithay_client_toolkit/) | Candidate Wayland client library, not Hayu's widget toolkit |

Native C++/OpenGL ES Noctalia v5 provides inspiration; Hayu is not required to use the same renderer or state model. This package does not copy Noctalia source code, assets, palette, or documentation. Before adopting third-party code/assets, record their origin and license terms.

## Specifications to review during implementation

This is a verification backlog, not a claim that every API has been researched or implemented: Wayland core and wlr-layer-shell; fractional-scale and viewporter; text-input/IME; compositor-advertised data-control; StatusNotifierItem and DBusMenu; MPRIS; NetworkManager; UPower; PipeWire/WirePlumber or the selected compatibility interface; XDG Base Directory; icon theme lookup; AT-SPI.

Each adapter must record its upstream URL, dependency version, protocol revision, required capabilities, and supporting tests. An upstream fact in these documents does not mean Hayu is already compatible with that upstream.
