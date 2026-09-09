# Phase MORROWIND — evidence

Created by **MORROWIND-A**, 2026-08-24, per `dev records/phase_MORROWIND.md`
§13. The plan says *"Do not invent PNGs"*, and that rule is the reason this
folder has a generator and a gate in it before it has a single image.

## What is here

| File | Produced by | Regenerate with |
|---|---|---|
| `MORROWIND-A_census.md` | `tools/census/generate.py` | `python tools/census/generate.py` |
| `MORROWIND-A_fyrox_diff.md` | MORROWIND-A, by hand from a module-by-module read | — |
| `MORROWIND-A_license_audit.md` | MORROWIND-A, by hand from the license files | — |
| `MORROWIND-A.md` | the sub-phase record | — |
| `MORROWIND-K.md` | shared graph surface and material compiler record | — |
| `MORROWIND-V.md` | clips, blend graphs, state machines and sync record | — |
| `MORROWIND-Q.md` | deterministic native asset cook, cache and resolver record | — |
| `MORROWIND-R.md` | budgeted residency, placeholders and cooked hot reload | — |
| `MORROWIND-S.md` | world partition, cell jobs and durable actor ownership | — |
| `MORROWIND-T.md` | HLOD/impostor cook and CPU floating-origin decision | — |
| `MORROWIND-AB.md` | portable SDF-backed DDGI and generated Details | — |
| `MORROWIND-AD.md` | terrain source-page streaming and runtime virtual texture | — |
| `MORROWIND-CS-CORRECTNESS.md` | hierarchy-correct gizmo translation and component-neutral viewport picking | — |
| `golden/` | a windowed GPU capture, once one has been taken | see below |

## 2026-09-09 implementation records

[O: prefabs](MORROWIND-O.md), [P: splines/blockout](MORROWIND-P.md),
[P2: scatter](MORROWIND-P2.md), [W: procedural animation](MORROWIND-W.md),
[W2: compression/jobs](MORROWIND-W2.md), [X: navigation](MORROWIND-X.md),
[Y: behavior/perception](MORROWIND-Y.md), [AF: saves](MORROWIND-AF.md).

## The rule this folder runs on

Two of the three documents above are **generated**, and the census is checked by
GHOSTFENCE (`python tools/ghostfence/run.py --row census`). A hand-typed audit
rots in a week — `phase_MORROWIND.md` §4 was accurate on 2026-08-23 and was
already **27,329 lines out of date** when MORROWIND-A measured it on 2026-08-24,
because Phase CONTROL landed in between. The generated version cannot drift
without failing a gate.

## Current visual evidence

Golden references now exist. PERSONA intentionally changed the shell; the
inherited image mismatch remains an acceptance item. Do not replace references
as part of unrelated feature work. The [2026-09-09 session record](MORROWIND-2026-09-09.md)
links new designer captures and the exact fast-gate result.

## Capture rule, inherited

**Captures after tonemapping.** The HDR target holds values far above one and a
PNG written from it directly is worthless as evidence. `SOMNIUM_CAPTURE_PNG`
writes the HDR target; `SOMNIUM_CAPTURE_DISPLAY_PNG` writes after tone
map/CAS/FXAA but before editor chrome; `SOMNIUM_CAPTURE_UI_PNG` writes the
finished window. For anything showing the editor, the last one is the only
correct choice.
