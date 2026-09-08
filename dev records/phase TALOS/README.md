# TALOS — scalability and small rendering cleanups

Implementation started 2026-09-06 from `d51e550` (`dev`). Scope narrowed by the
user on 2026-09-08: retain the scalability controls; defer the larger terrain,
GI, ocean and compute-shading program. Its 60 FPS target is not a requirement
for this smaller delivery. The pre-existing user edits to `phase_TALOS.md`
were preserved, with status/scope corrections added.

## Controls and behavior

Select **Camera** in the Outliner, then **Graphics Scalability** in Details:

| Choice | Scene dimensions relative to the viewport cap |
|---|---|
| Native | 100% |
| Balanced — default | 75% |
| Performance | Two-thirds, displayed as 67% |

FSR reconstructs to the unchanged output size. Other AA modes retain their
existing spatial blit. The toolbar resolution setting is a cap, applied before
scalability. Other lighting/material/effect controls remain independent.
Scalability participates in generic editing, undo and scene persistence. Older
Camera records without the new field receive Balanced. The launch override is
`SOMNIUM_GRAPHICS_SCALABILITY=native|balanced|performance` for newly spawned
cameras. Dynamic resolution stays optional and off by default; its floor is
relative to the fixed scale, and disabling it restores that fixed scale.

[Captured Camera controls](TALOS-AB_camera-controls.png).

## Small rendering changes

- An unchanged scene extent no longer reallocates all scene targets. Viewport
  cap changes and window changes now use the same sizing path, including a
  floating viewport.
- FSR follows the actual output surface independently of internal resolution.
- FSR output bindings are retained instead of recreating a bind group and two
  texture views every frame. Exposure/sharpness still update through the same
  uniform buffer.
- FSR reallocates render-sized resources only when render dimensions change,
  and display-sized resources only when display dimensions change. Both reset
  temporal history. At 2560×1392, changing only the scene scale now retains the
  two display HDR textures (54.375 MiB combined), rather than reallocating them.
  This is an allocation reduction, not a claimed additional FPS gain.
- `.somtime` adds output dimensions and nearest-rank p95. Capture audits add
  DPI, viewport rectangle, presentation mode, fixed/effective scale, scene time,
  camera and per-terrain cache state.

No WGSL shading algorithm or dependency was changed. The terrain cache remains
opt-in: the audit found normal/wetness/readiness and multiple-terrain ownership
issues that should be fixed before any default promotion. No new renderer,
quality framework, job system or controller was added.

## Measurements from 2026-09-06

RTX 5080 Laptop, Vulkan, NVIDIA 591.86, release; 180 warm-up + 300 measured
frames per run, 225–226 fresh GPU samples. Output/swapchain **2560×1392**, DPI
1.0, editor viewport rectangle **(54, 68, 2160, 1078)**, AutoVsync. The scene
covers the output with editor chrome overlaid; these dimensions are recorded
separately rather than calling the visible editor region 2560×1440.

All matched runs use one simulation step per rendered frame. At capture frame
480, the audits report time 8.000050 seconds. Walking pairs also match camera
position (-31.5, 24.696337, -3.5). FSR, DI/GI, GTAO and water RT were active;
terrain cache, hex and parallax were off. These are effective states, not new
optimizations attributed to TALOS.

| Workload / mode | Scene target | Mean GPU ms | Wall p95 ms | Wall p99 ms |
|---|---|---:|---:|---:|
| Coastal ground / Native 1 | 2560×1392 | 42.6660 | 45.7807 | See raw run |
| Coastal ground / Balanced | 1920×1044 | 27.3849 | 33.5807 | 36.1307 |
| Coastal ground / Performance | 1706×928 | 23.0061 | 28.3468 | 32.2313 |
| Coastal ground / Native 2 | 2560×1392 | 43.0301 | 46.0003 | 47.6983 |
| Coastal walking / Native | 2560×1392 | 44.0212 | 47.1170 | 48.8918 |
| Coastal walking / Balanced | 1920×1044 | 29.7318 | 34.1736 | 43.5022 |
| Island ground / Native | 2560×1392 | 30.7643 | 34.1092 | 35.6061 |
| Island ground / Balanced | 1920×1044 | 20.3590 | 23.2501 | 24.2852 |

