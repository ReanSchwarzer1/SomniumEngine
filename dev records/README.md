# Development records

Validation images live here instead of the repository root. Records are grouped
by project phase, then by the phase range that produced them. Filenames begin
with the phase identifier and describe the validated viewpoint or render stage.

**Start-here handoffs**

- [`post_IV_context_handoff.md`](post_IV_context_handoff.md) — start-here history after Phase IV. **XV-A–J complete.** Live numbers: [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md).
- [`post_25M2_context_handoff.md`](post_25M2_context_handoff.md) — historical post-25M-2 / Phase IV A–J narrative; superseded as the XV entry point by the post-IV handoff.

**Evidence folders**

- `phase IV/IV-D-E/` — finite-water and surface-optics evidence.
- `phase IV/IV-F-G-H/` — spectral surface, underwater transition, and shared
  default-landscape evidence.
- `phase IV/IV-I-J/` — vessel, shoreline LOD, and contact-band evidence.
- `phase IV/IV-K/` — ocean fidelity pass: the surface before and after the
  shading rewrite, and the authored water body that ships.
- `phase 26/` — inspector colour-picker evidence (planned; see `phase_26.md`).
- `phase VV/` — ray-traced water reflection evidence (planned; see
  `phase_VV.md`).
- `phase XV/` — terrain evidence (XV-A–J **complete**).
  Path: `phase XV/evidence/phase_XV-<subphase>_<purpose>.png`.
  Record: [`phase XV/evidence/XV-J_compile_gate.md`](phase%20XV/evidence/XV-J_compile_gate.md).
  Live contract: [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md).
  Plan: [`phase_XV.md`](phase_XV.md).
  XV-A provenance audit: [`phase XV/XV-A_research.md`](phase%20XV/XV-A_research.md).
  Codebase map: [`phase XV/XV-A_codebase_map.md`](phase%20XV/XV-A_codebase_map.md).

Captures must be taken after tonemapping. The renderer's HDR target holds
values far above one, so a PNG written from it directly is uniformly blown out
and worthless as evidence.

These images are engineering evidence, not runtime assets.
