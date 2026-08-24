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

## CONTROL-K — curve and gradient editing

Completed 2026-08-23. `Curve`, `Gradient` and `SliderCurve` are reflected value
types, so `FieldType::{Curve, Gradient}` travel through Seam 1 like a float
does. Two widgets, five presets, three live consumers. Record:
`CONTROL-K_curves.md`.

| Evidence | Result |
|---|---|
| Core value-type suite | `somnium_ecs` 55/55 green; 13 new tests cover clamping, segment shapes, NaN keys dropped before the sort, flat-form round trip, the exponential slider and the presets |
| Scene round trip | `scene_schema::curves_and_gradients_survive_a_save` — tangents and per-key interpolation modes survive the file |
| Forward compatibility | `an_absent_curve_field_loads_as_an_empty_curve` — a field added after a scene was written |
| Widget behaviour | 7 curve-editor and 4 gradient-editor tests, including "adding a stop does not change the ramp" and the linear→sRGB encode |
| Shader consumer | `PostParams.response`, 32 samples, re-sampled every frame — no refresh step exists to be found |
| Owed | draggable tangent handles; presets are keyboard-only |

## CONTROL-L — time of day

Completed 2026-08-23. Six scalars and five CONTROL-K tracks in one schema
block; the sun is analytic. Record: `CONTROL-L_time_of_day.md`.

| Evidence | Result |
|---|---|
| Solar position | 13 tests: equinox noon overhead at the equator, midnight below the horizon at three latitudes, the solstice favouring one hemisphere, east→west crossing, and **continuity across all 24 h** (no jump above 1.5° per 0.05 h) |
| Agreement with 25M | `a_high_sun_points_its_light_downward` — this module's rotation and `sun::transmittance` share an idea of "up" |
| Unauthored vs zero | `unauthored_tracks_report_nothing_rather_than_zero` |
| Reach | Details, a `HH:MM` context-bar scrub, six generated preset commands; `SOMNIUM_SUN_AZIMUTH`/`_ELEVATION` demoted to Seam 4 overrides |
| **Owed** | the four time-of-day captures, and the atmosphere-LUT cost per scrub frame — both need a windowed run |

## CONTROL-M — volumetric clouds

Completed 2026-08-23. Twenty-one parameters, one schema block, zero new
environment variables that are the only route to anything. Record:
`CONTROL-M_clouds.md`.

| Evidence | Result |
|---|---|
| Shader validation | `the_cloud_modules_validate` — the noise generators, the march concatenated after `atmosphere.wgsl`, and the composite all parse and validate under naga |
| Pass invariants | 9 tests: uniform alignment, the weather key regenerating on coverage and not on wind, a neutral shadow uniform when disabled, quarter-res never zero, wind wrapping at the weather period |
| Weather painter | 4 tests: raised-cosine falloff **exactly** zero at the rim, signed erase, per-channel isolation, and wrapping at the map's seam |
| Authoring surface | 15 `Sky` tests including "clouds are off until the number exists", the clamp of every value the renderer divides by, and the component/renderer defaults not drifting |
| Env routes | `SOMNIUM_CLOUDS` → `somnium.Sky.enabled`; `SOMNIUM_CLOUD_JITTER` → `editor.view.pipeline.cloud_jitter`. Audit unexplained count remains **0** |
| **Owed** | the `.somtime` row with and without jitter, the four preset captures, the fast-camera occlusion capture, the cloud-shadow capture — all need a windowed run. **The pass ships off until the row exists.** Also owed: the painter's viewport gesture, a cloud debug view |

## CONTROL-N — weather and wetness

Completed 2026-08-23. The coverage → precipitation → wetness chain, on
Lagarde's two time constants. Record: `CONTROL-N_weather.md`.

| Evidence | Result |
|---|---|
| Wetness model | 11 tests: framerate independence (10×0.1 s vs 100×0.01 s within 1e-3), specular recovering before diffuse, wetting faster than drying, snow leaving the ground dry, puddles lagging the film, a drizzle never soaking the world |
| The one preset | `every_weather_preset_names_a_sky` — `editor.weather.storm` closes the sky as well, in one `CommandGroup` undo entry |
| Wind | one vector, three consumers (cloud advection, ocean spectrum, precipitation shear). Foliage sway does not exist in this engine, so there was nothing to unify |
| Material channel | `porosity` in `GpuMaterial`'s existing padding — struct size and every offset unchanged; material schema 16 → 17 fields |
| Shader validation | `the_shading_module_validates` and the water modules, with `apply_wetness` and `rain_ripple_slope` in tree |
| **Owed** | the capture sequence; occlusion fade under cover, which wants MORROWIND-P's GPU particles rather than a readback |