The static order was Native → Balanced → Performance → Native. Native drift
was under 1%; compared with their mean, Balanced reduced GPU time about 36%
and Performance about 46%. Walking and Island Balanced reduced GPU time about
32% and 34%, respectively. These are GPU frame-time changes, not claims about
presented FPS. The later FSR allocation cleanup is not included in those gains.

`TALOS-A_before_coastal-ground.*` is the preliminary, wall-clock-simulation run
(44.7941 ms). It establishes the complaint but is not the matched control.
The `TALOS-AB_fixed_*`, `TALOS-AB_walk_*` and `TALOS-AB_island_*` files contain
the matched raw timings, display PNGs and effective-state audits.

Still captures preserve the visible scene composition with reduced fine detail.
The user accepted the scalability controls on 2026-09-08. This is not a claim
that every wetness/night/foliage/edit/teleport sequence has passed the original
full TALOS appearance contract. The walking rail is a deterministic level
camera route at 1.5 m/s, not a terrain-following player replay.

## Reproduce

Build with `cargo build --release -p hello_engine -j 1`, then:

```powershell
python "dev records/phase TALOS/capture.py" native coastal-ground --name native-check
python "dev records/phase TALOS/capture.py" balanced coastal-ground --name balanced-check
python "dev records/phase TALOS/capture.py" performance coastal-ground --name performance-check
python "dev records/phase TALOS/capture.py" native coastal-ground --name native-repeat
python "dev records/phase TALOS/capture.py" balanced coastal-ground --rail coastal-walk --name walking-check
```

The script refuses to overwrite evidence and clears inherited `SOMNIUM_*`
overrides for its child. It preserves the current user's normal editor settings;
the audit records the effective result. `SOMNIUM_TIME_FIXED_STEP=1` is a capture
mode, never the ordinary editor default. Wall/GPU measurement clocks stay real.

## Validation

The resumed `cargo test --workspace -j 1` completed on 2026-09-08: **2,264
passed, 0 failed, 2 ignored**, including documentation tests. The release
`hello_engine` build also passed. Changed Rust files pass rustfmt, and
`git diff --check` passes.

The live FSR resize stress capture passed after the allocation cleanup:

```powershell
python "dev records/phase TALOS/capture.py" balanced coastal-ground --resize-stress --name TALOS-tweak_resize-stress
```

This forces DRS down to its floor through repeated internal resizes, retaining
the 2560×1392 output. The final audit confirms 1286×698 internal resolution,
Balanced fixed scale 0.75, DRS enabled, and effective scale 0.5025. The captured
image was visually inspected; the run completed without GPU validation errors.
This checks resource lifetime and resizing, not a new steady-state speed claim.

GHOSTFENCE fast: **5 pass, 1 fail, 1 skip**. Census, pinned toolchain, shader
budget, one job system and no second system passed. The existing PERSONA
menu/toolbar/sculpt golden mismatch remains; reference images and comparison
thresholds were not changed. The fast gate reads the existing candidate and is
not a fresh TALOS UI comparison. Repository-wide rustfmt also reports existing
formatting differences in `terrain/foliage_paint.rs`; changed Rust files were
formatted separately.

## Context used

`context.md`, the TALOS/PORTAL-0/DOOM/TSUSHIMA development records, Graphify's
hub report, current source and rendering Git history informed the changes.
Ponytail (mandatory), Rust Pro, codebase-design and code-review guidance were
applied. One read-only audit agent was used before the user's no-agent request;
none were used after it. The local Bevy DLSS preparation code was inspected for
independent render/output resolution ownership; no reference-engine source was
copied and no new external dependency was introduced.
