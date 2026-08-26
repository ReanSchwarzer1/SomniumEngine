"""CONTROL-H — how each `SOMNIUM_*` variable is reachable without a shell.

CONTROL-A's table proved the gap; this table closes it, and it is checked-in
data rather than a heuristic so the answer for every variable is a sentence
somebody wrote and a test can verify.

Four routes:

``schema``
    The variable seeds a registered component field, which CONTROL-B's
    generated Details panel already edits. The value is the schema address, and
    the generator asserts the field exists.

``setting``
    A Seam-4 setting with a declared environment override. The value is the
    settings-schema address; the generator asserts it appears in
    ``settings.rs``'s ``ENV_OVERRIDES``, so the Preferences window really can
    show the "overridden by …" reason.

``command``
    A registered editor command. The value is the command id, and the generator
    asserts the id exists in the CONTROL-A2 registry.

``harness``
    Deliberately not reachable from the editor, with a stated reason. Capture
    runs, timing runs, headless startup drivers and one-shot import switches
    belong here: they are arguments to a *process*, not properties of a scene,
    and giving them a control would be a lie about what they do.

Nothing may be absent. The gate in ``test_control_b_gates.py`` fails on any
variable this file does not name, which is CONTROL-H's exit condition — "the
table has no unexplained rows" — expressed as a build failure.
"""

