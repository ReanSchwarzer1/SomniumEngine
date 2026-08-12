# Development records

Validation images live here instead of the repository root. Records are grouped
by project phase, then by the phase range that produced them. Filenames begin
with the phase identifier and describe the validated viewpoint or render stage.

- `phase IV/IV-D-E/` — finite-water and surface-optics evidence.
- `phase IV/IV-F-G-H/` — spectral surface, underwater transition, and shared
  default-landscape evidence.
- `phase IV/IV-I-J/` — vessel, shoreline LOD, and contact-band evidence.
- `phase IV/IV-K/` — ocean fidelity pass: the surface before and after the
  shading rewrite, and the authored water body that ships.
- `phase VV/` — ray-traced water reflection evidence (planned; see
  `phase_VV.md`).

Captures must be taken after tonemapping. The renderer's HDR target holds
values far above one, so a PNG written from it directly is uniformly blown out
and worthless as evidence.

These images are engineering evidence, not runtime assets.
