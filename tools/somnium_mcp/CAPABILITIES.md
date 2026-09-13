# Editor capability evidence

Generated from a running Hello Engine session by the main and workspace acceptance probes. These are native semantic edits, not an emulated editor. Full machine receipts remain in the ignored evidence directory.

## Live cases

| Family | Result | Evidence |
|---|---|---|
| animation state overlay | Pass | native_undo: True |
| graph topology | Pass | conflict_rejected: True, native_undo: True |
| timeline | Pass | markers: True, keys: True, media: True, native_undo: True |
| terrain and foliage | Pass | foliage_placed: 1, restored_count: 0 |
| luau source | Pass | compile_validation: True, published_undo: True |
| ui layout document | Pass | native_schema: True, published_undo: True |
| hello shared interaction preset | Pass | actions: 5, scene_undo: True |
| hello gpu animation | Pass | imported_clip: Bend, gpu_frames: 26, scrub: True, native_details: True, removed_allocations: True |
| spatial | Pass | pick_hits: 0, capsule_sweep: True, bookmark_recall: True |
| settings | Pass | query succeeded |
| script attachments | Pass | attach: True, exported_number: True, enabled: True, reorder: True, reload: True, native_undo: True |
| behavior compile apply | Pass | native_undo: True |
| navigation bake | Pass | polygons: 64, terminal_status: Ready — 64 polygons, 4 tiles, 1 unsupported meshes, clear: True |
| material native draft | Pass | reflected_edit: True, native_undo: True |
| renderer controls | Pass | native_ranges: 14, value_change: True, stale_value_rejected: True, restored: True |
| foliage controls | Pass | native_ranges: 7, custom_slope_survives_stroke: True, restored: True |
| positive pick | Pass | native_bounds_hit: True, distance_metres: 4.0 |
| content operations | Pass | create_folder: True, create_script: True, rename: True, create_material: True, assign_material: True, make_unique: True, assignment_undo: True |
| localisation | Pass | native_cells: True, range: True, native_undo: True, saved_catalogue: True, exported_csv: True, source_restored: True |
| native panels | Pass | floating_window: True, requested_size: [480, 600], dock_restored: True |

## Declared semantic operations

The following operations come from the live discovery response. A passing family case does not mean every individual operation or screen gesture was replayed.

| Owner | Operations |
|---|---|
| content | `new_folder`, `new_script`, `new_material`, `rename`, `assign_material`, `make_unique` |
| designer | `navigation_bake`, `navigation_clear`, `behavior_edit`, `animation_rig`, `animation_reset`, `animation_reload`, `animation_events`, `animation_save_events`, `save_play`, `load_play` |
| evidence | tools/somnium_mcp/editor_acceptance.py generates live case receipts and inventory; metadata describes routes, not a claim that every decorative UI gesture was replayed |
| graph | `add_node`, `literal`, `select`, `move`, `connect`, `disconnect`, `comment`, `group`, `delete`, `copy`, `paste`, `align`, `replace`, `undo`, `redo` |
| localisation | `cell`, `range`, `replace`, `sort`, `filter`, `undo`, `redo`, `save`, `export` |
| materials | material_open -> reflected native material draft -> material_save; native dirty/GPU path |
| preferences | settings query + setting with expected_value; stored preference has no scene undo |
| project | native File > Open Project or open_project; game launcher preserves private schemas |
| renderer controls | terrain_settings / foliage_settings query and native value updates with expected_value; session preview scope |
| scene | typed plan/commit, reflected Details, persistent IDs, one transaction undo |
| scripts | `attach`, `detach`, `reorder`, `enabled`, `number`, `bool`, `reload` |
| spatial | `camera`, `bookmark` |
| state machine | `state_initial`, `state_add_transition`, `state_set_transition`, `state_remove_transition`, `state_undo`, `state_redo` |
| terrain | `terrain_stroke`, `foliage_stroke` |
| timeline | `add_group`, `add_track`, `add_media`, `move_media`, `resize_media`, `add_marker`, `move_marker`, `add_key`, `move_key`, `remove_track`, `select_channel`, `replace`, `scrub`, `undo`, `redo` |
| workspace | `float`, `dock`, `place` |

Scene component schemas, commands, source validators and runtime diagnostics are discovered from the current editor. Preferences retain their own persistence semantics. Native picking tests transformed mesh/proxy bounds; positive ray distance and clear-capsule results are recorded above. Clearance considers registered physics colliders. Renderer preview knobs and brush settings restore their previous values; source files are separate from scene undo.

See [designer controls and contracts](../../docs/editor/automation.md). Rerun after changing the relevant owner or adapter:

```powershell
python -B tools/somnium_mcp/editor_acceptance.py --connection target/hello-editor/runtime/authoring-connection.json --output target/hello-editor/semantic-acceptance.json
python -B tools/somnium_mcp/editor_gap_acceptance.py --connection target/hello-editor/runtime/authoring-connection.json --output target/hello-editor/semantic-acceptance.json
python -B tools/somnium_mcp/generate_capabilities.py --source target/hello-editor --output tools/somnium_mcp/CAPABILITIES.md
```