# route, target-or-reason
ENV_ROUTES: dict[str, tuple[str, str]] = {
    # ── Post-processing and rendering: reflected component fields ───────────
    "SOMNIUM_ANALYTIC_GRAD": ("schema", "somnium.PostProcess.analytic_grad"),
    "SOMNIUM_BLOOM": ("schema", "somnium.PostProcess.bloom_enabled"),
    "SOMNIUM_CAS": ("schema", "somnium.PostProcess.cas_enabled"),
    "SOMNIUM_FSR": ("schema", "somnium.PostProcess.fsr_enabled"),
    "SOMNIUM_GTAO": ("schema", "somnium.PostProcess.gtao_enabled"),
    "SOMNIUM_LIGHT_SHAFTS": ("schema", "somnium.PostProcess.light_shafts"),
    "SOMNIUM_MESH_SDF": ("schema", "somnium.PostProcess.mesh_sdf"),
    "SOMNIUM_MOTION_BLUR": ("schema", "somnium.PostProcess.motion_blur_enabled"),
    "SOMNIUM_PATH_TRACER": ("schema", "somnium.PostProcess.path_tracer"),
    "SOMNIUM_PROBES": ("schema", "somnium.PostProcess.probes"),
    "SOMNIUM_RESTIR": ("schema", "somnium.PostProcess.restir_enabled"),
    "SOMNIUM_RESTIR_GI": ("schema", "somnium.PostProcess.restir_gi_enabled"),
    "SOMNIUM_RT_REFLECT": ("schema", "somnium.PostProcess.rt_reflect_enabled"),
    "SOMNIUM_RT_REFRACT": ("schema", "somnium.PostProcess.rt_refract_enabled"),
    "SOMNIUM_SPECULAR_GI": ("schema", "somnium.PostProcess.specular_gi"),
    "SOMNIUM_TAA": ("schema", "somnium.PostProcess.taa_enabled"),
    "SOMNIUM_VOLUMETRICS": ("schema", "somnium.PostProcess.volumetrics_enabled"),
    # CONTROL-M. The switch is a schema field; the variable is Seam 4's
    # override of it, and it wins so recorded captures keep working.
    "SOMNIUM_CLOUDS": ("schema", "somnium.Sky.enabled"),
    # CONTROL-M. Toft & Bowles' jitter, as a named pipeline toggle so the
    # with-and-without .somtime row is two runs rather than two builds.
    "SOMNIUM_CLOUD_JITTER": ("command", "editor.view.pipeline.cloud_jitter"),
    "SOMNIUM_WORLD_CACHE": ("schema", "somnium.PostProcess.world_cache"),
    "SOMNIUM_CPU_FRUSTUM": ("schema", "somnium.CameraSettings.frustum_cull"),
    "SOMNIUM_DYNRES": ("schema", "somnium.CameraSettings.dynamic_resolution"),
    "SOMNIUM_DYNRES_TARGET_MS": ("schema", "somnium.CameraSettings.dynamic_target_ms"),
    "SOMNIUM_DYNRES_FLOOR": ("schema", "somnium.CameraSettings.dynamic_floor"),
    "SOMNIUM_WATER_SPECTRUM": ("schema", "somnium.Water.spectrum_blend"),
    "SOMNIUM_FOLIAGE": ("schema", "somnium.Foliage.enabled"),

    # ── Seam 4 settings, with a declared override ──────────────────────────
    "SOMNIUM_CONTENT_ROOT": ("setting", "somnium.ProjectSettings.content_root"),
    "SOMNIUM_THUMBNAIL_BUDGET_MS": ("setting", "somnium.ProjectSettings.thumbnail_budget_ms"),
    "SOMNIUM_FLOAT_STEP": ("setting", "somnium.ProjectSettings.default_float_step"),
    "SOMNIUM_STARTUP_SCENE": ("setting", "somnium.ProjectSettings.startup_scene"),
    "SOMNIUM_SNAP_TRANSLATE": ("setting", "somnium.EditorSettings.snap_translate_m"),
    "SOMNIUM_SNAP_ROTATE": ("setting", "somnium.EditorSettings.snap_rotate_deg"),

    # ── Debug visualisations and pipeline switches: CONTROL-G's view menu ──
    # Every id below is generated from `somnium_ui::debug`'s own tables, so a
    # renamed view is a build failure here rather than a menu entry that
    # quietly stops matching.
    "SOMNIUM_SHADOW_DEBUG": ("command", "editor.view.debug.shadow_factor"),
    "SOMNIUM_CENSUS": ("command", "editor.view.pipeline.pixel_census"),
    "SOMNIUM_CULL_STATS": ("command", "editor.view.pipeline.cull_stats"),
    "SOMNIUM_SHADE_BINS": ("command", "editor.view.pipeline.shading_bins"),
    "SOMNIUM_TAA_DEBUG": ("command", "editor.view.pipeline.taa_debug"),
    "SOMNIUM_TAA_MATDBG": ("command", "editor.view.pipeline.taa_material_debug"),
    "SOMNIUM_RT_DEBUG": ("command", "editor.view.pipeline.rt_debug"),
    "SOMNIUM_PROFILE": ("command", "editor.view.profiler"),
    "SOMNIUM_NO_MESHLETS": ("command", "editor.view.pipeline.meshlets"),
    "SOMNIUM_NO_OCCLUSION": ("command", "editor.view.pipeline.occlusion"),
    "SOMNIUM_CASCADE_CULL": ("command", "editor.view.pipeline.cascade_cull"),
    "SOMNIUM_SPD": ("command", "editor.view.pipeline.spd"),
    "SOMNIUM_AERIAL": ("command", "editor.view.pipeline.aerial"),
    "SOMNIUM_AERIAL_HERO": ("command", "editor.view.pipeline.aerial_hero"),
    "SOMNIUM_HEXTILE": ("command", "editor.view.pipeline.hex_tiling"),
    "SOMNIUM_TERRAIN_CLIPMAP": ("command", "editor.view.pipeline.terrain_clipmap"),
    "SOMNIUM_TERRAIN_TRIPLANAR": ("command", "editor.view.pipeline.terrain_triplanar"),
    "SOMNIUM_TERRAIN_HEIGHT_BLEND": ("command", "editor.view.pipeline.terrain_height_blend"),
    "SOMNIUM_TERRAIN_PARALLAX": ("command", "editor.view.pipeline.terrain_parallax"),
    "SOMNIUM_TERRAIN_MACRO": ("command", "editor.view.pipeline.terrain_macro"),
    "SOMNIUM_TERRAIN_DETAIL_FADE": ("command", "editor.view.pipeline.terrain_detail_fade"),
    "SOMNIUM_LOD_MORPH": ("command", "editor.view.pipeline.terrain_lod_morph"),
    "SOMNIUM_RT_TERRAIN": ("command", "editor.view.pipeline.rt_terrain"),

    # ── Startup and capture harness: arguments to a process ────────────────
    "SOMNIUM_AUDIT_CONTENT_PATH": ("harness", "CONTROL-A capture driver: which drawer folder to open."),
    "SOMNIUM_AUDIT_LOG": ("harness", "CONTROL-A capture driver: where the renderer audit log is written."),
    "SOMNIUM_AUDIT_SELECT_ENTITY": ("harness", "CONTROL-A capture driver: which entity to select before capture."),
    "SOMNIUM_AUDIT_UI_STATE": ("harness", "CONTROL-A capture driver: which editor surface to open before capture."),
    "SOMNIUM_AUDIT_WINDOW_SIZE": ("harness", "CONTROL-A capture driver: the exact logical size to capture at."),
    "SOMNIUM_AUDIT_YAW_JUMP_DEGREES": ("harness", "Recorded fast-camera repro: the yaw step to inject."),
    "SOMNIUM_AUDIT_YAW_JUMP_FRAME": ("harness", "Recorded fast-camera repro: the frame to inject it on."),
    "SOMNIUM_CAPTURE": ("harness", "Capture harness: enables the offscreen capture path."),
    "SOMNIUM_CAPTURE_AFTER_TAA": ("harness", "Capture harness: which pipeline stage to read back."),
    "SOMNIUM_CAPTURE_AFTER_WATER": ("harness", "Capture harness: which pipeline stage to read back."),
    "SOMNIUM_CAPTURE_COMPARE": ("harness", "Capture harness: the reference image to diff against."),
    "SOMNIUM_CAPTURE_DISPLAY_PNG": ("harness", "Capture harness: output path for the display-referred image."),
    "SOMNIUM_CAPTURE_FRAME": ("harness", "Capture harness: which frame to capture."),
    "SOMNIUM_CAPTURE_PNG": ("harness", "Capture harness: output path for the scene image."),
    "SOMNIUM_CAPTURE_QUIT": ("harness", "Capture harness: exit after the capture completes."),
    "SOMNIUM_CAPTURE_UI_PNG": ("harness", "Capture harness: output path for the editor-surface image."),
    "SOMNIUM_CAPABILITY_REPORT": ("harness", "Startup capability probe output path; the live capability summary is exposed by diagnostics, while this variable records machine evidence for a run."),
    "SOMNIUM_TIME": ("harness", "`.somtime` harness: enables frame timing collection."),
    "SOMNIUM_TIME_COMPARE": ("harness", "`.somtime` harness: the baseline row to compare against."),
    "SOMNIUM_TIME_FRAMES": ("harness", "`.somtime` harness: how many frames to measure."),
    "SOMNIUM_TIME_LABEL": ("harness", "`.somtime` harness: the label written into the row."),
    "SOMNIUM_TIME_QUIT": ("harness", "`.somtime` harness: exit once the run completes."),
    "SOMNIUM_TIME_WARMUP": ("harness", "`.somtime` harness: frames discarded before measuring."),
    "SOMNIUM_PROFILE_EVERY": ("harness", "Profiler cadence for a headless run; the overlay is the editor's route."),
    "SOMNIUM_CAMERA_POS": ("harness", "Headless startup pose. The editor's route is the camera itself."),
    "SOMNIUM_CAMERA_YAW": ("harness", "Headless startup pose. The editor's route is the camera itself."),
    "SOMNIUM_CAMERA_PITCH": ("harness", "Headless startup pose. The editor's route is the camera itself."),
    "SOMNIUM_WATER_YAW": ("harness", "Recorded water repro: the fixed camera yaw it was measured at."),
    "SOMNIUM_WATER_PITCH": ("harness", "Recorded water repro: the fixed camera pitch it was measured at."),
    "SOMNIUM_TERRAIN_EYE": ("harness", "Recorded terrain repro: the fixed eye position it was measured at."),
    "SOMNIUM_SUN_AZIMUTH": ("harness", "Headless sun placement; CONTROL-L gives the sun a real control."),
    "SOMNIUM_SUN_ELEVATION": ("harness", "Headless sun placement; CONTROL-L gives the sun a real control."),
    "SOMNIUM_MAP": ("harness", "Startup scene for a headless run; the editor route is File > Open."),
    "SOMNIUM_IMPORT": ("harness", "One-shot glTF import at startup; the editor route is File > Import."),
    "SOMNIUM_HEIGHTMAP": ("harness", "One-shot heightmap load at startup; the editor route is the terrain tools."),
    "SOMNIUM_TERRAIN": ("harness", "Headless terrain bootstrap; the editor route is Create > Terrain."),
    "SOMNIUM_TERRAIN_RES": ("harness", "Headless terrain bootstrap resolution; authored terrain declares its own."),
    "SOMNIUM_VIEWPORT_RES": ("harness", "Fixed render resolution for capture; the editor route is dynamic resolution."),
    "SOMNIUM_MAXIMIZE": ("harness", "Window state at startup, before any editor surface exists."),
    "SOMNIUM_SCRIPT_CACHE": ("harness", "Script bytecode cache directory; a build-tool path, not a scene property."),
    "SOMNIUM_WRITE_DECLS": ("harness", "Writes the script declaration file and exits; a build step."),
    "SOMNIUM_TERRAIN_FORCE_RGBA8": ("harness", "Hardware-compatibility fallback probed at device creation."),
    "SOMNIUM_TERRAIN_ALLOW_OVERBUDGET": ("harness", "Lifts an assertion for a deliberate over-budget measurement run."),
    "SOMNIUM_TERRAIN_PROJECTION_SHARPNESS": ("harness", "Triplanar tuning constant held for a recorded shading repro."),
    "SOMNIUM_TERRAIN_RELIEF": ("harness", "Recorded terrain repro override; the authored route is Terrain > Relief."),
    "SOMNIUM_TERRAIN_WETNESS": ("harness", "Recorded terrain repro override; the authored route is Terrain > Wetness."),
    "SOMNIUM_SHADOW_RADIUS": ("harness", "Recorded shadow repro override; the authored route is the light's source radius."),
    "SOMNIUM_AERIAL_SPLIT": ("harness", "Forces every terrain tile onto the near pipeline for an A/B timing run."),
    "SOMNIUM_TAA_DILATE_EPS": ("harness", "Numerical epsilon held for a recorded TAA repro."),
    "SOMNIUM_WATER_VIEW": ("harness", "Places the camera at a water body for a recorded capture. The editor route is Focus Selection and the camera bookmarks."),
    "SOMNIUM_KIT_VIEW": ("harness", "Places the camera for an XV-J kit capture. The editor route is Focus Selection and the camera bookmarks."),
    "SOMNIUM_TIME_VIEW": ("harness", "Chooses the pose a `.somtime` run measures from. The editor route is the camera bookmarks."),
    "SOMNIUM_VIRTUAL_SHADOWS": ("harness", "Forces the demo sun to Virtual for unattended CSM/VSM timing runs; authored lights use Details."),
    "SOMNIUM_SHADOWTEST": ("harness", "Spawns a synthetic shadow-test scene at startup, before any editor surface exists."),
    "SOMNIUM_SHADE_ABLATE": ("harness", "Disables shading stages for an A/B measurement; its own documentation says it is never set from the UI."),
    "SOMNIUM_UI_SHAPER": ("harness", "A/B-only switch for the not-yet-available shaped-text implementation; exposing a selectable editor option would falsely imply that shaping works."),
}
