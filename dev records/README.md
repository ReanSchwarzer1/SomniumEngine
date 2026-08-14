# Development records

Validation images live here instead of the repository root. Records are grouped
by project phase, then by the phase range that produced them. Filenames begin
with the phase identifier and describe the validated viewpoint or render stage.

**Start-here handoffs**

- [`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md) — **current start-here.** A different model must learn the architecture (`context.md`, `ATTRIBUTION.md`, every markdown in this folder) then **audit Halcyon → HEAD** (VV, 24M–R, FSR 3, foliage LOD/cull). Phase 26 chrome is fine. Phase DF clipmaps are **in engine**; they need their **own** audit (`phase_DF.md` §12) before default-on — do not quietly retune them inside the Halcyon audit.
- [`halcyon_context_handoff.md`](halcyon_context_handoff.md) — Phase VV (Halcyon) **VV-A–H in tree**. Remaining Halcyon work is live evidence / §11 timings. **Superseded as start-here** by the post-Halcyon audit handoff. Plan: [`phase_VV.md`](phase_VV.md).
- [`post_IV_context_handoff.md`](post_IV_context_handoff.md) — IV/XV history after Phase IV. **XV-A–J complete.** Live numbers: [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md). Superseded as start-here by the post-Halcyon audit handoff.
- [`post_25M2_context_handoff.md`](post_25M2_context_handoff.md) — historical post-25M-2 / Phase IV A–J narrative; superseded as the XV entry point by the post-IV handoff.

**Evidence folders**

- `phase IV/IV-D-E/` — finite-water and surface-optics evidence.
- `phase IV/IV-F-G-H/` — spectral surface, underwater transition, and shared
  default-landscape evidence.
- `phase IV/IV-I-J/` — vessel, shoreline LOD, and contact-band evidence.
- `phase IV/IV-K/` — ocean fidelity pass: the surface before and after the
  shading rewrite, and the authored water body that ships.
- `phase 26/` — Metaphor editor-chrome evidence (capture live; see `phase_26.md`). The UI phase remains open.
- `phase VV/` — ray-traced water reflection evidence (captures still open;
  do not invent PNGs). See [`phase VV/README.md`](phase%20VV/README.md) and
  [`phase_VV.md`](phase_VV.md).
- `phase DF/` — Daggerfall terrain-clipmap evidence. Plan + **audit brief:**
  [`phase_DF.md`](phase_DF.md) §12. Timings: [`phase DF/DF-A_timings.md`](phase%20DF/DF-A_timings.md)
  (stale vs current look). Default **off**.
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
