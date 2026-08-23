# Phase CONTROL evidence record

## CONTROL-A — reachability audit

CONTROL-A established the regenerable audit and the two deliberately red
CONTROL-B gates without changing a widget.

| Evidence | Result |
|---|---|
| Reachability/component/hand-wiring audit | `CONTROL-A_reachability.md`; regeneration and `--check` are green |
| Historical census reconciliation | The current tree measures 676, not the plan's 675: `IF::` in `app.rs` is now 202 rather than the 2026-08-22 baseline's 201 |
| `FieldFlags::EDIT` -> generated row gate | Red by design: 76 editable fields have no generated row |
| `FieldType` -> `PropertyEditor` gate | Red by design: `Entity`, `Asset`, and `Array` have no editor |
| Pre-change UI suite | 215 tests green (`cargo test -p somnium_ui -j 1`) before CONTROL-A1/A2 edits |
| Baseline capture | 14 editor states at both 1280x720 and 1920x1080; see `CONTROL-A_baseline/README.md` |
| `assets/terrain/` open timing | `CONTROL-A_terrain_open.somtime`: real 60-PNG folder, `cpu Frame wall` mean 157.3965 ms, maximum 1085.5605 ms across 89 intervals; GPU `Frame` mean 1.4905 ms |

### Reproduction

```powershell
python tools/reachability/generate.py
python tools/reachability/generate.py --check
python tools/reachability/generate.py --gate inspector # expected exit 1 until CONTROL-B
python tools/reachability/generate.py --gate editors   # expected exit 1 until CONTROL-B
$env:SOMNIUM_CONTROL_B_GATES='1'
python -m unittest tools.reachability.test_control_b_gates # expected two failures until CONTROL-B
Remove-Item Env:SOMNIUM_CONTROL_B_GATES
```

The opt-in environment guard keeps ordinary Python discovery and workspace CI
green while preserving executable red gates. CONTROL-B removes the failures;
it must not remove or weaken the tests.

The evidence-only startup controls and timing reproduction are documented in
`CONTROL-A_baseline/README.md`. The timing run used 90 frames, no warmup,
1280x720, `SOMNIUM_AUDIT_CONTENT_PATH=terrain`, and the shipped synchronous
thumbnail path. `Frame wall` was added to `.somtime` so the UI-thread decode
stall is measured rather than hidden behind renderer-only CPU/GPU scopes.

## CONTROL-A1 — input seam

Completed 2026-08-23. Modifier delivery, focus-into-view traversal, ordered
gesture cancellation and modal focus return are in tree. The UI suite is
225/225 green; the new tests cover Shift delivery, exact shortcut modifiers, long-tree traversal,
scrub restoration before popup dismissal, modal return and precision/snap.

## CONTROL-A2 — command registry

Completed 2026-08-23. One 52-command registry drives menus, Create/context
surfaces, toolbar, shortcuts, palette and the F1 command index. Stable IDs
replace positional dispatch; fuzzy free text, strict structured tokens and
persisted recency are tested. The core library suite is 128/128 green.
