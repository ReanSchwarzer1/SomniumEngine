# Development records

Validation images live here instead of the repository root. Records are grouped
by project phase, then by the phase range that produced them. Filenames begin
with the phase identifier and describe the validated viewpoint or render stage.

**Start-here handoffs**

- [`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md) — **current start-here.** A different model must learn the architecture (`context.md`, `ATTRIBUTION.md`, every markdown in this folder) then **audit Halcyon → HEAD** (VV, 24M–R, FSR 3, foliage LOD/cull). Phase 26 chrome is fine. Phase DF clipmaps are **in engine**; they need their **own** audit (`phase_DF.md` §12) before default-on — do not quietly retune them inside the Halcyon audit. Terrain shading occupancy (Island 30+ fps vs Coastal ~20 fps on the ground, compact PSO, do not flip uniforms expecting a drop): [`terrain_shading_occupancy_2026-08-14.md`](terrain_shading_occupancy_2026-08-14.md).
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
- `phase CR/` — Crysis occupancy. Plan: [`phase_CR.md`](phase_CR.md).
  Table: [`phase CR/CR-A_occupancy.md`](phase%20CR/CR-A_occupancy.md).
- `phase DOOM/` — optimization. **A, B, C, E, F in tree; D and G–M deferred.**
  Plan + status: [`phase_DOOM.md`](phase_DOOM.md) §15. Evidence and every
  number: [`phase DOOM/README.md`](phase%20DOOM/README.md). `.somtime` files are
  deterministic GPU timing runs with a stddev per row — **do not overwrite the
  `DOOM-A_*` baselines.** Headline: Frame 38.4 → 19.9 ms with dynamic
  resolution; tile binning and the aerial terrain pipeline are built, correct
  and default **off** because both measured slower.
- `phase PORTAL/` — **not created yet.** Phase PORTAL (Source) is a
  **plan only**, nothing in tree: [`phase_PORTAL.md`](phase_PORTAL.md). It is
  the engineering-health phase — CI gates that can actually fail, one lint
  policy, a one-command capture+`.somtime` parity harness, a Source-style
  `ConVar` registry for the 96 `SOMNIUM_*` variables, the seven complex
  functions (`handle_editor_event` is **cyclomatic 381**), the `somnium_ui` and
  physics test holes, and the open defects in `context.md` §18.
  **PORTAL-D migrates this folder's durable content into `context.md`** —
  measured tables, negative results, frozen contracts — because **this folder
  stays out of version control** and everything in it lives on one machine.
  Migration is a *distillation*, not a concatenation: 40 markdown files /
  17 387 lines against a `context.md` of 4 594. Retention rule and the
  `docs/context/` fallback: `phase_PORTAL.md` §5.10.
  **PORTAL-A creates the evidence folder**; do not invent PNGs before it runs.
  It adds no rendering or editor feature and its success condition includes the
  frame time **not moving**. Sequencing vs Phase CONTROL: §7.
  **`rust-doctor` was run on 2026-08-18** — score 65/100, `authoritative:
  false`, 3 files its parser could not read, and a default gate that evaluates
  to `not-evaluated`. Read §2.1 before trusting the number.

- `phase CONTROL/` — **not created yet.** Phase CONTROL (Northlight) is a
  **plan only**, nothing in tree: [`phase_CONTROL.md`](phase_CONTROL.md). It is
  the editor-reach phase — the reflection-driven Details seam, an asset database
  with thumbnails, material authoring, drag and drop, preferences, the scene-load
  fix, then volumetric clouds, time of day and weather. **CONTROL-A creates the
  evidence folder**; do not invent PNGs before it runs.
- `phase KENSHI/` — **not created yet.** Phase KENSHI (OGRE) is a **plan only**,
  nothing in tree: [`phase_KENSHI.md`](phase_KENSHI.md). It is the **scale**
  phase, and it is third: CONTROL, then MORROWIND, then this. Premise: after
  MORROWIND every feature will have been accepted on a *single-feature*
  `.somtime` row, and **nobody will ever have run the frame that has crowds,
  streaming, particles and agents in it at once**. Four tracks — **THE HUB**
  (determinism, `.somtime` v2 with a scale axis, the sweep harness, the scale
  rig), **BEEP** (the profiler finished: CPU depth, memory, job queues,
  per-system attribution, capture-to-file, and a Panda3D-style *networked*
  client), **WORLD'S END** (the sweep, whose product is a publishable
  `limits.md`), and **SKELETON** (the fixes — multi-threaded recording via
  pipeline cycling, the pose task graph, virtual texturing).
  **Its judging rule is the no-speculative-fix rule: no optimisation is
  authorized until a measurement names it**, and every Track 3 sub-phase is
  marked `BLOCKED` until the sweep indicts it — on the DOOM precedent that a
  refusal on evidence (tile binning, aerial terrain) is worth more than a
  feature. **Read the tags in its §4:** unlike CONTROL and MORROWIND it cannot
  open with a measured audit, because the tree it measures does not exist yet,
  so half of §4 is labelled `[P]` for predicted and KENSHI-A's whole job is to
  replace those with `[M]`. Appendix A carries the determinism checklist, the
  v2 parser branch, the shape classifier and the sweep spec format.
- `phase MORROWIND/` — **not created yet.** Phase MORROWIND (NetImmerse) is a
  **plan only**, nothing in tree: [`phase_MORROWIND.md`](phase_MORROWIND.md). It
  is the engine-half phase — the runtime (game-facing) UI, skeletal animation,
  prefabs, the asset cook and world streaming, navigation, GPU particles and
  virtual shadow maps, input actions, save games, and an audio crate that is
  currently **93 lines with three one-line stubs**. Eight tracks, 36 sub-phases,
  and it **retires `context.md` §17.6's numbering** (§1.3 there): Phase 26
  shipped as the editor's information architecture and Phase 27 as its paint
  layer, so the §17.6 entries claiming those numbers for a UI framework and for
  animation no longer describe reality. **Gated on CONTROL-B and CONTROL-C
  being in tree**; §6.7 is the non-overlap table and it forbids this phase from
  building a curve editor, a gradient editor, a preferences window, time of day,
  clouds or weather — all CONTROL's. §9.3 gives the eleven-sub-phase cut if the
  whole phase cannot be run. **MORROWIND-A creates this evidence folder** and
  writes the census script; do not invent PNGs before it runs. §16 states which
  claims were verified by reading the tree on 2026-08-23 and which were not —
  notably that **no web-research pass completed**, so the plan carries no
  third-party version claims by design.
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
