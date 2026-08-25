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
| `golden/` | a windowed GPU capture, once one has been taken | see below |

## The rule this folder runs on

Two of the three documents above are **generated**, and the census is checked by
GHOSTFENCE (`python tools/ghostfence/run.py --row census`). A hand-typed audit
rots in a week — `phase_MORROWIND.md` §4 was accurate on 2026-08-23 and was
already **27,329 lines out of date** when MORROWIND-A measured it on 2026-08-24,
because Phase CONTROL landed in between. The generated version cannot drift
without failing a gate.

## Golden images

`golden/` is empty and `golden/manifest.json` does not exist yet. That is the
honest state: golden references need a **windowed GPU run**, and GHOSTFENCE
reports the row as `SKIP` with the command attached rather than reporting a
green it did not earn.

To take a reference:

```bash
SOMNIUM_CAPTURE_UI_PNG="dev records/phase MORROWIND/golden/shell_1920.png" \
SOMNIUM_CAPTURE_FRAME=120 \
SOMNIUM_CAPTURE_QUIT=1 \
cargo run -p hello_engine
```

Then record it in `golden/manifest.json`:

```json
{
  "images": [
    {
      "name": "shell_1920",
      "reference": "shell_1920.png",
      "candidate": "target/ghostfence/shell_1920.png",
      "threshold": { "channel_tolerance": 2, "failing_fraction": 0.001, "max_channel": 24 }
    }
  ]
}
```

A failing comparison writes `target/ghostfence/<name>.diff.png`: the reference
darkened to a quarter, with every failing pixel highlighted in magenta scaled by
how far it moved. The point is that a failure tells you *where*, not just that a
number changed.

## Capture rule, inherited

**Captures after tonemapping.** The HDR target holds values far above one and a
PNG written from it directly is worthless as evidence. `SOMNIUM_CAPTURE_PNG`
writes the HDR target; `SOMNIUM_CAPTURE_DISPLAY_PNG` writes after tone
map/CAS/FXAA but before editor chrome; `SOMNIUM_CAPTURE_UI_PNG` writes the
finished window. For anything showing the editor, the last one is the only
correct choice.