## CONTROL-O — deferred decals

Completed 2026-08-23, stretch and all. It ships with the drag gesture, which
was its stated exit condition. Record: `CONTROL-O_decals.md`.

| Evidence | Result |
|---|---|
| Clustering reuse | `cluster.rs` grew a `ClusterVolume` trait; the counting sort is generic and its 12 existing tests are unchanged. One binning algorithm, tested once |
| GPU layout | 4 tests: 128 bytes and 16-aligned, the bounding sphere as the half-**diagonal**, the inverse transform mapping the box onto the unit cube, angle fade stored as a cosine |
| Placement | 4 core tests: floor and wall orientation, a degenerate normal not producing NaN, a default box with real projection depth |
| The gesture | `every_route_resolves_to_exactly_one_semantic_request` — the `Alt` route, the non-`Alt` route through the *same* drop, and the "point at terrain" refusal. Renamed from `seven_routes_…`; there are eight |
| Shader validation | the decal loop parses and validates; `decal_params.x == 0` skips it entirely |
| **Owed** | captures; mesh-surface drops; a decal debug view; emissive decals |

## Phase close

All twenty sub-phases A–O are in tree as of 2026-08-23. The reachability audit
regenerates clean: **108** `SOMNIUM_*` identifiers, **zero** unexplained,
**23** reflected schemas, **231** editable fields all credited to the generated
inspector, and a hand-wiring census of **0**.

The whole workspace test suite is green (`cargo test --workspace --lib -j 1`,
plus `cargo test -p somnium_renderer --test shaders_validate`). `-j 1` is not
optional on this checkout — parallel linking hits OneDrive file locks, which
present as `LNK1104` and are not a code failure.

What Track 2 and Track 3 owe is **evidence, not implementation**: every capture
and every `.somtime` row needs a windowed GPU run. Until those rows exist the
cloud pass, the weather driver and the day cycle all ship **off**, which is the
arrangement §12 asks for and the reason no existing scene changes.

## Live-session defect pass

Five defects reported from a running editor, all with green tests over them.
Record: `CONTROL-P_live_session_fixes.md`.

| Defect | Cause | Fix |
|---|---|---|
| Fly-cam ignored WASD | a right-press over the viewport never cleared UI keyboard focus, and the CONTROL-K editors claimed the keyboard unconditionally | `release_keyboard()` on viewport right-press (modals exempt); the curve/gradient editors claim keys only while something is selected |
| …and `S` still lagged 2–3 s | bare `S` is bound to the Scale tool, so the shortcut dispatcher ate the press; the `!repeat` guard meant OS key-repeat then fell through, which is why it started moving exactly when auto-repeat did | `viewport_camera_active` latch from right-press to release; the dispatcher stands down while the fly-cam is driving. Cleared on a release anywhere, and on window focus loss |
| Snap controls did nothing | `attach_combo_popup` was never called on the two snap combos, so no dropdown existed to emit `SelectionChanged` — the handlers were correct and unreachable | attach both, widen `combo_entries()`; also send the "snap" query to the Preferences search box |
| Cancel chip blinked at ~3 Hz | the asset inventory job resubmitted on a 350 ms timer whether or not anything changed | gate on a content-root stamp (as a guard, not an early return); `JobSnapshot` carries priority and the chip skips `Background`; `update_jobs` is idempotent and restores "Ready" |
| Clouds blocky | the march divided the *whole ray* by `max_steps`, so a shallow ray took kilometre-long steps through a 2 km slab | the step is a distance in metres from the layer thickness; cost bounded by an iteration cap and the early-out |
| Clouds shimmered | temporal jitter with nothing to average it — TAA has no motion vector for a sky pixel | frame-stable jitter by default; `temporal_jitter` is opt-in |

Plus `Sky ▸ Clouds ▸ Cloud Quality` (Quarter / Half / Full resolution), because
step counts cannot buy pixels the buffer does not have; and the vestigial
`post_tonemap_combo`/`post_tonemap_popup` pair removed.

New tests: `a_right_press_on_the_viewport_releases_the_keyboard`,
`a_modal_keeps_the_keyboard_through_a_right_press`,
`a_snapshot_reports_the_priority_it_was_submitted_with`,
`quality_selects_the_march_resolution`,
`quality_reaches_details_as_a_named_choice`,
`temporal_jitter_is_off_by_default`,
`the_fly_cam_owns_the_keyboard_while_right_mouse_is_held`,
`a_release_outside_the_viewport_still_ends_the_fly_cam`,
`losing_window_focus_ends_the_fly_cam`.
