# ADR-0004 — Renderer selection through prototyping

Date: November 2, 2026. Status: **proposed; no backend selected**.

## Candidates and experiment

GLES/EGL and wgpu are candidates, not pinned dependencies. A CPU renderer for testing may be evaluated without requiring another production backend. Use the same scene: bar shapes/clips/text, popup, image, input, and scale changes.

Measure startup, idle, text quality, resize/hotplug, GPU/driver compatibility, PSS/GPU allocations, dependencies, and Wayland integration effort. Record the font stack and presentation mode for a fair comparison.

## Criteria and results

Surface/lifecycle correctness comes first, followed by hardware compatibility, resources, debugging, maintenance, and licensing. Do not choose GLES merely because Noctalia uses it. M0 produces a report with commit/config/environment, samples, limitations, and a recommendation. Change status to accepted after the maintainer decision; record rejection reasons for alternatives. No benchmark results exist yet.
