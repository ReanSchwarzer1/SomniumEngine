//! The main entry point for the Somnium Renderer.
//!
//! Orchestrates the `GlobalResourcePool`, the shader system, and rendering passes.
//!
//! Phase 11 additions:
//! - `ShadowMapResources` + `ShadowPass` for cascade shadow maps
//! - `GpuDirectionalLight` upload each frame
//! - View buffer expanded to 208 bytes (adds raw view matrix for cascade selection)
//! - `set_view` now takes separate `view` and `proj` matrices
//! - `set_directional_light` / `set_cascade_debug` public APIs

use crate::{
    bindless::GlobalResourcePool,
    command::DrawCommand,
    context::RenderContext,
    geometry::GeometryPool,
    instance::InstancePool,
    material::pool::{GpuMaterial, MaterialPool},
    pass::{
        gizmo::{GizmoMode, GizmoPass},
        grid::GridPass,
        outline::OutlinePass,
        particle::{GpuParticle, ParticlePass},
        postprocess::{HDR_FORMAT, PostProcessPass},
        shading::ShadingPass,
        shadow::ShadowPass,
        visibility::VisibilityBufferPass,
    },
    shadow::{ATLAS_SIZE, GpuDirectionalLight, ShadowMapResources, cascade::compute_cascades},
    texture_pool::TexturePool,
};
use somnium_ui::UiManager;
use winit::window::Window;

/// Above this many copies of the same mesh in one frame, cluster expansion is
/// abandoned for that mesh and each draw becomes a single whole-mesh argument.
///
/// Chosen low because the trade flips fast: the argument count is
/// `instances x clusters`, so anything genuinely instanced blows past what
/// per-cluster culling can ever recover.
const MAX_INSTANCES_FOR_CLUSTERING: u32 = 8;

/// Everything the ECS layer needs to spawn one entity from an uploaded scene node.
#[derive(Debug, Clone)]
pub struct UploadedNode {
    pub entity_name: String,
    pub vertex_offset: u32,
    pub index_offset: u32,
    pub index_count: u32,
    pub material_id: u32,
    /// Source material index, retained so the editor can attach the editable
    /// `.sommat` sibling rather than authoring the runtime pool slot.
    pub material_index: usize,
    pub transform: glam::Mat4,
}

/// The primary renderer struct.
/// Bytes one view's uniform block occupies in the staging buffer.
///
/// 224 is what the block holds; 256 is what it is spaced at, because a copy
/// offset has to be aligned and 256 is the alignment every backend accepts.
const VIEW_SLOT_BYTES: u64 = 256;

/// How many view slots a frame may use.
///
/// Four viewports plus the overlay pass, rounded up. A frame asking for more
/// wraps rather than growing the buffer: the wrap is visible (two views share a
/// matrix) where a silent reallocation mid-frame is not.
const VIEW_SLOTS: u64 = 8;

/// Another window's surface, when the scene is not going into the editor's.
///
/// MORROWIND-J step 2. A floated viewport does not get its own recording of the
/// scene: it gets *the* recording. Everything from the visibility pass to the
/// selection outline lands here instead, and only the editor's chrome still
/// goes to the window this call was made for.
///
/// The alternative was recording the scene a second time for the second window,
/// which works and costs a whole extra frame of scene work — and puts the
/// floating viewport on the secondary-view path, without TAA, without FSR and
/// without ReSTIR's history, because those are bound to one camera and one
/// rectangle. Redirecting keeps the floating viewport on the *primary* path,
/// which is the one the editor has always used, and leaves the editor's own
/// window with no scene to draw at all.
#[derive(Clone, Copy)]
pub struct SceneTarget<'a> {
    /// Where the scene, the gizmos, the outline and the particles land.
    pub view: &'a wgpu::TextureView,
    /// That surface's size in physical pixels, which is what the upscale
    /// target has to match.
    pub size: (u32, u32),
}

/// The slot the editor overlays read from, after every view has been recorded.
const OVERLAY_VIEW_SLOT: u64 = VIEW_SLOTS - 1;

/// What a frame's per-view overrides are overriding.
///
/// MORROWIND-J step 3. A view may turn TAA off or select a debug
/// visualisation; something has to hold what those settings were before the
/// loop touched them, and it cannot be the loop.
#[derive(Clone, Copy, Debug, Default)]
struct FrameViewState {
    taa: bool,
    fsr: bool,
    shading_debug: f32,
    overlays: bool,
}

pub struct SomniumRenderer {
    /// Global descriptor pool (bindless arrays, includes light buffer at binding 6).
    pub global_pool: GlobalResourcePool,
    /// High level material system cache.
    pub shaders: crate::shaders::Shaders,
    /// DREAMS-B: one spatiotemporal sampling resource shared by noisy passes.
    pub grain_masks: crate::pass::grain::GrainMasks,
    /// The visibility buffer render pass.
    pub vis_pass: VisibilityBufferPass,
    /// The final shading pass.
    pub shading_pass: ShadingPass,
    /// Shadow atlas + comparison sampler (shared between shadow pass and shading pass).
    pub shadow_resources: ShadowMapResources,
    /// Depth-only shadow render pass (4 cascades).
    pub shadow_pass: ShadowPass,
    /// Authored directional-light CSM/VSM request. CSM is the measured default.
    pub directional_shadow_policy: crate::shadow::virtual_map::ShadowLightPolicy,
    /// Software-sparse page allocator shared by the portable and GPU paths.
    pub virtual_shadow_cache: crate::shadow::virtual_map::VirtualShadowMap,
    /// Allocated lazily: a CSM-only scene pays no second shadow-atlas memory.
    pub virtual_shadow_gpu: Option<crate::shadow::virtual_map::VirtualShadowGpu>,
    /// Page raster work prepared from screen-visible receivers this frame.
    pub virtual_shadow_work: Vec<crate::shadow::virtual_map::RenderPage>,
    /// Honest branch gate: resources alone do not make VSM sampleable.
    virtual_shadow_readiness: crate::shadow::virtual_map::VirtualShadowReadiness,
    /// Explicit content revision for in-place mesh edits whose offsets/counts
    /// remain stable and therefore cannot be discovered by hashing commands.
    shadow_caster_content_revision: u64,
    /// DOOM-D: pure invalidation policy for the four persistent CSM quadrants.
    cascade_shadow_cache: crate::shadow::cache::CascadeShadowCache,
    /// Hash of the filtered caster content touching each resolved cascade.
    cascade_shadow_revisions: [u64; crate::shadow::NUM_CASCADES],

    /// Global geometry storage.
    pub geometry: GeometryPool,
    /// Global material storage.
    pub materials_pool: MaterialPool,
    /// Global texture storage.
    pub texture_pool: TexturePool,
    /// Per-frame instance storage.
    pub instances: InstancePool,

    /// The current camera view matrix (world → view).
    pub view_matrix: glam::Mat4,
    /// The current projection matrix.
    pub proj_matrix: glam::Mat4,
    /// The current combined view-projection matrix.
    pub view_proj: glam::Mat4,
    /// The current camera position in world space.
    pub camera_pos: glam::Vec3,
    /// The elapsed engine time in seconds.
    pub time: f32,

    /// Directional light direction (toward the light, world space, normalized).
    pub light_direction: glam::Vec3,
    /// Scene-wide indirect-light strength, driven by `PostProcessComponent`.
    ibl_intensity: f32,
    /// Directional light color, pre-multiplied by intensity.
    pub light_color: glam::Vec3,
    /// Directional moonlight illuminance in lux (Phase 25M-2).
    pub moon_intensity: f32,

    /// When true, the shading pass tints pixels by cascade index (debug overlay).
    cascade_debug: bool,

    /// Phase 13D: packed flags. Bit 0 = cel, bit 1 = PCSS, bit 2 = contact,
    /// bit 3 = analytic grads, bit 4 = ReSTIR DI sun vis (set at upload).
    pub shading_mode: u32,
    /// Phase TSUSHIMA-F: multiple-scattering compensation, direct and IBL.
    pub brdf_multiscatter: bool,
    /// Phase TSUSHIMA-F: Hammon's rough diffuse in place of Burley.
    pub brdf_rough_diffuse: bool,
    /// Phase TSUSHIMA-F: AO-derived occlusion of direct light.
    pub brdf_micro_shadow: bool,
    /// What the per-view overrides are overriding, for this frame.
    frame_view_state: FrameViewState,
    /// One slot per view, copied into the view buffer from inside the encoder.
    view_stage: wgpu::Buffer,
    /// Which staging slot the view currently being recorded writes into.
    ///
    /// Set by `apply_scene_view` rather than threaded down, because the upload
    /// happens eighty lines into a method that already takes six arguments —
    /// and unlike the matrices themselves, nothing else reads it.
    view_slot: u64,
    /// MORROWIND-J step 3. The views this frame draws.
    ///
    /// Empty is not "no views" — it is *the* view, the whole window with
    /// whatever camera `set_view` was last given, which is what every frame
    /// before this field existed drew. The editor fills it in when it wants
    /// more than one, and clearing it puts the renderer back on the single-view
    /// path rather than on a one-element multi-view path.
    views: Vec<crate::view::SceneView>,
    /// Phase 13C: Accumulated local lights for the frame.
    local_lights: Vec<crate::cluster::GpuLocalLight>,

    /// Water textures bind group.
    pub water_textures_bind_group: Option<wgpu::BindGroup>,
    /// Phase IV-C: mask/depth/SDF resources keyed by ECS water handles.
    pub water_bodies: crate::water_body::WaterBodyRegistry,

    /// Editor infinite-grid overlay pass.
    grid_pass: GridPass,
    /// When true, the grid overlay is composited into the HDR target.
    grid_enabled: bool,

    /// Post-processing pass: owns the HDR render target + tone-maps to swapchain.
    postprocess_pass: PostProcessPass,
    atmosphere_pass: crate::pass::atmosphere::AtmospherePass,
    auto_exposure_pass: crate::pass::auto_exposure::AutoExposurePass,
    pub taa_pass: crate::pass::taa::TaaPass,
    pub gtao_pass: crate::pass::gtao::GtaoPass,
    pub bloom_pass: crate::pass::bloom::BloomPass,
    pub dof_pass: crate::pass::dof::DofPass,
    pub raytrace_pass: crate::pass::raytrace::RaytracePass,
    rt_debug_pass: crate::pass::raytrace::RtDebugPass,
    pub restir_pass: crate::pass::restir::RestirPass,
    /// Phase 24L: ray-traced indirect diffuse.
    pub restir_gi_pass: crate::pass::restir_gi::RestirGiPass,
    /// Phase 24M–Q: world cache, scene specular, path tracer, SDF, probes.
    pub lighting_extra_pass: crate::pass::lighting_extra::LightingExtraPass,
    /// MORROWIND-AB: portable SDF-backed diffuse probe volume.
    pub ddgi_pass: crate::pass::ddgi::DdgiPass,
    /// Phase VV: ray-traced water reflections (Halcyon).
    pub water_reflection_pass: crate::pass::water_reflection::WaterReflectionPass,
    /// Phase 24AC: contrast adaptive sharpening, the last pass of the frame.
    pub cas_pass: crate::pass::cas::CasPass,
    /// Bilinear blit when the 3D target is smaller than the swapchain.
    present_pass: crate::pass::present::PresentPass,
    /// FSR 3 temporal reconstruct to display resolution.
    pub fsr_pass: crate::pass::fsr::FsrPass,
    /// LDR post chain size (FXAA / CAS / present). Display-sized when FSR is
    /// on, scene-sized otherwise.
    ldr_width: u32,
    ldr_height: u32,
    /// Phase 24AD: screen-space motion, for motion blur and future object motion.
    pub velocity_pass: crate::pass::velocity::VelocityPass,
    /// Phase 24Z: motion blur, which waited on 24AD for its velocity.
    pub motion_blur_pass: crate::pass::motion_blur::MotionBlurPass,
    /// Phase 24AE: minimum projected screen radius for a shadow caster.
    ///
    /// Unreal ships 0.01 as `r.Shadow.RadiusThreshold`. Zero disables the test,
    /// which is the A/B. Public so the editor can scrub it.
    pub shadow_radius_threshold: f32,
    /// Exposure multiplier applied before ACES tone mapping (default 1.0).
    pub exposure: f32,
    /// Meter the frame and adapt exposure to it, rather than using `exposure`.
    pub auto_exposure: bool,
    /// Seconds since the previous frame, for exposure adaptation.
    pub frame_delta_time: f32,
    /// Stops applied on top of the metered exposure. Negative darkens.
    pub exposure_compensation: f32,
    /// Colour grading and lens settings (Phases 24Y / 24Z).
    pub grading: crate::pass::postprocess::Grading,
    /// View-projection without TAA's sub-pixel jitter, for reprojection.
    view_proj_unjittered: glam::Mat4,
    /// Current render target size, for computing the jitter offset.
    render_width: u32,
    render_height: u32,
    /// Half the sun's angular diameter, radians (Phase 24E). Drives the
    /// specular highlight's size and the shadow penumbra's width.
    pub sun_angular_radius: f32,
    /// Tone-mapping curve index; matches `Tonemapper::as_index`.
    pub tonemapper: u32,
    /// Radial vignette strength (0 = off, 1 = default, higher = stronger).
    ///
    /// Defaults to **off**: an always-on vignette darkens the viewport edges,
    /// which reads as a dirty screen in an editor. Enable it per-scene through
    /// the Post Processing entity (Phase 15A1).
    pub vignette_strength: f32,
    /// Chromatic-aberration strength in UV units at the screen edge (0 = off).
    pub chromatic_aberration: f32,
    /// Phase 15A2: FXAA anti-aliasing pass (runs before editor overlays).
    fxaa_pass: crate::pass::fxaa::FxaaPass,
    /// Weighted-blended OIT (MORROWIND-AC). Default off.
    pub oit_pass: crate::pass::oit::OitPass,
    /// SMAA 1x / T2x (MORROWIND-AC). Owns whether it runs.
    pub smaa_pass: crate::pass::smaa::SmaaPass,
    /// Whether FXAA is applied. When off, post-processing writes straight to
    /// the swapchain and the pass costs nothing.
    pub fxaa_enabled: bool,

    /// Editor transform gizmo pass.
    gizmo_pass: GizmoPass,

    /// Phase 13E: light visualization pass (point/spot/directional bounds).
    light_gizmo_pass: crate::pass::light_gizmo::LightGizmoPass,
    /// Light gizmos submitted this frame.
    light_gizmo_queue: Vec<crate::pass::light_gizmo::LightGizmoDesc>,
    /// Free-form editor lines, drawn in the same batch as the light gizmos.
    ///
    /// `LightGizmoDesc` describes one of a fixed set of shapes; a spline is a
    /// polyline whose length the author decides, and squeezing that through a
    /// struct of fixed fields would have meant either a second pass or a
    /// pretend "light" with a point list bolted on.
    line_gizmo_queue: Vec<crate::pass::light_gizmo::LineVertex>,
    /// When true, submitted light gizmos are drawn (toggle with `L`).
    light_gizmos_enabled: bool,
    /// Master switch for editor-only viewport overlays. Play-in-editor keeps
    /// their state but suppresses transform/light gizmos, selection outlines,
    /// and the optional editor grid from the player's view.
    editor_overlays_enabled: bool,
    /// MORROWIND-E2. One warning, not one per frame: a game whose UI hook draws
    /// nothing has a bug, and sixty log lines a second is not how to say so.
    game_ui_empty_warned: bool,
    /// Which gizmo operation is active.
    pub gizmo_mode: GizmoMode,
    /// World-space position of the selected entity (None when nothing selected).
    pub gizmo_world_pos: Option<glam::Vec3>,
    /// Orientation of the gizmo axes. Identity in world mode; the selected
    /// entity's propagated rotation in local mode.
    pub gizmo_world_rotation: glam::Quat,

    /// Phase 11.5I: Selection outline pass.
    outline_pass: OutlinePass,
    /// Mesh data for the currently selected entity (vertex_offset, index_offset,
    /// index_count, model matrix). None when no entity with a mesh is selected.
    outline_entity: Option<(u32, u32, u32, glam::Mat4)>,

    /// Phase 11.5J: GPU particle billboard pass.
    particle_pass: ParticlePass,
    /// Particle instances to render this frame (uploaded in render()).
    pending_particles: Vec<GpuParticle>,

    /// Phase 13: Water pass.
    pub water_pass: crate::pass::water::WaterPass,
    water_queue: Vec<(
        u32,
        glam::Mat4,
        crate::pass::water::WaterMaterialData,
        u32,
        u32,
        u32,
    )>,
    underwater_pass: crate::pass::underwater::UnderwaterPass,
    /// Active finite body at the camera XZ, if any.
    pub underwater_body: Option<u32>,
    /// Smooth 0..1 camera submersion reported to gameplay/editor systems.
    pub camera_submersion: f32,

    /// Phase 25A-2: per-terrain splat/layer parameters read by `shading.wgsl`.
    terrain_materials: crate::material::pool::TerrainMaterialPool,
    /// Inspector override for `SOMNIUM_SHADOW_DEBUG` (0 = use env).
    pub shading_debug: f32,
    /// CONTROL-G: the named pipeline switches, seeded from the environment.
    ///
    /// The individual fields below stay as they are — every pass reads them
    /// and nothing about that changes. This is the *authored* state, and
    /// [`Self::apply_debug_toggles`] is the one place it reaches them.
    pub debug_toggles: somnium_ui::debug::DebugToggles,
    /// Deterministic HDR frame readback for A/B measurement. Inert unless
    /// `SOMNIUM_CAPTURE` or `SOMNIUM_CAPTURE_COMPARE` is set.
    capture: crate::capture::FrameCapture,
    /// Phase 29. Public because the editor drives its toggle and reads its
    /// report; there is nothing to encapsulate behind an accessor pair.
    pub profiler: crate::profiler::GpuProfiler,
    /// Phase DOOM-A. Inert unless `SOMNIUM_TIME` is set; when it is, it forces
    /// the profiler on, accumulates unsmoothed samples and writes a `.somtime`
    /// table with a standard deviation beside every mean.
    timing: Option<crate::timing::TimingRun>,
    /// Phase DOOM-B. Counts pixels per prospective shading bin. Public so the
    /// editor can toggle it and read the table; default off (`SOMNIUM_CENSUS=1`).
    pub census_pass: crate::pass::census::CensusPass,
    /// Phase DOOM-C. Routes tiles to per-bin shading pipelines. Public for the
    /// Post FX toggle and the Terrain aerial-split field.
    pub classify_pass: crate::pass::classify::ClassifyPass,
    /// Phase DOOM-E: camera distance past which terrain shades through the
    /// aerial pipeline. Editable on the Terrain entity.
    pub aerial_split: f32,
    /// Whether the user wants the aerial split at all.
    ///
    /// **Default off, and measured rather than assumed.** With only hex tiling
    /// and the parallax march removed the aerial pipeline changed 925 pixels of
    /// 2 938 110 — invisible — and cost **+2.3 ms** on the Coastal overview,
    /// because `gpu_material_for_camera` had already switched both off above
    /// 80 m and the second full-screen pass was pure overhead. It only pays
    /// with [`Self::aerial_hero_bank`], which is a real look change.
    pub aerial_split_enabled: bool,
    /// Phase DOOM-E: also cut the aerial pipeline's layer scan to the hero
    /// bank. Changes the look of distant terrain on a 32-layer map, so it is
    /// opt-in and measured separately.
    pub aerial_hero_bank: bool,
    /// Whether it is actually running this frame — false when the near spec has
    /// nothing to drop, so the second pipeline would be a copy of the first.
    aerial_split_active: bool,
    /// Phase DOOM-B ablation code, from `SOMNIUM_SHADE_ABLATE`. Zero in every
    /// normal run; non-zero deliberately renders a wrong image so one class of
    /// pixel can be timed on its own.
    shade_ablate: u32,
    /// Phase DOOM-F. Scene size the editor asked for, before any dynamic scale.
    base_extent: (u32, u32),
    /// Phase DOOM-F resolution controller. Public: the Camera details drive its
    /// toggle, target and floor. Off by default.
    pub dynamic_resolution: crate::viewport_resolution::DynamicResolution,
    /// Material ids belonging to a terrain, so a capture can label its pixels.
    terrain_material_ids: std::collections::HashSet<u32>,
    /// All created terrains, indexed by terrain id (`TerrainComponent::terrain_id`).
    pub terrains: Vec<crate::terrain::TerrainData>,
    /// Phase DF: one clipmap stack per terrain, same index as `terrains`.
    pub clipmaps: Vec<crate::terrain::clipmap::TerrainClipmap>,
    clipmap_pass: crate::pass::terrain_clipmap::TerrainClipmapPass,
    /// Terrain ids (+ model matrices) submitted for the current frame.
    terrain_queue: Vec<(u32, glam::Mat4)>,

    /// Phase 15A: indirect draw arguments for the visibility pass.
    indirect: crate::indirect::IndirectDrawBuffer,
    /// Whether the GPU-driven indirect path is currently active.
    /// When false the renderer falls back to one `draw()` per object.
    gpu_driven: bool,
    /// Whether the device supports it at all (gates the runtime toggle).
    supports_gpu_driven: bool,
    /// Whether the device may consume GPU-authored compact draw counts.
    supports_counted_draws: bool,

    /// Phase 19: environment cubemap for image-based lighting.
    ibl_pass: crate::pass::ibl::IblPass,
    /// Phase CONTROL-M: volumetric clouds. Public because the editor drives
    /// every one of its parameters from `SkyComponent`.
    pub cloud_pass: crate::pass::clouds::CloudPass,
    /// Phase CONTROL-O: deferred decals, binned through the same froxel grid
    /// as the local lights.
    pub decal_grid: crate::pass::decal::DecalGrid,
    /// This frame's decals, pushed by the engine layer and cleared after
    /// binning — the same lifecycle `local_lights` has, and for the same
    /// reason: the binning must use the *render's* view matrix, not whatever
    /// the matrix was when the ECS was walked.
    pub decals: Vec<crate::pass::decal::GpuDecal>,
    /// Phases 24U/25I: froxel volume carrying aerial perspective and fog.
    pub volumetric_pass: crate::pass::volumetric::VolumetricPass,

    /// Phase 15E: Hi-Z depth pyramid, rebuilt from the visibility depth buffer
    /// each frame and consumed by the two-phase occlusion cull.
    pub hiz_pass: crate::pass::hiz::HiZPass,
    /// False until a pyramid has been built from real geometry. Occlusion
    /// culling is held off until then — see the note at the cull dispatch.
    hiz_ready: bool,
    /// `SOMNIUM_CULL_STATS=1` reads the indirect args back after each cull
    /// phase and logs how many draws survived. Off by default: the readback
    /// stalls the pipeline waiting on the GPU.
    cull_stats: bool,
    /// Staging copies of the indirect args after phase one and phase two.
    cull_stats_buffers: Option<[wgpu::Buffer; 2]>,
    /// `SOMNIUM_NO_OCCLUSION=1` keeps frustum culling but skips the Hi-Z half,
    /// so the two can be measured apart.
    occlusion_off: bool,

    /// Phase 15B: GPU frustum-culling compute pass.
    cull_pass: crate::pass::cull::CullPass,
    /// Per-draw local AABBs for culling, rebuilt each frame.
    cull_aabbs: Vec<crate::culling::GpuCullAabb>,
    /// Phase 15F: this frame's indirect arguments, one per cluster.
    cluster_args: Vec<crate::indirect::DrawIndirectArgs>,
    /// How many times each mesh appears in this frame's draw queue, used to
    /// decide whether cluster expansion is worth it (Phase 17G).
    instanced_counts: std::collections::HashMap<u32, u32>,
    /// When true, a draw is expanded into one indirect argument per cluster so
    /// culling works below whole-object granularity. `SOMNIUM_NO_MESHLETS=1`
    /// forces the whole-mesh path, for A/B measurement.
    pub meshlet_draws: bool,
    /// When false the cull shader keeps every draw (useful for A/B checks).
    pub culling_enabled: bool,
    /// CPU AABB frustum early-out for terrain vis draws (Phase CR-B). Default
    /// on. Independent of [`Self::culling_enabled`] (GPU 15B / F10). Off-screen
    /// casters still reach the shadow pass via `shadow_only_queue`.
    pub cpu_frustum_cull: bool,
    /// Cull shadow casters against cascade volumes, never the camera (CR-E).
    pub cascade_caster_cull: bool,
    /// This frame's cascade view-projections, for CPU caster tests.
    cascade_view_projs: [glam::Mat4; crate::shadow::NUM_CASCADES],
    /// Terrain chunks rebuilt this frame; capacity is kept across frames (CR-F).
    rebuilt_chunks: Vec<u32>,
    /// This frame's packed terrain LOD word, keyed by chunk vertex offset
    /// (PORTAL-0-D).
    ///
    /// `gpu_instance_from_cmd` used to recover this by scanning every chunk
    /// of every terrain for each draw command — O(draws x chunks), which is
    /// quadratic in terrain size and was measurable as a cross-scene ratio:
    /// Coastal (256 chunks / 89 draws) against Island (64 / 56) predicts
    /// 6.4x and the `Instances` zone measured 5.4x. The terrain loop below
    /// already holds the chunk when it builds the draw, so the word is
    /// recorded there instead of searched for afterwards.
    terrain_lod_by_vertex: std::collections::HashMap<u32, u32>,
    /// Off-camera casters that still shadow into a cascade. Not in `draw_queue`,
    /// so they skip vis / GPU 15B, but they occupy instance slots after the
    /// opaque vis draws so the shadow pass can find their transforms.
    shadow_only_queue: Vec<DrawCommand>,
    /// Persistent shadow-caster list (CR-F). Cleared and refilled each frame.
    shadow_caster_scratch: Vec<crate::pass::shadow::ShadowCaster>,

    /// Phase 21: forward pass for alpha-blended materials.
    transparent_pass: crate::pass::transparent::TransparentPass,
    /// Blended draws submitted this frame (routed automatically by material).
    transparent_queue: Vec<DrawCommand>,
    /// Per-material flag: true when the material is alpha-blended. Lets
    /// `submit` route draws without any call site needing to know.
    material_blend: Vec<bool>,
    /// Phase 17D: per-material double-sided flag, used to split the visibility
    /// draws between the back-face-culled and two-sided pipelines.
    material_double_sided: Vec<bool>,
    /// Number of leading indirect arguments that belong to single-sided
    /// materials. Everything after it is drawn with culling off.
    single_sided_args: usize,

    /// The list of draw commands submitted this frame.
    draw_queue: Vec<DrawCommand>,
}

/// One TSUSHIMA-F term's switch: its own variable, or the group switch, or on.
///
/// The group switch exists so a single `SOMNIUM_TERRAIN_BRDF=0` still returns
/// the whole pre-phase response in one go; the per-term variables exist
/// because measuring three terms through one switch cannot say which of them
/// moved the picture, and on this content they do not move it equally.
fn brdf_term_enabled(var_name: &str) -> bool {
    if let Ok(v) = std::env::var(var_name) {
        return v != "0";
    }
    std::env::var("SOMNIUM_TERRAIN_BRDF").as_deref() != Ok("0")
}

impl SomniumRenderer {
    /// Push [`Self::debug_toggles`] into the fields the passes actually read.
    ///
    /// Called after every menu flip rather than every frame: the toggles are
    /// authored state that changes when a person changes it, and copying them
    /// per frame would quietly overwrite anything else that writes the same
    /// field — which is exactly how the terrain inspector's parallax control
    /// and a view-menu toggle would end up fighting.
    pub fn apply_debug_toggles(&mut self) {
        let on = |id: &str| self.debug_toggles.is_on(id);
        self.meshlet_draws = on("meshlets");
        self.occlusion_off = !on("occlusion");
        self.cull_stats = on("cull_stats");
        self.cascade_caster_cull = on("cascade_cull");
        self.aerial_split_enabled = on("aerial");
        self.aerial_hero_bank = on("aerial_hero");
        self.census_pass.enabled = on("pixel_census");
        self.cloud_pass.jitter = on("cloud_jitter");
        self.classify_pass.enabled = on("shading_bins");
        let grain = on("dreams_grain");
        self.grain_masks.set_shared_enabled(grain);
        self.gtao_pass.set_grain_enabled(grain);
        self.restir_pass.set_grain_enabled(grain);
        self.restir_gi_pass.set_grain_enabled(grain);
        self.taa_pass.set_grain_enabled(grain);
        self.volumetric_pass.set_grain_enabled(grain);
        self.grain_masks.set_stf_enabled(on("dreams_stf"));

        let hex = on("hex_tiling");
        let morph = on("terrain_lod_morph");
        let height_blend = on("terrain_height_blend");
        let triplanar = u32::from(on("terrain_triplanar"));
        let macro_on = on("terrain_macro");
        let detail_fade = on("terrain_detail_fade");
        let parallax = on("terrain_parallax");
        for terrain in &mut self.terrains {
            terrain.hex_tiling = hex;
            terrain.lod_morph = morph;
            terrain.height_blend = height_blend;
            terrain.projection_mode = triplanar;
            // Strength and distance are authored numbers, not booleans: the
            // toggle turns the effect off and restores the held value rather
            // than inventing one.
            terrain.macro_strength = if macro_on { 0.55 } else { 0.0 };
            terrain.detail_fade_start = if detail_fade { 60.0 } else { 1.0e9 };
            terrain.parallax_scale = if parallax { terrain.parallax_held } else { 0.0 };
            terrain.invalidate_unique_colour();
        }
        self.reconcile_clipmaps();
    }

    /// Whether virtual texturing has taken ownership of the clipmap.
    ///
    /// In VT mode `TerrainLayerTextures`' legacy layer arrays are 4x4
    /// placeholders — `load_bc7_layers` registers dummies on purpose — and the
    /// real BC7 pages only reach shading through the clipmap rings. A VT
    /// terrain with the clipmap off is therefore not "the clipmap turned off",
    /// it is terrain shaded from eight mean colours. The clipmap is not
    /// optional here, and `terrain_clipmap` is not allowed to claim otherwise.
    #[must_use]
    pub fn clipmap_owned_by_virtual_texturing(&self) -> bool {
        self.terrains
            .iter()
            .any(|terrain| terrain.virtual_texture_enabled)
    }

    /// The single writer of [`TerrainClipmap::enabled`].
    ///
    /// It used to have three. `apply_debug_toggles` wrote it from
    /// `debug_toggles`, `EditorEvent::ToggleTerrainClipmap` wrote it straight
    /// from the checkbox, and the per-frame terrain submit in `somnium_core`
    /// wrote it back to `true` for every virtual-textured terrain. The
    /// per-frame one ran last and ran always, so on any machine that takes the
    /// BC7 path the Clipmap checkbox had not turned the clipmap off since the
    /// virtual-texturing commit: the click was accepted, reverted within the
    /// frame, and the checkbox re-ticked itself at the next inspector refresh.
    ///
    /// Everything that wants to change it now goes through here.
    pub fn reconcile_clipmaps(&mut self) {
        let want = !crate::terrain::clipmap::TerrainClipmap::env_forced_off()
            && (self.debug_toggles.is_on("terrain_clipmap")
                || self.clipmap_owned_by_virtual_texturing());
        for clipmap in &mut self.clipmaps {
            // Coming back on has to force a refresh. While it was off the
            // camera kept moving and the rings kept their old centres, so the
            // first frame back shades the ground from a cache of somewhere
            // else, which is itself a straight-edged patch of wrong terrain.
            if want && !clipmap.enabled {
                clipmap.invalidate();
            }
            clipmap.enabled = want;
        }
    }

    /// Initialize the renderer using the provided `RenderContext`.
    pub fn new(ctx: &RenderContext) -> Self {
        let geometry = GeometryPool::new(&ctx.device);
        let materials_pool = MaterialPool::new(&ctx.device);
        let instances = InstancePool::new(&ctx.device);

        // Phase 11D/13: View buffer expanded to 224 bytes to include raw `view` matrix and `time`.
        let view_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("View Buffer"),
            size: 224,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // MORROWIND-J step 3. One slot per view, copied into the view buffer
        // from inside the encoder so the update is ordered against the passes
        // that read it. See `stage_view_buffer`.
        let view_stage = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("View Staging"),
            size: VIEW_SLOTS * VIEW_SLOT_BYTES,
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Phase 11A: Directional light buffer (336 bytes — expanded Phase 25M-2 for moon_direction).
        let light_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("DirectionalLight Buffer"),
            size: std::mem::size_of::<GpuDirectionalLight>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Phase 25A-2: terrain's splat/layer parameters, indexed by
        // `Material::terrain_index` from the shared shading pass.
        let terrain_materials = crate::material::pool::TerrainMaterialPool::new(&ctx.device);

        let global_pool = GlobalResourcePool::new(
            &ctx.device,
            &geometry.vertex_buffer,
            &geometry.index_buffer,
            &instances.buffer,
            &view_buffer,
            &materials_pool.buffer,
            &light_buffer,
            &terrain_materials.buffer,
        );

        // Phase DOOM-B. Built here rather than in the struct literal because it
        // borrows the global pool's layout, and the literal moves the pool.
        // MORROWIND-C. Every WGSL module in the crate is registered here, once,
        // and every pass below composes through it. This replaces the 29-line
        // `MaterialSystem` stub that described Ogre's HLMS and did nothing.
        let shaders = crate::shaders::Shaders::new();
        let grain_masks = crate::pass::grain::GrainMasks::new(&ctx.device, &ctx.queue, &shaders);

        let census_pass =
            crate::pass::census::CensusPass::new(&ctx.device, &shaders, &global_pool.layout);
        let classify_pass =
            crate::pass::classify::ClassifyPass::new(&ctx.device, &shaders, &global_pool.layout);

        // Phase 11.5H: Editor infinite-grid overlay (renders to HDR target).
        let grid_pass = GridPass::new(
            &ctx.device,
            &shaders,
            HDR_FORMAT,
            &global_pool.view_proj_buffer,
        );

        // Phase 24A-3: built before the post-process pass, which binds its
        // result buffer.
        let mut auto_exposure_pass =
            crate::pass::auto_exposure::AutoExposurePass::new(&ctx.device, &shaders);

        // Phase 24J: acceleration structures. Constructed even when the device
        // lacks ray query — the pass then does nothing, which keeps the call
        // sites free of feature checks.
        let raytrace_pass = crate::pass::raytrace::RaytracePass::new(
            &ctx.device,
            ctx.features.contains(crate::context::RAY_TRACING_FEATURES),
        );

        // Phase 24K: reservoir-based direct lighting, on top of 24J.
        let restir_pass = crate::pass::restir::RestirPass::new(
            &ctx.device,
            &shaders,
            raytrace_pass.supported(),
            grain_masks.view(),
            ctx.config.width,
            ctx.config.height,
        );

        // Phase 24L: indirect diffuse on the same acceleration structures.
        // Takes the global pool's layout because it resolves a ray hit through
        // the same scene bindings the shading pass rasterises through.
        let restir_gi_pass = crate::pass::restir_gi::RestirGiPass::new(
            &ctx.device,
            &shaders,
            &global_pool.layout,
            raytrace_pass.supported(),
            grain_masks.view(),
            ctx.config.width,
            ctx.config.height,
        );
        let lighting_extra_pass = crate::pass::lighting_extra::LightingExtraPass::new(
            &ctx.device,
            &shaders,
            &global_pool.layout,
            raytrace_pass.supported(),
            ctx.config.width,
            ctx.config.height,
        );
        let ddgi_pass = crate::pass::ddgi::DdgiPass::new(&ctx.device, &shaders);
        let clipmap_pass = crate::pass::terrain_clipmap::TerrainClipmapPass::new(
            &ctx.device,
            &shaders,
            &global_pool.layout,
        );

        let rt_debug_pass =
            crate::pass::raytrace::RtDebugPass::new(&ctx.device, &shaders, raytrace_pass.layout());

        // Phase 24Z: depth of field, driven by the same aperture as exposure.
        let dof_pass = crate::pass::dof::DofPass::new(
            &ctx.device,
            &shaders,
            HDR_FORMAT,
            ctx.config.width,
            ctx.config.height,
        );

        // Phase 24T: built before the post-process pass, which samples its result.
        let bloom_pass = crate::pass::bloom::BloomPass::new(
            &ctx.device,
            &shaders,
            HDR_FORMAT,
            ctx.config.width,
            ctx.config.height,
        );

        // Phase 11.5K: Post-process pass owns the Rgba16Float HDR render target.
        let postprocess_pass = PostProcessPass::new(
            &ctx.device,
            &shaders,
            ctx.config.format,
            ctx.config.width,
            ctx.config.height,
            auto_exposure_pass.exposure_buffer(),
            bloom_pass.result_view(),
        );
        auto_exposure_pass.resize(&ctx.device, &postprocess_pass.hdr_view);

        // Phase 24I: screen-space occlusion, consumed by the shading pass.
        let gtao_pass = crate::pass::gtao::GtaoPass::new(
            &ctx.device,
            &shaders,
            grain_masks.view(),
            ctx.config.width,
            ctx.config.height,
        );

        // Phase 24F: resolves the jittered HDR frames into a stable image.
        let taa_pass = crate::pass::taa::TaaPass::new(
            &ctx.device,
            &shaders,
            HDR_FORMAT,
            ctx.config.width,
            ctx.config.height,
            auto_exposure_pass.exposure_buffer(),
        );

        // Phase 11.5B: Transform gizmo (renders to swapchain after tone-mapping).
        let gizmo_pass = GizmoPass::new(
            &ctx.device,
            &shaders,
            ctx.config.format,
            &global_pool.view_proj_buffer,
        );

        // Phase 13E: light gizmos (drawn to the swapchain like the transform gizmo).
        let light_gizmo_pass = crate::pass::light_gizmo::LightGizmoPass::new(
            &ctx.device,
            &shaders,
            ctx.config.format,
            &global_pool.view_proj_buffer,
        );

        // Phase 11.5J: GPU billboard particle pass.
        let particle_pass = ParticlePass::new(&ctx.device, &shaders, ctx.config.format);

        // Phase 11.5I: Selection outline (stencil-based, renders to swapchain).
        let outline_pass = OutlinePass::new(
            &ctx.device,
            &shaders,
            ctx.config.format,
            &geometry.vertex_buffer,
            &geometry.index_buffer,
            ctx.config.width,
            ctx.config.height,
        );

        // Phase 11B: Shadow atlas and comparison sampler.
        // Shading pass output format is now Rgba16Float (HDR target).
        let shadow_resources = ShadowMapResources::new(&ctx.device);

        // Phase 11B: Depth-only shadow pass (4 cascades into the atlas).
        let shadow_pass = ShadowPass::new(&ctx.device, &shaders, &ctx.queue, &global_pool.layout);

        // Phase 19: build the environment cubemap before the shading pass, which
        // binds it. Contents are generated on the first frame (and whenever the
        // sun changes), not here.
        // Phase 24C: the atmosphere LUTs must exist before the IBL pass, which
        // binds them to ray-march the sky into the environment cubemap.
        let atmosphere_pass = crate::pass::atmosphere::AtmospherePass::new(&ctx.device, &shaders);
        let ibl_pass = crate::pass::ibl::IblPass::new(&ctx.device, &shaders, &atmosphere_pass);

        let vis_pass = VisibilityBufferPass::new(
            &ctx.device,
            &shaders,
            ctx.config.width,
            ctx.config.height,
            &global_pool.layout,
        );
        let hiz_pass = crate::pass::hiz::HiZPass::new(
            &ctx.device,
            &shaders,
            &ctx.queue,
            ctx.config.width,
            ctx.config.height,
            &vis_pass.depth_view,
        );
        let volumetric_pass =
            crate::pass::volumetric::VolumetricPass::new(&ctx.device, &shaders, grain_masks.view());

        // Phase CONTROL-M. Built before the shading pass because shading binds
        // its cloud-shadow field, and the field must exist before the bind
        // group that names it.
        let cloud_pass = crate::pass::clouds::CloudPass::new(
            &ctx.device,
            &shaders,
            ctx.config.width,
            ctx.config.height,
        );

        // Phase CONTROL-O. Built before the shading pass, like the clouds and
        // for the same reason: shading binds its buffers.
        let decal_grid = crate::pass::decal::DecalGrid::new(&ctx.device);

        let shading_pass = ShadingPass::new(
            &ctx.device,
            &shaders,
            &global_pool.layout,
            HDR_FORMAT, // shading writes to the Rgba16Float HDR texture
            &vis_pass.view,
            &shadow_resources.atlas_depth_view,
            &shadow_resources.comparison_sampler,
            &ibl_pass.cube_view,
            &ibl_pass.sampler,
            gtao_pass.output_view(),
            &vis_pass.depth_view,
            restir_pass
                .visibility_view()
                .expect("ReSTIR always allocates its visibility target"),
            restir_gi_pass
                .radiance_view()
                .expect("ReSTIR GI always allocates its radiance target"),
            &volumetric_pass.view,
            &volumetric_pass.sampler,
            lighting_extra_pass.aux_view(),
            lighting_extra_pass.volume_view(),
            ddgi_pass.sh_buffer(),
            &cloud_pass.shadow_view,
            &cloud_pass.shadow_params,
            &decal_grid,
            grain_masks.packed(),
        );

        // Phase 21: forward pass for blended materials. Built here because it
        // needs the global bind group layout and the environment cubemap.
        let transparent_pass = crate::pass::transparent::TransparentPass::new(
            &ctx.device,
            &shaders,
            HDR_FORMAT,
            &global_pool.layout,
            &ibl_pass.cube_view,
            &ibl_pass.sampler,
        );

        let texture_pool = TexturePool::new(&ctx.device);

        // Default sun direction (normalized (1,2,-1)) and white light at intensity 5.
        let default_dir = glam::Vec3::new(1.0, 2.0, -1.0).normalize();
        let default_color = glam::Vec3::splat(5.0);

        let water_pass = crate::pass::water::WaterPass::new(
            &ctx.device,
            &shaders,
            HDR_FORMAT,
            ctx.config.width,
            ctx.config.height,
        );
        let water_reflection_pass = crate::pass::water_reflection::WaterReflectionPass::new(
            &ctx.device,
            &shaders,
            &global_pool.layout,
            raytrace_pass.supported(),
            ctx.config.width,
            ctx.config.height,
        );
        let water_textures_bind_group =
            Some(crate::pass::water::create_default_texture_bind_group(
                &ctx.device,
                &ctx.queue,
                &water_pass.tex_bind_group_layout,
                water_pass.spectrum.views(),
            ));
        let underwater_pass =
            crate::pass::underwater::UnderwaterPass::new(&ctx.device, &shaders, HDR_FORMAT);

        // Phase 24AD. Built here rather than inside the struct literal because
        // it borrows the visibility pass's depth view, which the literal moves.
        let velocity_pass = crate::pass::velocity::VelocityPass::new(
            &ctx.device,
            &shaders,
            &vis_pass.depth_view,
            ctx.config.width,
            ctx.config.height,
        );

        Self {
            global_pool,
            vis_pass,
            shading_pass,
            shadow_resources,
            shadow_pass,
            directional_shadow_policy: crate::shadow::virtual_map::ShadowLightPolicy::default(),
            virtual_shadow_cache: crate::shadow::virtual_map::VirtualShadowMap::new(
                crate::shadow::virtual_map::VirtualShadowConfig::default(),
            )
            .expect("built-in virtual shadow configuration must be valid"),
            virtual_shadow_gpu: None,
            virtual_shadow_work: Vec::with_capacity(64),
            virtual_shadow_readiness: crate::shadow::virtual_map::VirtualShadowReadiness::default(),
            shadow_caster_content_revision: 0,
            cascade_shadow_cache: crate::shadow::cache::CascadeShadowCache::default(),
            cascade_shadow_revisions: [0; crate::shadow::NUM_CASCADES],
            geometry,
            materials_pool,
            texture_pool,
            instances,
            view_matrix: glam::Mat4::IDENTITY,
            proj_matrix: glam::Mat4::IDENTITY,
            view_proj: glam::Mat4::IDENTITY,
            camera_pos: glam::Vec3::ZERO,
            time: 0.0,
            brdf_multiscatter: brdf_term_enabled("SOMNIUM_TERRAIN_BRDF_MS"),
            brdf_rough_diffuse: brdf_term_enabled("SOMNIUM_TERRAIN_BRDF_DIFFUSE"),
            brdf_micro_shadow: brdf_term_enabled("SOMNIUM_TERRAIN_BRDF_MICROSHADOW"),
            light_direction: default_dir,
            ibl_intensity: 1.0,
            light_color: default_color,
            moon_intensity: 0.010,
            cascade_debug: false,
            shading_mode: 2 | 4 | 8,
            local_lights: Vec::new(),
            grid_pass,
            grid_enabled: false,
            postprocess_pass,
            atmosphere_pass,
            auto_exposure_pass,
            taa_pass,
            gtao_pass,
            bloom_pass,
            dof_pass,
            raytrace_pass,
            rt_debug_pass,
            restir_pass,
            restir_gi_pass,
            lighting_extra_pass,
            ddgi_pass,
            water_reflection_pass,
            velocity_pass,
            motion_blur_pass: crate::pass::motion_blur::MotionBlurPass::new(
                &ctx.device,
                &shaders,
                HDR_FORMAT,
                ctx.config.width,
                ctx.config.height,
            ),
            cas_pass: crate::pass::cas::CasPass::new(
                &ctx.device,
                &shaders,
                ctx.config.format,
                ctx.config.width,
                ctx.config.height,
            ),
            present_pass: crate::pass::present::PresentPass::new(
                &ctx.device,
                &shaders,
                ctx.config.format,
                ctx.config.width,
                ctx.config.height,
            ),
            fsr_pass: crate::pass::fsr::FsrPass::new(
                &ctx.device,
                &shaders,
                &ctx.queue,
                ctx.config.width,
                ctx.config.height,
                ctx.config.width,
                ctx.config.height,
            ),
            ldr_width: ctx.config.width,
            ldr_height: ctx.config.height,
            shadow_radius_threshold: std::env::var("SOMNIUM_SHADOW_RADIUS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.01),
            // EV100 15 (direct sunlight): 1 / (1.2 * 2^15). The renderer cannot
            // call into somnium_core for this — core depends on the renderer,
            // not the other way round — so the value is inlined.
            exposure: 1.0 / (1.2 * 32768.0),
            auto_exposure: true,
            frame_delta_time: 1.0 / 60.0,
            exposure_compensation: 0.0,
            grading: crate::pass::postprocess::Grading::default(),
            sun_angular_radius: 0.004_654,
            view_proj_unjittered: glam::Mat4::IDENTITY,
            render_width: ctx.config.width,
            render_height: ctx.config.height,
            tonemapper: 0,
            vignette_strength: 0.0,
            chromatic_aberration: 0.0,
            fxaa_pass: crate::pass::fxaa::FxaaPass::new(
                &ctx.device,
                &shaders,
                ctx.config.format,
                ctx.config.width,
                ctx.config.height,
            ),
            fxaa_enabled: true,
            oit_pass: crate::pass::oit::OitPass::new(
                &ctx.device,
                &shaders,
                HDR_FORMAT,
                ctx.config.width,
                ctx.config.height,
            ),
            smaa_pass: crate::pass::smaa::SmaaPass::new(
                &ctx.device,
                &shaders,
                ctx.config.format,
                ctx.config.width,
                ctx.config.height,
            ),
            gizmo_pass,
            gizmo_mode: GizmoMode::Translate,
            gizmo_world_pos: None,
            gizmo_world_rotation: glam::Quat::IDENTITY,
            light_gizmo_pass,
            light_gizmo_queue: Vec::new(),
            line_gizmo_queue: Vec::new(),
            light_gizmos_enabled: true,
            editor_overlays_enabled: true,
            game_ui_empty_warned: false,
            outline_pass,
            outline_entity: None,
            particle_pass,
            pending_particles: Vec::new(),
            indirect: crate::indirect::IndirectDrawBuffer::new(&ctx.device),
            transparent_pass,
            transparent_queue: Vec::new(),
            material_blend: Vec::new(),
            material_double_sided: Vec::new(),
            single_sided_args: 0,
            ibl_pass,
            volumetric_pass,
            cloud_pass,
            decal_grid,
            decals: Vec::new(),
            hiz_pass,
            hiz_ready: false,
            cull_stats: std::env::var("SOMNIUM_CULL_STATS").is_ok_and(|v| v == "1"),
            cull_stats_buffers: None,
            occlusion_off: std::env::var("SOMNIUM_NO_OCCLUSION").is_ok_and(|v| v == "1"),
            cull_pass: crate::pass::cull::CullPass::new(&ctx.device, &shaders),
            cull_aabbs: Vec::with_capacity(256),
            cluster_args: Vec::with_capacity(256),
            instanced_counts: std::collections::HashMap::new(),
            meshlet_draws: !std::env::var("SOMNIUM_NO_MESHLETS").is_ok_and(|v| v == "1"),
            culling_enabled: true,
            cpu_frustum_cull: !cpu_frustum_env_off(),
            cascade_caster_cull: !cascade_cull_env_off(),
            cascade_view_projs: [glam::Mat4::IDENTITY; crate::shadow::NUM_CASCADES],
            rebuilt_chunks: Vec::with_capacity(32),
            terrain_lod_by_vertex: std::collections::HashMap::new(),
            shadow_only_queue: Vec::with_capacity(256),
            shadow_caster_scratch: Vec::with_capacity(256),
            gpu_driven: ctx.supports_gpu_driven(),
            supports_gpu_driven: ctx.supports_gpu_driven(),
            supports_counted_draws: ctx.supports_counted_draws(),
            water_pass,
            water_queue: Vec::new(),
            underwater_pass,
            underwater_body: None,
            camera_submersion: 0.0,
            terrain_materials,
            // Every other debug lever in this renderer can be set from the
            // environment; the shading debug view could only be set from the
            // editor menu, which a headless capture has no way to click. The
            // codes are `somnium_ui::debug::VIEWS`.
            shading_debug: std::env::var("SOMNIUM_SHADING_DEBUG")
                .ok()
                .and_then(|v| v.trim().parse::<f32>().ok())
                .unwrap_or(0.0),
            frame_view_state: FrameViewState::default(),
            view_stage,
            view_slot: 0,
            views: Vec::new(),
            debug_toggles: somnium_ui::debug::DebugToggles::from_env(),
            capture: crate::capture::FrameCapture::from_env(),
            profiler: crate::profiler::GpuProfiler::new(&ctx.device, &ctx.queue, ctx.features),
            timing: crate::timing::TimingRun::from_env(),
            census_pass,
            classify_pass,
            // 150 m: DOOM-B put 63.9% of Coastal ground inside 100 m and 54.9%
            // of the overview between 100 and 400 m, so a split in that gap is
            // where the two viewpoints genuinely differ.
            aerial_split: std::env::var("SOMNIUM_AERIAL_SPLIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &f32| v.is_finite() && *v > 0.0)
                .unwrap_or(150.0),
            aerial_split_enabled: std::env::var("SOMNIUM_AERIAL").as_deref() == Ok("1"),
            aerial_hero_bank: std::env::var("SOMNIUM_AERIAL_HERO").as_deref() == Ok("1"),
            aerial_split_active: false,
            shade_ablate: crate::pass::shading::ablate::from_env(),
            base_extent: (ctx.config.width.max(1), ctx.config.height.max(1)),
            dynamic_resolution: crate::viewport_resolution::DynamicResolution::default(),
            terrain_material_ids: std::collections::HashSet::new(),
            terrains: Vec::new(),
            clipmaps: Vec::new(),
            clipmap_pass,
            terrain_queue: Vec::with_capacity(4),
            draw_queue: Vec::with_capacity(256),

            water_textures_bind_group,
            water_bodies: Default::default(),
            // Shader ownership and its DREAMS sampling resource stay together.
            shaders,
            grain_masks,
        }
    }

    /// Add a texture to the global bindless pool.
    pub fn add_texture(&mut self, ctx: &RenderContext, view: wgpu::TextureView) -> u32 {
        let index = self.texture_pool.add_texture(view.clone());
        self.global_pool.texture_views[index as usize] = view;
        self.global_pool.update_textures(&ctx.device);
        index
    }

    /// Upload worker-decoded RGBA8 pixels into the bindless pool. Material
    /// asset jobs use this main-thread half after file IO and decode complete.
    pub fn upload_material_texture(
        &mut self,
        ctx: &RenderContext,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> i32 {
        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Material Asset Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let row_bytes = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_row = row_bytes.div_ceil(align) * align;
        let upload = if padded_row == row_bytes {
            rgba.to_vec()
        } else {
            let mut padded = vec![0_u8; padded_row as usize * height as usize];
            for row in 0..height as usize {
                let source = row * row_bytes as usize;
                let target = row * padded_row as usize;
                padded[target..target + row_bytes as usize]
                    .copy_from_slice(&rgba[source..source + row_bytes as usize]);
            }
            padded
        };
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &upload,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.add_texture(
            ctx,
            texture.create_view(&wgpu::TextureViewDescriptor::default()),
        ) as i32
    }

    /// Upload a `LoadedScene` to the GPU pools and return one `UploadedNode` per
    /// renderable node (mesh_index is Some). The caller can then spawn ECS entities
    /// using the returned data.
    pub fn upload_scene(
        &mut self,
        ctx: &RenderContext,
        scene: &somnium_asset::LoadedScene,
    ) -> Vec<UploadedNode> {
        // 1. Textures --------------------------------------------------------
        //
        // **Only colour maps are sRGB** (Phase 17E remainder). Every imported
        // texture used to be uploaded as `Rgba8UnormSrgb`, including normal,
        // metallic-roughness/ARM and occlusion maps — none of which are colour.
        // The sRGB decode then bent all of them: an authored roughness of 0.5
        // arrives as ~0.21, so *every imported material read glossier than it
        // was made*, which is what the 17E note recorded as "bark roughness";
        // normal maps were skewed the same way, weakening all surface detail.
        //
        // Usage comes from the materials rather than from the file, because
        // glTF images carry no colour-space flag — how a texture is referenced
        // is the only thing that says what it means.
        let mut is_colour = vec![false; scene.textures.len()];
        for m in &scene.materials {
            for slot in [m.albedo_map, m.emissive_map] {
                if let Some(i) = slot {
                    if let Some(flag) = is_colour.get_mut(i) {
                        *flag = true;
                    }
                }
            }
        }

        let texture_indices: Vec<Option<i32>> = scene
            .textures
            .iter()
            .enumerate()
            .map(|(tex_index, tex)| {
                // Full mip chain. Without it, minified textures alias badly — the
                // sampler asks for trilinear filtering but a single level leaves
                // nothing to filter between, so detailed materials shimmer at
                // distance and read as noise.
                let levels = build_mip_chain(&tex.data, tex.width, tex.height);
                let mip_level_count = levels.len() as u32;

                let wgpu_tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Scene Texture"),
                    size: wgpu::Extent3d {
                        width: tex.width,
                        height: tex.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: if is_colour.get(tex_index).copied().unwrap_or(true) {
                        wgpu::TextureFormat::Rgba8UnormSrgb
                    } else {
                        wgpu::TextureFormat::Rgba8Unorm
                    },
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });

                for (level, (lw, lh, data)) in levels.iter().enumerate() {
                    // write_texture requires rows padded to COPY_BYTES_PER_ROW_ALIGNMENT.
                    let row_bytes = lw * 4;
                    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
                    let padded_row = row_bytes.div_ceil(align) * align;

                    let upload: std::borrow::Cow<[u8]> = if padded_row == row_bytes {
                        std::borrow::Cow::Borrowed(data)
                    } else {
                        let mut buf = vec![0u8; padded_row as usize * *lh as usize];
                        for row in 0..*lh as usize {
                            let src = row * row_bytes as usize;
                            let dst = row * padded_row as usize;
                            buf[dst..dst + row_bytes as usize]
                                .copy_from_slice(&data[src..src + row_bytes as usize]);
                        }
                        std::borrow::Cow::Owned(buf)
                    };

                    ctx.queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &wgpu_tex,
                            mip_level: level as u32,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        &upload,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(padded_row),
                            rows_per_image: Some(*lh),
                        },
                        wgpu::Extent3d {
                            width: *lw,
                            height: *lh,
                            depth_or_array_layers: 1,
                        },
                    );
                }

                let view = wgpu_tex.create_view(&wgpu::TextureViewDescriptor::default());
                Some(self.add_texture(ctx, view) as i32)
            })
            .collect();

        // 2. Materials -------------------------------------------------------
        let resolve_tex = |opt: Option<usize>| -> i32 {
            opt.and_then(|i| texture_indices.get(i).and_then(|&t| t))
                .unwrap_or(-1)
        };

        let material_ids: Vec<u32> = scene
            .materials
            .iter()
            .map(|mat| {
                let id = self.materials_pool.add_material(
                    &ctx.queue,
                    GpuMaterial {
                        base_color: mat.base_color,
                        roughness: mat.roughness,
                        metallic: mat.metallic,
                        albedo_map: resolve_tex(mat.albedo_map),
                        normal_map: resolve_tex(mat.normal_map),
                        metallic_roughness_map: resolve_tex(mat.metallic_roughness_map),
                        occlusion_map: resolve_tex(mat.occlusion_map),
                        transmission: mat.transmission,
                        emissive: mat.emissive,
                        emissive_map: resolve_tex(mat.emissive_map),
                        terrain_index: -1,
                        porosity: 0.5,
                        _pad: 0.0,
                        // Phase 17D: only MASK cuts out. OPAQUE ignores alpha entirely
                        // and BLEND goes to the forward pass, so a cutoff on either
                        // would punch holes in geometry that should be solid.
                        alpha_cutoff: crate::material::pool::cutout_threshold(
                            mat.alpha_mode,
                            mat.alpha_cutoff,
                        ),
                        flags: if mat.double_sided {
                            crate::material::pool::MATERIAL_FLAG_DOUBLE_SIDED
                        } else {
                            0
                        } | if mat.foliage {
                            crate::material::pool::MATERIAL_FLAG_FOLIAGE
                        } else {
                            0
                        },
                    },
                );
                // Phase 17D: remember double-sidedness so the visibility pass can
                // draw those instances with back-face culling switched off.
                self.set_material_double_sided(id, mat.double_sided);
                // Phase 21: remember which materials are blended so `submit` can
                // route their draws to the forward transparent pass.
                self.set_material_blend(id, mat.alpha_mode == somnium_asset::AlphaMode::Blend);
                id
            })
            .collect();

        // 3. Meshes ----------------------------------------------------------
        let mesh_allocs: Vec<crate::geometry::MeshAllocation> = scene
            .meshes
            .iter()
            .map(|mesh| {
                let alloc = self
                    .geometry
                    .upload_mesh(&ctx.queue, &mesh.vertices, &mesh.indices, 0);
                // Phase 24J: a BLAS describes geometry in object space, so it
                // is built once here and then referenced by however many
                // instances place it in the world.
                self.raytrace_pass.register_mesh(
                    &ctx.device,
                    alloc.vertex_offset,
                    mesh.vertices.len() as u32,
                    alloc.index_offset,
                    alloc.index_count,
                );
                alloc
            })
            .collect();

        // 4. Build UploadedNode list ----------------------------------------
        scene
            .nodes
            .iter()
            .filter_map(|node| {
                let mesh_idx = node.mesh_index?;
                let alloc = mesh_allocs.get(mesh_idx)?;
                let mat_idx = node.material_index.unwrap_or(0);
                let mat_id = material_ids.get(mat_idx).copied().unwrap_or(0);
                Some(UploadedNode {
                    entity_name: node.name.clone(),
                    vertex_offset: alloc.vertex_offset,
                    index_offset: alloc.index_offset,
                    index_count: alloc.index_count,
                    material_id: mat_id,
                    material_index: mat_idx,
                    transform: node.transform,
                })
            })
            .collect()
    }

    /// Current internal 3D target size (may be smaller than the swapchain).
    pub fn scene_extent(&self) -> (u32, u32) {
        (self.render_width, self.render_height)
    }

    /// Set the camera matrices for this frame.
    ///
    /// `view`  — world-to-camera (look_at matrix)
    /// `proj`  — camera projection (perspective / orthographic)
    /// `camera_pos` — camera world-space position
    pub fn set_view(&mut self, view: glam::Mat4, proj: glam::Mat4, camera_pos: glam::Vec3) {
        self.view_matrix = view;
        self.proj_matrix = proj;
        // Unjittered, for TAA's own reprojection: it has to compare like with
        // like across frames, and each frame's jitter is different.
        self.view_proj_unjittered = proj * view;

        // Phase 24F: nudge the projection by a sub-pixel offset so successive
        // frames sample the scene at slightly different positions. FSR and TAA
        // both apply that offset on z_axis (Bevy's wgpu convention).
        let jitter = if self.fsr_pass.enabled {
            self.fsr_pass
                .jitter_ndc(self.render_width, self.render_height)
        } else {
            self.taa_pass
                .jitter_ndc(self.render_width, self.render_height)
        };
        // Bevy / AMD jitter-space: NDC offset on z_axis. `translate * proj`
        // inverts it on perspective_rh (z_axis.w == −1).
        let mut jittered = proj;
        jittered.z_axis.x += jitter.x;
        jittered.z_axis.y += jitter.y;

        self.view_proj = jittered * view;
        self.camera_pos = camera_pos;
    }

    /// The view uniform block, exactly as the shaders read it.
    fn view_buffer_bytes(&self, view_proj: glam::Mat4) -> Vec<u8> {
        let inv_view_proj = view_proj.inverse();
        let debug_flag = if self.cascade_debug { 1.0f32 } else { 0.0f32 };
        let mut view_data = Vec::with_capacity(224);
        view_data.extend_from_slice(bytemuck::bytes_of(&view_proj.to_cols_array()));
        view_data.extend_from_slice(bytemuck::bytes_of(&inv_view_proj.to_cols_array()));
        view_data.extend_from_slice(bytemuck::bytes_of(&self.view_matrix.to_cols_array()));
        view_data.extend_from_slice(bytemuck::bytes_of(&self.camera_pos.to_array()));
        view_data.extend_from_slice(bytemuck::bytes_of(&debug_flag));
        view_data.extend_from_slice(bytemuck::bytes_of(&self.time));
        view_data.extend_from_slice(bytemuck::bytes_of(&[0.0f32; 3]));
        view_data
    }

    fn write_view_buffer(&self, queue: &wgpu::Queue, view_proj: glam::Mat4) {
        let view_data = self.view_buffer_bytes(view_proj);
        queue.write_buffer(&self.global_pool.view_proj_buffer, 0, &view_data);
    }

    /// Put one view's matrices where the *encoder* will pick them up.
    ///
    /// `Queue::write_buffer` cannot be used for this and the reason is the
    /// whole of MORROWIND-J step 3's first bug: staged writes are applied at
    /// the **start of the submit**, not in the order they were issued among
    /// encoder commands. With one submit per frame that means the last write of
    /// the frame is what every pass in it sees — so four views written this way
    /// all render with the fourth camera, and the four tiles come out
    /// identical. (They came out identical with the *first* camera, in fact,
    /// because the overlay pass writes the primary matrix after the loop and
    /// that write is the last one.)
    ///
    /// Staging plus `copy_buffer_to_buffer` puts the update *into* the command
    /// stream, where it is ordered against the passes that read it.
    fn stage_view_buffer(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        slot: u64,
        view_proj: glam::Mat4,
    ) {
        let data = self.view_buffer_bytes(view_proj);
        let offset = (slot % VIEW_SLOTS) * VIEW_SLOT_BYTES;
        queue.write_buffer(&self.view_stage, offset, &data);
        encoder.copy_buffer_to_buffer(
            &self.view_stage,
            offset,
            &self.global_pool.view_proj_buffer,
            0,
            data.len() as u64,
        );
    }

    /// Scene-wide indirect-light strength (Phase 22C), uploaded with the sun.
    pub fn set_ibl_intensity(&mut self, intensity: f32) {
        self.ibl_intensity = intensity.max(0.0);
    }

    /// Directional moonlight illuminance in lux (Phase 25M-2).
    pub fn set_moon_intensity(&mut self, intensity: f32) {
        self.moon_intensity = intensity.max(0.0);
    }

    /// Colour the HDR target is cleared to, in cd/m² (Phase 24A).
    ///
    /// This used to be a flat 0.07 grey, which was fine when light was an
    /// arbitrary multiplier and became black the moment exposure turned
    /// physical — 0.07 cd/m² against a 100 000 lux scene is night.
    ///
    /// Anywhere no geometry is drawn is sky, so the clear takes the same
    /// horizon colour and sun-driven luminance scale that `ibl_gen.wgsl` gives
    /// the environment cubemap. The background then matches the light the scene
    /// is actually receiving, and darkens with the sun instead of staying put.
    ///
    /// Interim: Phase 24C draws the atmosphere properly and this goes away.
    fn background_color(&self) -> wgpu::Color {
        // Matches `horizon_color` in ibl_gen.wgsl.
        const HORIZON: glam::Vec3 = glam::Vec3::new(0.5, 0.7, 0.9);
        const SKY_LUMINANCE_PER_LUX: f32 = 0.08;

        let illuminance = self
            .light_color
            .dot(glam::Vec3::new(0.2126, 0.7152, 0.0722));
        let sky = HORIZON * (illuminance * SKY_LUMINANCE_PER_LUX);
        wgpu::Color {
            r: f64::from(sky.x),
            g: f64::from(sky.y),
            b: f64::from(sky.z),
            a: 1.0,
        }
    }

    /// Set the directional light parameters for this frame.
    pub fn set_directional_light(&mut self, direction: glam::Vec3, color: glam::Vec3) {
        self.light_direction = direction.normalize();
        self.light_color = color;
    }

    /// Preserve the per-light authored shadow choice.
    ///
    /// `Virtual` resolves to the safe CSM branch until
    /// [`Self::directional_shadow_technique`] reports every GPU stage ready.
    pub fn set_directional_shadow_policy(
        &mut self,
        policy: crate::shadow::virtual_map::ShadowLightPolicy,
    ) {
        self.directional_shadow_policy = policy;
    }

    /// Lazily allocate the physical page pool and shader-visible page table.
    /// A CSM-only scene never calls this and pays no VSM memory cost.
    pub fn enable_virtual_shadow_resources(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: crate::shadow::virtual_map::VirtualShadowConfig,
    ) -> Result<(), &'static str> {
        let cache = crate::shadow::virtual_map::VirtualShadowMap::new(config)?;
        let gpu = crate::shadow::virtual_map::VirtualShadowGpu::new(device, queue, config)?;
        self.shading_pass
            .set_virtual_shadow_resources(device, &self.vis_pass.view, &gpu);
        self.water_pass.set_virtual_shadow_resources(&gpu);
        self.virtual_shadow_cache = cache;
        self.virtual_shadow_gpu = Some(gpu);
        self.virtual_shadow_readiness = crate::shadow::virtual_map::VirtualShadowReadiness {
            gpu_resources: true,
            page_raster: true,
            shading_sample: true,
        };
        Ok(())
    }

    /// Effective production branch, after capability/completeness fallback.
    #[must_use]
    pub fn directional_shadow_technique(&self) -> crate::shadow::virtual_map::ShadowTechnique {
        self.directional_shadow_policy
            .effective(self.virtual_shadow_readiness)
    }

    /// Invalidate cached shadow depth after an in-place caster geometry edit.
    /// Transform/index/range changes are detected automatically each frame.
    pub fn invalidate_shadow_casters(&mut self) {
        self.shadow_caster_content_revision = self.shadow_caster_content_revision.wrapping_add(1);
    }

    /// Add a local light (Point or Spot) for this frame (Phase 13C).
    pub fn submit_local_light(&mut self, light: crate::cluster::GpuLocalLight) {
        if self.local_lights.len() < crate::cluster::MAX_LOCAL_LIGHTS {
            self.local_lights.push(light);
        }
    }

    /// Toggle the cascade debug overlay (tints pixels by cascade index).
    /// Press `C` in hello_engine to activate.
    pub fn set_cascade_debug(&mut self, enabled: bool) {
        self.cascade_debug = enabled;
    }

    /// Show or hide the editor infinite-grid overlay (Phase 11.5H).
    /// The grid is off by default; toggle with `G` in the editor.
    pub fn set_grid_enabled(&mut self, enabled: bool) {
        self.grid_enabled = enabled;
    }

    /// Toggle the editor grid overlay and return the new state.
    pub fn toggle_grid(&mut self) -> bool {
        self.grid_enabled = !self.grid_enabled;
        self.grid_enabled
    }

    /// Set the active gizmo mode (Translate / Rotate / Scale).
    pub fn set_gizmo_mode(&mut self, mode: GizmoMode) {
        self.gizmo_mode = mode;
    }

    /// Show or suppress editor-only viewport overlays without losing their
    /// selection/toggle state. Used by Play-in-editor.
    pub fn set_editor_overlays_enabled(&mut self, enabled: bool) {
        self.editor_overlays_enabled = enabled;
    }

    /// Whether editor-only viewport overlays may be drawn or picked.
    /// The view-projection to use for cursor picking.
    ///
    /// **Not** [`Self::view_proj`], which carries the per-frame sub-pixel
    /// jitter FSR and TAA need. Editor overlays — the gizmo included — are
    /// drawn with the unjittered matrix, so picking must use it too or the
    /// ray and the arrow the user aimed at disagree by the jitter.
    #[must_use]
    pub fn picking_view_proj(&self) -> glam::Mat4 {
        self.view_proj_unjittered
    }

    pub fn editor_overlays_enabled(&self) -> bool {
        self.editor_overlays_enabled
    }

    /// Toggle the Phase 15 GPU-driven indirect draw path, returning the new
    /// state. Returns `false` (and does nothing) if the device lacks support.
    ///
    /// Both paths must produce an identical image, so this doubles as an A/B
    /// check: flip it and the scene should not change.
    pub fn toggle_gpu_driven(&mut self) -> bool {
        if !self.supports_gpu_driven {
            return false;
        }
        self.gpu_driven = !self.gpu_driven;
        self.gpu_driven
    }

    /// Toggle GPU frustum culling, returning the new state (Phase 15B).
    ///
    /// A correct cull is invisible, so this is the way to check it: flip it and
    /// nothing on screen should change. Anything that pops in or out was being
    /// culled wrongly.
    pub fn toggle_culling(&mut self) -> bool {
        self.culling_enabled = !self.culling_enabled;
        self.culling_enabled
    }

    /// Toggle CPU camera-frustum early-out (Phase CR-B). Independent of F10.
    pub fn toggle_cpu_frustum(&mut self) -> bool {
        self.set_cpu_frustum(!self.cpu_frustum_cull)
    }

    /// Apply the Camera Details checkbox. `SOMNIUM_CPU_FRUSTUM=0` wins.
    /// Apply the Camera entity's Phase DOOM-F settings.
    ///
    /// Switching the controller **off** resizes back to the base extent
    /// immediately rather than leaving the last scale frozen — otherwise
    /// unticking the box would appear to do nothing until the next window
    /// resize, which reads as a broken control.
    pub fn set_dynamic_resolution(
        &mut self,
        ctx: &RenderContext,
        enabled: bool,
        target_ms: f32,
        floor: f32,
    ) {
        let was = self.dynamic_resolution.enabled;
        self.dynamic_resolution.enabled = enabled;
        if target_ms.is_finite() && target_ms > 0.0 {
            self.dynamic_resolution.target_ms = target_ms;
        }
        if floor.is_finite() {
            self.dynamic_resolution.min_scale = floor.clamp(0.25, 1.0);
        }
        if was && !enabled {
            self.dynamic_resolution.reset();
            let (w, h) = self.base_extent;
            self.resize_targets(ctx, w, h);
        }
    }

    /// Current dynamic-resolution scale, 1.0 when the controller is off.
    #[must_use]
    pub fn dynamic_resolution_scale(&self) -> f32 {
        self.dynamic_resolution.scale()
    }

    pub fn set_cpu_frustum(&mut self, on: bool) -> bool {
        if cpu_frustum_env_off() {
            self.cpu_frustum_cull = false;
            return false;
        }
        self.cpu_frustum_cull = on;
        self.cpu_frustum_cull
    }

    /// Whether terrain vis enqueue will run the CPU AABB test this frame.
    pub fn cpu_frustum_active(&self) -> bool {
        self.cpu_frustum_cull && !cpu_frustum_env_off()
    }

    /// `SOMNIUM_CPU_FRUSTUM=0` forces the CPU early-out off.
    pub fn cpu_frustum_env_off() -> bool {
        cpu_frustum_env_off()
    }

    /// Whether the GPU-driven indirect path is currently in use.
    pub fn gpu_driven(&self) -> bool {
        self.gpu_driven
    }

    /// Whether this device supports the GPU-driven path at all.
    pub fn supports_gpu_driven(&self) -> bool {
        self.supports_gpu_driven
    }

    /// DOOM-G counted submission. The measured small-scene result was inside
    /// noise, so dense Phase-15 submission stays the default and this remains
    /// an explicit experiment.
    fn counted_draws_active(&self) -> bool {
        self.gpu_driven
            && self.supports_counted_draws
            && std::env::var("SOMNIUM_DRAW_COMPACTION").as_deref() == Ok("1")
    }

    /// Submit one light's gizmo for this frame (Phase 13E).
    ///
    /// Cleared every frame like the draw queue; the editor re-submits each
    /// light it wants visualized.
    pub fn submit_light_gizmo(&mut self, desc: crate::pass::light_gizmo::LightGizmoDesc) {
        self.light_gizmo_queue.push(desc);
    }

    /// Queue raw line-list vertices for this frame's editor overlay.
    ///
    /// Pairs, in world space. Cleared with the rest of the gizmo queues at the
    /// end of the frame, so a caller submits every frame it wants them.
    pub fn submit_gizmo_lines(
        &mut self,
        vertices: impl IntoIterator<Item = crate::pass::light_gizmo::LineVertex>,
    ) {
        self.line_gizmo_queue.extend(vertices);
    }

    /// Show or hide light gizmos (on by default).
    pub fn set_light_gizmos_enabled(&mut self, enabled: bool) {
        self.light_gizmos_enabled = enabled;
    }

    /// Toggle light gizmos and return the new state.
    pub fn toggle_light_gizmos(&mut self) -> bool {
        self.light_gizmos_enabled = !self.light_gizmos_enabled;
        self.light_gizmos_enabled
    }

    /// Whether light gizmos are currently drawn.
    pub fn light_gizmos_enabled(&self) -> bool {
        self.light_gizmos_enabled
    }

    /// Update the world-space position the gizmo should appear at.
    pub fn set_gizmo_world_pos(&mut self, pos: glam::Vec3) {
        self.gizmo_world_pos = Some(pos);
    }

    /// Update both halves of the transform gizmo's world-space frame.
    pub fn set_gizmo_world_transform(&mut self, pos: glam::Vec3, rotation: glam::Quat) {
        self.gizmo_world_pos = Some(pos);
        self.gizmo_world_rotation = rotation.normalize();
    }

    /// Hide the gizmo (e.g. when no entity is selected).
    pub fn clear_gizmo(&mut self) {
        self.gizmo_world_pos = None;
    }

    /// Set the selected entity's mesh data for outline rendering (Phase 11.5I).
    pub fn set_outline_entity(
        &mut self,
        vertex_offset: u32,
        index_offset: u32,
        index_count: u32,
        model: glam::Mat4,
    ) {
        self.outline_entity = Some((vertex_offset, index_offset, index_count, model));
    }

    /// Clear the selection outline (call when nothing is selected or selection has no mesh).
    pub fn clear_outline(&mut self) {
        self.outline_entity = None;
    }

    /// Replace this frame's particle list (Phase 11.5J).
    ///
    /// Called once per frame from `app.rs` after CPU simulation; the renderer
    /// uploads the data and draws instanced billboards in `render()`.
    pub fn set_particles(&mut self, particles: Vec<GpuParticle>) {
        self.pending_particles = particles;
    }

    /// Resize all internal renderer targets.
    /// Copy the indirect args into the phase's staging buffer.
    fn snapshot_indirect(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        phase: usize,
    ) {
        let bytes = (self.indirect.len() * 16) as u64;
        let needs_alloc = self
            .cull_stats_buffers
            .as_ref()
            .is_none_or(|b| b[phase].size() < bytes);
        if needs_alloc {
            let make = || {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Cull Stats Readback"),
                    size: bytes.max(16),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                })
            };
            self.cull_stats_buffers = Some([make(), make()]);
        }
        let dst = &self.cull_stats_buffers.as_ref().unwrap()[phase];
        encoder.copy_buffer_to_buffer(&self.indirect.buffer, 0, dst, 0, bytes);
    }

    /// Map both snapshots and log how many draws each phase left alive.
    ///
    /// `instance_count` doubles as the cull verdict, so counting the non-zero
    /// entries is exactly the number of draws that phase submitted.
    fn report_cull_stats(&self, ctx: &RenderContext, draw_count: usize) {
        let Some(buffers) = &self.cull_stats_buffers else {
            return;
        };
        let bytes = (draw_count * 16) as u64;
        let mut alive = [0usize; 2];
        // Draw counts alone cannot be compared between the whole-mesh and
        // cluster paths — a mesh and a 128-triangle cluster are not the same
        // unit of work. Summing the indices actually submitted is.
        let mut indices = [0u64; 2];

        for (phase, buf) in buffers.iter().enumerate() {
            let slice = buf.slice(0..bytes);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
            {
                let data = slice
                    .get_mapped_range()
                    .expect("indirect readback mapped by the poll above");
                // DrawIndirectArgs: vertex_count, instance_count, first_vertex,
                // first_instance — instance_count is the second u32.
                for a in data.chunks_exact(16) {
                    if u32::from_le_bytes([a[4], a[5], a[6], a[7]]) != 0 {
                        alive[phase] += 1;
                        indices[phase] += u32::from_le_bytes([a[0], a[1], a[2], a[3]]) as u64;
                    }
                }
            }
            buf.unmap();
        }

        tracing::info!(
            "CULLSTATS total={draw_count} phase1_drawn={} phase2_drawn={} culled={} tris_drawn={}",
            alive[0],
            alive[1],
            draw_count - alive[0] - alive[1],
            (indices[0] + indices[1]) / 3,
        );
    }

    /// Record one visibility pass.
    ///
    /// `clear` distinguishes the two occlusion phases: phase one clears the
    /// targets, phase two loads them so it adds to what phase one drew rather
    /// than starting over.
    fn record_visibility(&self, encoder: &mut wgpu::CommandEncoder, clear: bool) {
        let color_load = if clear {
            wgpu::LoadOp::Clear(wgpu::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            })
        } else {
            wgpu::LoadOp::Load
        };
        let depth_load = if clear {
            wgpu::LoadOp::Clear(1.0)
        } else {
            wgpu::LoadOp::Load
        };

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Visibility Pass"),
            multiview_mask: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.vis_pass.view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: color_load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.vis_pass.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: depth_load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        rpass.set_bind_group(0, &self.global_pool.bind_group, &[]);
        rpass.set_bind_group(1, &self.vis_pass.cutout_bind_group, &[]);

        if self.gpu_driven && !self.indirect.is_empty() {
            let total = self.indirect.len();
            let split = self.single_sided_args.min(total);
            if self.counted_draws_active() {
                // DOOM-G: the cull phase copied only survivors into the two
                // fixed partitions and authored their counts on the GPU. Dense
                // args stay untouched as the phase-two/diagnostic contract.
                if split > 0 {
                    rpass.set_pipeline(&self.vis_pass.pipeline);
                    rpass.multi_draw_indirect_count(
                        self.cull_pass.compact_buffer(),
                        0,
                        self.cull_pass.count_buffer(),
                        0,
                        split as u32,
                    );
                }
                if total > split {
                    rpass.set_pipeline(&self.vis_pass.pipeline_two_sided);
                    rpass.multi_draw_indirect_count(
                        self.cull_pass.compact_buffer(),
                        (split as u64) * crate::indirect::ARGS_SIZE,
                        self.cull_pass.count_buffer(),
                        std::mem::size_of::<u32>() as u64,
                        (total - split) as u32,
                    );
                }
            } else {
                // Phase 15A fallback: submit the dense stream. Culled entries
                // carry instance_count = 0 and cost no raster work.
                if split > 0 {
                    rpass.set_pipeline(&self.vis_pass.pipeline);
                    rpass.multi_draw_indirect(&self.indirect.buffer, 0, split as u32);
                }
                if total > split {
                    rpass.set_pipeline(&self.vis_pass.pipeline_two_sided);
                    rpass.multi_draw_indirect(
                        &self.indirect.buffer,
                        (split as u64) * crate::indirect::ARGS_SIZE,
                        (total - split) as u32,
                    );
                }
            }
        } else if clear {
            rpass.set_pipeline(&self.vis_pass.pipeline);
            // Fallback for devices without multi-draw indirect. There are no
            // indirect args to cull, so everything is drawn in phase one and
            // phase two has nothing to add.
            for (inst_id, cmd) in self.draw_queue.iter().enumerate() {
                rpass.draw(0..cmd.index_count, inst_id as u32..(inst_id as u32 + 1));
            }
        }
    }

    /// Resize the scene targets to the size the *editor* asked for.
    ///
    /// This is the authoritative base size — window size through the viewport
    /// resolution preset — and it is what Phase DOOM-F's dynamic scale is
    /// applied on top of. Recording it separately is what lets the controller
    /// return to native when it is switched off, and stops a scaled frame from
    /// being mistaken for a new base and scaled again.
    pub fn resize(&mut self, ctx: &RenderContext, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.base_extent = (width, height);
        }
        let (width, height) = self.dynamic_resolution.apply(width, height);
        self.resize_targets(ctx, width, height);
    }

    fn resize_targets(&mut self, ctx: &RenderContext, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.vis_pass.resize(
                &ctx.device,
                &self.shaders,
                width,
                height,
                &self.global_pool.layout,
            );
            // Bloom must resize first: PostProcess keeps its result view in the
            // final bind group, and the old view becomes stale here.
            self.bloom_pass.resize(&ctx.device, width, height);
            self.postprocess_pass
                .resize(&ctx.device, width, height, self.bloom_pass.result_view());
            self.auto_exposure_pass
                .resize(&ctx.device, &self.postprocess_pass.hdr_view);
            self.render_width = width;
            self.render_height = height;
            self.gtao_pass.resize(&ctx.device, width, height);
            // Phase DOOM-B/C: the vis buffer and depth are new textures now,
            // and a cached bind group would point at the old ones.
            self.census_pass.invalidate();
            self.classify_pass.invalidate();
            self.dof_pass.resize(&ctx.device, width, height);
            self.rt_debug_pass.invalidate();
            self.restir_pass.resize(&ctx.device, width, height);
            self.restir_gi_pass.resize(&ctx.device, width, height);
            self.lighting_extra_pass.resize(&ctx.device, width, height);
            // CONTROL-M: the quarter-res march target follows the scene, and
            // dropping its bind groups here is what makes the next frame
            // rebuild them against the new depth view rather than the old one.
            self.cloud_pass.resize(&ctx.device, width, height);
            self.shading_pass.resize(
                &ctx.device,
                &self.vis_pass.view,
                self.gtao_pass.output_view(),
                &self.vis_pass.depth_view,
                self.restir_pass
                    .visibility_view()
                    .expect("ReSTIR always allocates its visibility target"),
                self.restir_gi_pass
                    .radiance_view()
                    .expect("ReSTIR GI always allocates its radiance target"),
                self.lighting_extra_pass.aux_view(),
                self.lighting_extra_pass.volume_view(),
            );
            self.taa_pass.resize(&ctx.device, HDR_FORMAT, width, height);
            // Scene-sized, not window-sized: OIT accumulates at the resolution
            // the transparent pass rasterises at, which dynamic resolution and
            // the viewport preset both change.
            self.oit_pass.resize(&ctx.device, width, height);
            self.fxaa_pass
                .resize(&ctx.device, ctx.config.format, width, height);
            self.smaa_pass
                .resize(&ctx.device, ctx.config.format, width, height);
            self.cas_pass
                .resize(&ctx.device, ctx.config.format, width, height);
            self.present_pass
                .resize(&ctx.device, ctx.config.format, width, height);
            self.fsr_pass.resize(
                &ctx.device,
                &ctx.queue,
                width,
                height,
                ctx.config.width,
                ctx.config.height,
            );
            self.ldr_width = 0;
            self.ldr_height = 0;
            self.velocity_pass
                .resize(&ctx.device, &self.vis_pass.depth_view, width, height);
            self.water_pass.resize(&ctx.device, width, height);
            self.water_reflection_pass
                .resize(&ctx.device, width, height);
            self.underwater_pass.invalidate();
            self.taa_pass.rebuild(
                &ctx.device,
                &self.postprocess_pass.hdr_view,
                &self.vis_pass.depth_view,
                self.velocity_pass.view(),
                self.water_pass.surface_view(),
            );
            self.motion_blur_pass.resize(&ctx.device, width, height);
            self.outline_pass.resize(&ctx.device, width, height);
            // Must follow vis_pass: the level-0 bind group references its depth view.
            self.hiz_pass.resize(
                &ctx.device,
                &ctx.queue,
                width,
                height,
                &self.vis_pass.depth_view,
            );
            // The new texture is zero-filled, i.e. everything at the near
            // plane, so occlusion has to stand down until it is rebuilt.
            self.hiz_ready = false;
        }
    }

    /// FXAA / CAS / present follow the tone-map attachment. FSR tone-maps at
    /// display size; the TAA+blit path stays at scene size.
    fn ensure_ldr_size(&mut self, ctx: &RenderContext, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if self.ldr_width == width && self.ldr_height == height {
            return;
        }
        self.fxaa_pass
            .resize(&ctx.device, ctx.config.format, width, height);
        self.smaa_pass
            .resize(&ctx.device, ctx.config.format, width, height);
        self.cas_pass
            .resize(&ctx.device, ctx.config.format, width, height);
        self.present_pass
            .resize(&ctx.device, ctx.config.format, width, height);
        self.ldr_width = width;
        self.ldr_height = height;
    }

    /// Submit a draw command.
    ///
    /// Blended materials are routed to the forward transparent pass instead of
    /// the visibility buffer, which can only resolve one triangle per pixel.
    /// Callers do not need to know which is which.
    pub fn submit(&mut self, cmd: DrawCommand) {
        let blended = self
            .material_blend
            .get(cmd.material_id as usize)
            .copied()
            .unwrap_or(false);
        if blended {
            self.transparent_queue.push(cmd);
        } else {
            self.draw_queue.push(cmd);
        }
    }

    /// Record whether a material is alpha-blended, so `submit` can route it.
    /// Materials default to opaque when never registered.
    /// Record that `material_id` renders from both sides (Phase 17D).
    pub fn set_material_double_sided(&mut self, material_id: u32, double_sided: bool) {
        let idx = material_id as usize;
        if self.material_double_sided.len() <= idx {
            self.material_double_sided.resize(idx + 1, false);
        }
        self.material_double_sided[idx] = double_sided;
    }

    /// Whether `material_id` renders from both sides.
    fn is_double_sided(&self, material_id: u32) -> bool {
        self.material_double_sided
            .get(material_id as usize)
            .copied()
            .unwrap_or(false)
    }

    pub fn set_material_blend(&mut self, material_id: u32, blended: bool) {
        let idx = material_id as usize;
        if self.material_blend.len() <= idx {
            self.material_blend.resize(idx + 1, false);
        }
        self.material_blend[idx] = blended;
    }

    /// Submit a water rendering command.
    pub fn add_water(
        &mut self,
        water_id: u32,
        transform: glam::Mat4,
        water: crate::pass::water::WaterMaterialData,
        vertex_offset: u32,
        index_offset: u32,
        index_count: u32,
    ) {
        self.water_queue.push((
            water_id,
            transform,
            water,
            vertex_offset,
            index_offset,
            index_count,
        ));
    }

    /// Allocate a stable ECS-visible water handle.
    pub fn allocate_water_body_id(&mut self) -> u32 {
        self.water_bodies.allocate_id()
    }

    /// Create or restore renderer-owned mask/depth/SDF state for one body.
    pub fn ensure_water_body(
        &mut self,
        ctx: &RenderContext,
        descriptor: crate::water_body::WaterBodyDescriptor,
    ) -> Result<(), String> {
        if self.water_bodies.descriptor(descriptor.water_id) != Some(descriptor) {
            self.water_bodies.create_or_replace(ctx, descriptor)?;
            // The underwater bind group retains the body's mask/depth views.
            // Replacing a descriptor under the same stable ECS id allocates
            // new textures, so the cached group must not keep the old views.
            self.underwater_pass.invalidate();
        }
        Ok(())
    }

    /// Upload the compact mask-derived surface mesh for a water body.
    pub fn upload_water_body_mesh(
        &mut self,
        ctx: &RenderContext,
        water_id: u32,
    ) -> Result<crate::geometry::MeshAllocation, String> {
        let (vertices, indices) = self
            .water_bodies
            .get(water_id)
            .ok_or_else(|| format!("water body {water_id} is not loaded"))?
            .finite_mesh(2.0);
        if indices.is_empty() {
            return Err(format!("water body {water_id} mask produced no triangles"));
        }
        Ok(self
            .geometry
            .upload_mesh(&ctx.queue, &vertices, &indices, 0))
    }

    /// Query the same deterministic surface used by the GPU water shader.
    pub fn query_water_surface(
        &self,
        water_id: u32,
        terrain_local_xz: glam::Vec2,
        time: f32,
    ) -> Option<crate::water_body::WaterSurfaceSample> {
        self.water_bodies
            .sample_surface(water_id, terrain_local_xz, time)
    }

    /// Return the terrain-local deepest wet point for deterministic placement.
    pub fn deepest_water_point(&self, water_id: u32) -> Option<(glam::Vec2, f32)> {
        self.water_bodies.deepest_point(water_id)
    }

    /// Test whether a terrain-local point lies inside the displaced water volume.
    pub fn water_contains_point(
        &self,
        water_id: u32,
        terrain_local_point: glam::Vec3,
        time: f32,
    ) -> bool {
        self.water_bodies
            .contains_point(water_id, terrain_local_point, time)
    }

    /// Create a new heightmap terrain (Phase 14) and return its terrain id.
    ///
    /// Phase 25A-2 does three things here that the terrain pass used to do for
    /// itself: reserve a rewritable vertex span per chunk in the global pool,
    /// publish the splatmap and the twelve layer maps into the bindless texture
    /// array, and register a `GpuMaterial` so chunk draws look like every other
    /// draw to the rest of the renderer.
    pub fn create_terrain(
        &mut self,
        ctx: &RenderContext,
        desc: crate::terrain::TerrainDescriptor,
    ) -> u32 {
        self.create_terrain_inner(ctx, desc, false)
    }

    /// Same as [`Self::create_terrain`], but extra-bank layers 16–31 and splat
    /// maps 4–7 stay unbound. Island uses this so 16 authored materials is a
    /// real GPU budget, not only a CPU splat constraint.
    pub fn create_terrain_hero_bank(
        &mut self,
        ctx: &RenderContext,
        desc: crate::terrain::TerrainDescriptor,
    ) -> u32 {
        self.create_terrain_inner(ctx, desc, true)
    }

    fn create_terrain_inner(
        &mut self,
        ctx: &RenderContext,
        desc: crate::terrain::TerrainDescriptor,
        hero_bank_only: bool,
    ) -> u32 {
        let mut terrain = crate::terrain::TerrainData::new(
            &ctx.device,
            &ctx.queue,
            desc,
            ctx.supports_bc_compression(),
        );
        terrain.reserve_pool_spans(&mut self.geometry);
        if hero_bank_only {
            terrain.apply_hero_bank_gpu_budget();
        }

        // The layer maps are `texture_2d_array`s and the bindless array is
        // `texture_2d`, so each layer is published as its own single-layer view
        // of the same texture. No copy, no second upload.
        let layer_view = |tex: &wgpu::Texture, layer: u32, label: &str| {
            tex.create_view(&wgpu::TextureViewDescriptor {
                label: Some(label),
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            })
        };
        let mut ids = crate::terrain::TerrainTextureIds::default();
        ids.splat_maps = std::array::from_fn(|i| {
            if hero_bank_only && i >= 4 {
                -1
            } else {
                self.add_texture(ctx, terrain.splatmap.views[i].clone()) as i32
            }
        });
        ids.macro_map = self.add_texture(ctx, terrain.macro_view.clone()) as i32;
        // Phase TSUSHIMA-B/C. Registered once, like the macro map: the bake is
        // rewritten in place after a sculpt, so these three indices are valid
        // for the terrain's life and no bind group is ever invalidated.
        ids.horizon_maps = [
            self.add_texture(ctx, terrain.horizon_gpu.angles_a_view.clone()) as i32,
            self.add_texture(ctx, terrain.horizon_gpu.angles_b_view.clone()) as i32,
        ];
        ids.sky_visibility = self.add_texture(ctx, terrain.horizon_gpu.sky_view.clone()) as i32;
        ids.relief_normal = self.add_texture(ctx, terrain.relief_gpu.view.clone()) as i32;
        let hero = crate::terrain::textures::TERRAIN_HERO_LAYERS;
        // Virtual mode deliberately leaves every legacy layer id at -1. The
        // 4x4 arrays only keep the struct/fallback shape valid; publishing
        // them would make live shading sample black placeholders instead of
        // the mean-colour cold-cache fallback.
        if terrain.layer_textures.virtual_texture.is_none() {
            for layer in 0..hero {
                let i = layer as usize;
                ids.albedo[i] = self.add_texture(
                    ctx,
                    layer_view(
                        &terrain.layer_textures.albedo,
                        layer,
                        "Terrain Layer Albedo+Height",
                    ),
                ) as i32;
                ids.surface[i] = self.add_texture(
                    ctx,
                    layer_view(
                        &terrain.layer_textures.surface,
                        layer,
                        "Terrain Layer Surface",
                    ),
                ) as i32;
            }
            if !hero_bank_only {
                for layer in 0..(crate::terrain::textures::TERRAIN_LAYER_COUNT - hero) {
                    let i = (hero + layer) as usize;
                    ids.albedo[i] = self.add_texture(
                        ctx,
                        layer_view(
                            &terrain.layer_textures.albedo_extra,
                            layer,
                            "Terrain Layer Albedo+Height Extra",
                        ),
                    ) as i32;
                    ids.surface[i] = self.add_texture(
                        ctx,
                        layer_view(
                            &terrain.layer_textures.surface_extra,
                            layer,
                            "Terrain Layer Surface Extra",
                        ),
                    ) as i32;
                }
            }
        }
        if let Some(gpu) = &terrain.layer_textures.virtual_texture {
            ids.virtual_texture = [
                self.add_texture(ctx, gpu.albedo_view.clone()) as i32,
                self.add_texture(ctx, gpu.surface_view.clone()) as i32,
                self.add_texture(ctx, gpu.page_table_view.clone()) as i32,
                gpu.shader_atlas_size(),
            ];
        }
        terrain.texture_ids = ids;
        terrain.terrain_index = self.terrain_materials.allocate().unwrap_or(0);

        let clipmap = crate::terrain::clipmap::TerrainClipmap::new(&ctx.device);

        // The `GpuMaterial` itself is nearly empty: `terrain_index` is what
        // sends the shading pass down the splat path, and everything it would
        // otherwise read lives in the terrain-material entry.
        terrain.material_id = self.materials_pool.add_material(
            &ctx.queue,
            GpuMaterial {
                base_color: [1.0, 1.0, 1.0, 1.0],
                roughness: 0.9,
                metallic: 0.0,
                albedo_map: -1,
                normal_map: -1,
                metallic_roughness_map: -1,
                alpha_cutoff: 0.0,
                flags: 0,
                occlusion_map: -1,
                transmission: 0.0,
                emissive: [0.0; 3],
                emissive_map: -1,
                terrain_index: terrain.terrain_index as i32,
                porosity: 0.5,
                _pad: 0.0,
            },
        );
        // Opaque and single-sided, which is what an unregistered material
        // already defaults to — recorded explicitly so terrain does not depend
        // on that default staying put.
        self.set_material_blend(terrain.material_id, false);
        self.set_material_double_sided(terrain.material_id, false);
        self.terrain_material_ids.insert(terrain.material_id);

        self.terrains.push(terrain);
        self.clipmaps.push(clipmap);
        let last = self.clipmaps.len() - 1;
        let (detail_a, detail_s) = self.clipmaps[last].detail_sampled();
        let (macro_a, macro_s) = self.clipmaps[last].macro_sampled();
        let detail_a = detail_a.clone();
        let detail_s = detail_s.clone();
        let macro_a = macro_a.clone();
        let macro_s = macro_s.clone();
        self.shading_pass
            .set_clipmap_arrays(&ctx.device, &detail_a, &detail_s, &macro_a, &macro_s);
        (self.terrains.len() - 1) as u32
    }

    /// Queue a terrain for rendering this frame with the given model matrix.
    pub fn submit_terrain(&mut self, terrain_id: u32, model: glam::Mat4) {
        if (terrain_id as usize) < self.terrains.len() {
            self.terrain_queue.push((terrain_id, model));
        }
    }

    /// Wait until in-flight GPU work that still references scene textures is done.
    pub fn wait_gpu(&self, ctx: &RenderContext) {
        let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// Drop GPU terrains, clipmaps, and water so a map load does not leak slots.
    pub fn reset_scene_gpu(&mut self) {
        self.terrains.clear();
        self.clipmaps.clear();
        self.terrain_queue.clear();
        self.terrain_material_ids.clear();
        self.terrain_materials.reset();
        self.water_bodies.clear();
        self.underwater_body = None;
        self.underwater_pass.invalidate();
        self.clear_gizmo();
    }

    /// Mutable access to a terrain (sculpting, painting, raycasts).
    pub fn terrain_mut(&mut self, terrain_id: u32) -> Option<&mut crate::terrain::TerrainData> {
        self.terrains.get_mut(terrain_id as usize)
    }

    /// Shared access to a terrain.
    pub fn terrain(&self, terrain_id: u32) -> Option<&crate::terrain::TerrainData> {
        self.terrains.get(terrain_id as usize)
    }

    /// Execute the rendering pipeline for the current frame.
    /// Poll for edited shader files and swap in what compiles (MORROWIND-C).
    ///
    /// Debug builds only — `Shaders::poll_reload` is a no-op in release, so the
    /// call costs one branch in a shipped build.
    ///
    /// Returns a message for the toast when something happened, and `None` when
    /// nothing did. Three rules, in order of how badly each is usually got
    /// wrong:
    ///
    /// 1. **A broken edit shows naga's diagnostic.** Not "shader compilation
    ///    failed" — the location and the reason, which is the only useful thing
    ///    in a shader failure and the difference between a two-minute fix and
    ///    an afternoon.
    /// 2. **A broken edit changes nothing.** The old source stays registered,
    ///    the old pipelines stay bound, and the next frame draws exactly what
    ///    the last one did. Never a black screen, never a silent revert.
    /// 3. **A good edit swaps atomically.** The new module and pipeline are
    ///    built before either replaces its predecessor.
    ///
    /// Say where a shader error is, in the file somebody has open.
    ///
    /// DREAMS-A. naga parses the *composed* text, so it reports a line in a
    /// string built from up to eight files. Before this the message was
    /// prefixed with the **root** module's name, which for an error inside
    /// `brdf.wgsl` names a file the error is not in: measured, the diagnostic
    /// read `wgsl:195` for a mistake on line 48 of a 120-line module.
    ///
    /// The composed snippet is kept. It is what naga drew the caret under, and
    /// dropping it would trade one confusing message for a shorter one.
    fn shader_diagnostic(
        root: &str,
        source: &str,
        map: &somnium_shader::SourceMap,
        error: &naga::front::wgsl::ParseError,
    ) -> String {
        let body = error.emit_to_string(source);
        let Some(location) = error.location(source) else {
            // No span to translate. The root's name is still the best label
            // available, and saying so is better than dropping the label.
            return format!("{root}: {body}");
        };
        let line = location.line_number as usize;
        match map.locate(line) {
            Some(origin) => format!(
                "{}:{}:{} (composed {root} line {line})\n{body}",
                origin.module, origin.line, location.line_position,
            ),
            // The hoisted `enable` header, which was lifted out of several
            // modules and belongs to none of them.
            None => format!("{root}: hoisted header line {line}\n{body}"),
        }
    }

    /// Coverage is honest and partial: the shading pass rebuilds, because it is
    /// the acceptance case and `brdf.wgsl` composes into it. Every other pass
    /// reports its reload and keeps its existing pipeline until it grows a
    /// `reload` of its own — `ShadingPass::reload` is the pattern, and the
    /// message says which passes are waiting so the gap is visible rather than
    /// mistaken for a shader that did not take.
    pub fn reload_shaders(&mut self, ctx: &RenderContext) -> Option<String> {
        let outcome = self.shaders.poll_reload(|module, source, map| {
            // Parse only. Full validation needs capability flags that mirror the
            // device's, and a parse failure is the overwhelming majority of what
            // a mid-edit save produces; a variant that parses and fails
            // validation is caught at `create_render_pipeline` below and leaves
            // the old pipeline in place either way.
            naga::front::wgsl::parse_str(source)
                .map(|_| ())
                .map_err(|error| Self::shader_diagnostic(module, source, map, &error))
        });

        if outcome.is_empty() {
            return None;
        }
        if !outcome.failures.is_empty() {
            for failure in &outcome.failures {
                tracing::warn!("shader reload rejected: {failure}");
            }
            return Some(outcome.summary());
        }

        let mut rebuilt = 0usize;
        let shading_dirty = outcome
            .invalidated
            .iter()
            .any(|key| key.module == self.shaders.id("shading.wgsl"));
        if shading_dirty {
            match self.shading_pass.reload(&ctx.device, &self.shaders) {
                Ok(()) => rebuilt += 1,
                Err(error) => {
                    tracing::warn!("shading pipeline rebuild failed: {error}");
                    return Some(format!("Shader reload failed - {error}"));
                }
            }
        }

        let pending = outcome.invalidated.len() - rebuilt;
        let mut message = format!(
            "Reloaded {} shader module(s), {rebuilt} pipeline(s) rebuilt",
            outcome.reloaded.len()
        );
        if pending > 0 {
            message.push_str(&format!(
                " ({pending} variant(s) awaiting a pass-side reload)"
            ));
        }
        tracing::info!("{message}");
        Some(message)
    }

    pub fn render(&mut self, ctx: &RenderContext, ui: &mut UiManager, window: &Window) {
        self.render_with_game_ui(ctx, ui, window, None, None);
    }

    /// The frame, with a game's UI in it.
    ///
    /// MORROWIND-E2. `render` is this with `None`, kept so a caller that has no
    /// game UI — a test, a capture harness — does not have to say so.
    ///
    /// The callback runs at pass 9, **before** the editor shell: a HUD belongs
    /// under the editor's panels, and in a shipped build there is no editor
    /// shell for it to be under. In immersive mode the editor's `end_frame` is
    /// skipped entirely and the game's UI is the only UI, which is the same
    /// code path rather than a second one.
    pub fn render_with_game_ui(
        &mut self,
        ctx: &RenderContext,
        ui: &mut UiManager,
        window: &Window,
        game_ui: Option<&mut dyn somnium_ui::GameUi>,
        scene_target: Option<SceneTarget<'_>>,
    ) {
        // Phase 29: collects whatever timings have landed and picks this
        // frame's query slot. Before any recording, and before the counters
        // below start accumulating.
        self.profiler.begin_frame();
        self.grain_masks.advance_packed(&ctx.queue);

        // ── Phase DOOM-F: dynamic resolution ─────────────────────────────────
        //
        // Here, at the top of the frame, because a resize reallocates every
        // scene-sized target and nothing has been encoded yet. Doing it later
        // would invalidate views a half-recorded encoder is already holding.
        //
        // The signal is the profiler's smoothed GPU `Frame` scope, not the CPU
        // frame delta: `TimeState`'s hybrid limiter and vsync both pin the CPU
        // delta near the budget whatever the GPU is doing, so a controller
        // reading it would conclude the frame is always exactly on target. That
        // does mean dynamic resolution needs the profiler, which is a real
        // dependency and is stated in the Help page rather than hidden.
        if self.dynamic_resolution.enabled {
            if self.profiler.enabled() {
                // A few frames after a change, throw away the rolling average.
                // Otherwise the next decision is taken on a window that still
                // holds the previous resolution and the resize transient — the
                // controller then reads a frame time no resolution ever
                // produced and undoes its own last correct move.
                if self.dynamic_resolution.take_settle_due() {
                    self.profiler.reset_smoothing();
                }
                let frame_ms = self.profiler.total_ms();
                if let Some(scale) = self.dynamic_resolution.tick(frame_ms) {
                    let (w, h) = self
                        .dynamic_resolution
                        .apply(self.base_extent.0, self.base_extent.1);
                    tracing::debug!(
                        scale,
                        frame_ms,
                        target = self.dynamic_resolution.target_ms,
                        "dynamic resolution → {w}×{h}"
                    );
                    self.resize_targets(ctx, w, h);
                }
            } else {
                // Detect, do not demand — but say so once rather than sitting
                // silently at native while the user believes it is working.
                self.dynamic_resolution.enabled = false;
                self.dynamic_resolution.reset();
                tracing::warn!(
                    "dynamic resolution needs the profiler for a GPU frame time; switching it off"
                );
            }
        }

        // Whichever surface the scene is going to, not necessarily this
        // window's: FSR upscales to the size of the thing it lands on.
        let present_size = scene_target.map_or((ctx.config.width, ctx.config.height), |t| t.size);
        let (ldr_w, ldr_h) = if self.fsr_pass.enabled {
            present_size
        } else {
            (self.render_width, self.render_height)
        };
        self.ensure_ldr_size(ctx, ldr_w, ldr_h);

        // ── 4. Acquire swapchain texture ─────────────────────────────────────
        //
        // PORTAL-0-B times this rather than wrapping it in a CPU scope: under
        // `PresentMode::AutoVsync` the presentation block lands here, and a
        // scope would have to close before `end_frame` to be reported in the
        // same frame, which this one does — but the row belongs beside
        // `Frame wall` and `Frame CPU` rather than among the engine's own
        // zones, because it is a wait and not work.
        let acquire_started = std::time::Instant::now();
        let output = match ctx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(tex) => tex,
            wgpu::CurrentSurfaceTexture::Suboptimal(tex) => tex,
            _ => {
                tracing::warn!("Failed to acquire surface texture");
                // The per-frame queues still have to be emptied. Returning
                // without clearing leaves this frame's submissions in place and
                // the next frame appends to them, so everything is drawn twice
                // — invisible for opaque geometry, but it double-blends the
                // transparent pass and wastes a whole frame of work.
                self.clear_frame_queues();
                return;
            }
        };
        self.profiler.surface_acquire_ms = acquire_started.elapsed().as_secs_f32() * 1000.0;
        let surface_view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // The picture and the chrome can be in different windows. Everything
        // below writes to one of these two, and which one is not a detail: the
        // scene, the gizmos, the outline and the particles belong to the
        // viewport, and the menus, panels and status bar belong to the editor.
        let scene_view = scene_target.map_or(&surface_view, |target| target.view);

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Main Render Encoder"),
            });

        // Nothing is going to draw the scene into this window, and the UI pass
        // loads rather than clears. Without this the editor's own swapchain
        // keeps whatever was in that buffer two frames ago, which with a
        // triple-buffered surface reads as a flicker behind the panels.
        if scene_target.is_some() {
            encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Clear Editor Swapchain"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &surface_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
        }

        // Phase 29: the profiler brackets each pass from outside, so no pass
        // has to know it is being timed. Scopes nest — `end` closes the
        // innermost — and the frame scope is what everything else indents under.
        self.profiler.begin(&mut encoder, "Frame");

        // Phase 29 counters, from Flax's `RenderStatsData`: a pass time says
        // how long something took and never why, and "why" is nearly always one
        // of these. Counted off the draw queue rather than the indirect buffer
        // because the indirect counts are only known on the GPU — this is what
        // was *submitted*, which is the number a regression shows up in first.
        {
            let c = &mut self.profiler.counters;
            c.draw_calls = u32::try_from(self.draw_queue.len()).unwrap_or(u32::MAX);
            c.instances = c.draw_calls;
            c.triangles = self
                .draw_queue
                .iter()
                .map(|d| d.index_count / 3)
                .fold(0u32, u32::saturating_add);
            c.terrain_chunks = u32::try_from(
                self.draw_queue
                    .iter()
                    .filter(|d| self.terrain_material_ids.contains(&d.material_id))
                    .count(),
            )
            .unwrap_or(u32::MAX);
            c.tlas_instances = self.raytrace_pass.instance_count();
            c.dreams_grain_consumers = if self.grain_masks.shared_enabled() {
                u32::from(self.gtao_pass.enabled)
                    + u32::from(self.volumetric_pass.enabled)
                    + u32::from(self.taa_pass.enabled())
                    + u32::from(self.restir_pass.active())
                    + u32::from(self.restir_gi_pass.active())
            } else {
                0
            } + u32::from(self.grain_masks.stf_enabled());
        }

        // ── MORROWIND-J step 3: one pass per view ────────────────────────────
        //
        // The scene above is recorded once *per view*, each into its own
        // rectangle of the swapchain. `views()` is a single view in the shape
        // the editor has always had, so a one-viewport frame records exactly
        // what it recorded before this loop existed.
        self.capture_frame_view_state();
        let views = self.scene_views();
        let mut capture_now = false;
        for (index, view) in views.iter().enumerate() {
            // Opt-in, like `SOMNIUM_SOMUI_DEBUG`. Four tiles that all show the
            // same picture is the failure mode of this loop, and it is
            // indistinguishable from four tiles that all *should* — so the one
            // thing worth printing is what each view was actually pointed at.
            if std::env::var("SOMNIUM_VIEW_DEBUG").as_deref() == Ok("1") {
                tracing::info!(
                    "VIEWDBG {index} rect={:?} eye={:?} ortho={}",
                    view.rect,
                    view.camera_pos,
                    view.proj.w_axis.w == 1.0
                );
            }
            capture_now |= self.record_scene_view(
                ctx,
                &mut encoder,
                scene_view,
                view,
                index == 0,
                index as u64,
            );
        }
        self.restore_primary_view(&views);

        // Display-only evidence (tone map, bloom, FXAA, CAS). Capture before
        // gizmos/UI so an A/B measures the scene rather than editor chrome.
        if capture_now && self.capture.wants_display() {
            if scene_target.is_some() {
                // The capture copies out of *this* window's texture, and the
                // scene is not in it. Saying so beats writing a PNG of the
                // editor's panels under the name of a scene A/B.
                tracing::warn!("display capture skipped: the viewport is in its own window");
            } else if ctx.config.usage.contains(wgpu::TextureUsages::COPY_SRC) {
                self.capture.record_display(
                    &ctx.device,
                    &mut encoder,
                    &output.texture,
                    ctx.config.width,
                    ctx.config.height,
                    ctx.config.format,
                );
            } else {
                tracing::warn!("display capture skipped: surface lacks COPY_SRC usage");
            }
        }

        // Overlays draw onto the reconstructed image. The scene used a jittered
        // view_proj; FSR/TAA already undid that. Feeding the jittered matrix
        // here makes gizmos and outlines swim by a pixel every frame.
        // The same rule as the scene's upload above: one view keeps the plain
        // write it has always used.
        if self.views.len() > 1 {
            self.stage_view_buffer(
                &ctx.queue,
                &mut encoder,
                OVERLAY_VIEW_SLOT,
                self.view_proj_unjittered,
            );
        } else {
            self.write_view_buffer(&ctx.queue, self.view_proj_unjittered);
        }

        self.profiler.begin(&mut encoder, "Editor overlays");

        // ── 8.5 Gizmo Pass → swapchain (after tone-mapping, before UI) ───────
        if self.editor_overlays_enabled
            && let Some(gizmo_pos) = self.gizmo_world_pos
        {
            let dist = (self.camera_pos - gizmo_pos).length().max(0.5);
            let scale = dist * 0.15;
            let model = glam::Mat4::from_translation(gizmo_pos)
                * glam::Mat4::from_quat(self.gizmo_world_rotation)
                * glam::Mat4::from_scale(glam::Vec3::splat(scale));
            self.gizmo_pass.update_transform(&ctx.queue, model);
            self.gizmo_pass
                .record(&mut encoder, scene_view, self.gizmo_mode);
        }

        // ── 8.7 Selection outline → swapchain (Phase 11.5I) ─────────────────
        if self.editor_overlays_enabled
            && let Some((v_off, i_off, i_cnt, model)) = self.outline_entity
        {
            self.outline_pass.record(
                &ctx.queue,
                &mut encoder,
                scene_view,
                self.view_proj_unjittered,
                model,
                v_off,
                i_off,
                i_cnt,
                [0.98, 0.58, 0.07, 1.0], // orange highlight (#FA9412)
                0.007,                   // ~2-3 px at typical camera distance
            );
        }

        // ── 8.75 Light gizmos → swapchain (Phase 13E) ────────────────────────
        if self.editor_overlays_enabled
            && self.light_gizmos_enabled
            && !(self.light_gizmo_queue.is_empty() && self.line_gizmo_queue.is_empty())
        {
            let mut lines =
                crate::pass::light_gizmo::build_light_gizmo_lines(&self.light_gizmo_queue);
            lines.extend_from_slice(&self.line_gizmo_queue);
            self.light_gizmo_pass
                .record(&ctx.device, &ctx.queue, &mut encoder, scene_view, &lines);
        }

        // ── 8.8 Particle Pass → swapchain (Phase 11.5J) ──────────────────────
        if !self.pending_particles.is_empty() {
            self.particle_pass.record(
                &ctx.queue,
                &mut encoder,
                scene_view,
                self.view_proj_unjittered,
                self.view_matrix,
                &self.pending_particles,
            );
        }

        self.profiler.end(&mut encoder); // Editor overlays

        // ── 9. UI Overlay ────────────────────────────────────────────────────
        self.profiler.begin(&mut encoder, "UI");
        // MORROWIND-E2. The game first, the editor over it. A separate profiler
        // zone on purpose: a HUD that costs two milliseconds should be visible
        // as a HUD that costs two milliseconds, not as an editor that got
        // slower.
        if let Some(game_ui) = game_ui {
            self.profiler.begin(&mut encoder, "Game UI");
            let mut frame = somnium_ui::GameUiFrame::new(
                window,
                &ctx.device,
                &ctx.queue,
                &mut encoder,
                &surface_view,
                ctx.config.format,
            );
            game_ui.draw_ui(&mut frame);
            let drawn = frame.drawn();
            self.profiler.end(&mut encoder); // Game UI
            if drawn == 0 && !self.game_ui_empty_warned {
                self.game_ui_empty_warned = true;
                tracing::warn!(
                    "on_render_ui drew no canvas; a game UI hook that draws nothing is the                      bug MORROWIND-E2 exists to fix (plan A.7, track 1)"
                );
            }
        }
        if !ui.is_immersive() {
            ui.end_frame(window, &ctx.device, &ctx.queue, &mut encoder, &surface_view);
        }
        self.profiler.end(&mut encoder); // UI

        // Editor evidence (Phase 26-Zeta). Unlike the display capture above,
        // this runs *after* the UI pass, so it is the only capture that can
        // show chrome. Phase 26-Zeta §10 asks for visual evidence that is not
        // a fabricated screenshot; this is where it comes from.
        if capture_now && self.capture.wants_ui() {
            if ctx.config.usage.contains(wgpu::TextureUsages::COPY_SRC) {
                self.capture.record_ui(
                    &ctx.device,
                    &mut encoder,
                    &output.texture,
                    ctx.config.width,
                    ctx.config.height,
                    ctx.config.format,
                );
            } else {
                tracing::warn!("ui capture skipped: surface lacks COPY_SRC usage");
            }
        }

        let stats_draws = if self.cull_stats {
            self.indirect.len()
        } else {
            0
        };
        self.profiler.end(&mut encoder); // Frame
        self.profiler.end_frame(&mut encoder);
        ctx.queue.submit(std::iter::once(encoder.finish()));
        // Must follow the submit: the map would otherwise race the copy that
        // fills the buffer it is reading.
        self.profiler.after_submit(&ctx.device);
        // Phase DOOM-B. `after_submit` above already polled the device, so a
        // census readback from an earlier frame may have landed; collect first
        // so the newest available count is what a timing run records.
        self.census_pass.collect();
        self.census_pass.after_submit();

        // Phase DOOM-A. After `after_submit`, because that is what polls the
        // device and lets an earlier frame's readback callback fire — sampling
        // before it would read the same stale frame every time and report a
        // standard deviation of zero.
        if let Some(run) = &mut self.timing
            && run.active()
        {
            run.tick(
                &self.profiler,
                self.census_pass.result,
                &ctx.adapter,
                &ctx.device,
                (self.render_width, self.render_height),
            );
        }
        if stats_draws > 0 {
            self.report_cull_stats(ctx, stats_draws);
        }
        if capture_now {
            // After submit, so the copies recorded above have actually run.
            // Resolved before the draw queue is cleared, because labelling a
            // pixel as terrain means looking its instance back up in it.
            let terrain_ids = &self.terrain_material_ids;
            let draws = &self.draw_queue;
            self.capture.resolve(&ctx.device, |instance| {
                draws
                    .get(instance as usize)
                    .is_some_and(|d| terrain_ids.contains(&d.material_id))
            });
            let profile_report = self.profiler.report();
            if self.profiler.enabled() {
                for line in &profile_report {
                    tracing::info!("XV-J-PROFILE {line}");
                }
            }
            tracing::info!(
                scene = ?(self.render_width, self.render_height),
                swapchain = ?(ctx.config.width, ctx.config.height),
                fsr = self.fsr_pass.enabled,
                clipmap = self.clipmaps.first().map(|c| c.enabled).unwrap_or(false),
                "DF-A-VIEWPORT"
            );
            if let Some(t) = self.terrains.first() {
                tracing::info!(
                    "XV-J-RESIDENCY compressed={} from_assets={} hero={} extra={} wetness={:.3} hex={} aerial_lod_m=80",
                    t.layer_textures.compressed,
                    t.layer_textures.from_assets,
                    t.layer_textures.resolution,
                    t.layer_textures.extra_resolution,
                    t.wetness,
                    t.hex_tiling,
                );
            }
            // The Windows GUI executable has no console attached in release
            // builds, so redirected stdout is legitimately empty. This
            // explicit audit sink keeps the capture matrix reproducible and
            // records effective pass state alongside timings/counters.
            if let Ok(path) = std::env::var("SOMNIUM_AUDIT_LOG") {
                let mut lines = vec![
                    format!("scene={}x{}", self.render_width, self.render_height),
                    format!("swapchain={}x{}", ctx.config.width, ctx.config.height),
                    format!("surface_format={:?}", ctx.config.format),
                    format!("device_features={:?}", ctx.features),
                    format!(
                        "effective fsr={} taa={} cas={} bloom={} gtao={} volumetrics={} shafts={} water_rt={} water_refract={} restir_di={} restir_gi={} motion_blur={} dof={} lighting_extra_flags=0x{:x}",
                        self.fsr_pass.enabled,
                        self.taa_pass.enabled(),
                        self.cas_pass.enabled,
                        self.bloom_pass.enabled,
                        self.gtao_pass.enabled,
                        self.volumetric_pass.enabled,
                        self.volumetric_pass.enabled && self.volumetric_pass.fog.shafts,
                        self.water_reflection_pass.enabled,
                        self.water_reflection_pass.refract_enabled,
                        self.restir_pass.enabled,
                        self.restir_gi_pass.enabled,
                        self.motion_blur_pass.enabled,
                        self.dof_pass.enabled,
                        self.lighting_extra_pass.flags_bits(),
                    ),
                    format!(
                        "lighting_extra_accumulated_frames={}",
                        self.lighting_extra_pass.accumulated_frames()
                    ),
                    format!("sun_direction_y={:.6}", self.light_direction.y),
                ];
                if let Some(t) = self.terrains.first() {
                    lines.push(format!(
                        "terrain compressed={} from_assets={} hero={} extra={} wetness={:.3} hex={} parallax={:.4}",
                        t.layer_textures.compressed,
                        t.layer_textures.from_assets,
                        t.layer_textures.resolution,
                        t.layer_textures.extra_resolution,
                        t.wetness,
                        t.hex_tiling,
                        t.parallax_scale,
                    ));
                }
                lines.extend(profile_report);
                if let Err(error) = std::fs::write(&path, lines.join("\n")) {
                    tracing::error!("audit log write to {path} failed: {error}");
                }
            }
        }
        // wgpu 30 moved presentation from the surface texture to the queue,
        // so the present is ordered against submitted work explicitly rather
        // than implicitly by the texture's lifetime.
        ctx.queue.present(output);

        self.clear_frame_queues();
    }

    // ── MORROWIND-J step 3: several views in one frame ──────────────────────

    /// Ask for a specific set of views next frame.
    ///
    /// Passing an empty slice restores the single full-window view, which is
    /// the path the renderer has always taken and not a one-element special
    /// case of the new one.
    pub fn set_scene_views(&mut self, views: &[crate::view::SceneView]) {
        self.views.clear();
        self.views.extend_from_slice(views);
    }

    /// How many views the next frame will record.
    #[must_use]
    pub fn scene_view_count(&self) -> usize {
        self.views.len().max(1)
    }

    /// Remember what the per-view overrides are overriding.
    ///
    /// Taken once, at the top of the frame, rather than saved and restored
    /// around each view: the second view would otherwise save the *first
    /// view's* overrides as the thing to go back to, and the settings would
    /// walk one view further from the truth every frame.
    fn capture_frame_view_state(&mut self) {
        self.frame_view_state = FrameViewState {
            taa: self.taa_pass.enabled(),
            fsr: self.fsr_pass.enabled,
            shading_debug: self.shading_debug,
            overlays: self.editor_overlays_enabled,
        };
    }

    /// The camera the last `set_view` established, as a view.
    ///
    /// The unjittered projection, because a caller building tiles out of this
    /// is going to hand it back and `set_view` re-applies the jitter — applying
    /// it twice is a permanent half-pixel offset that reads as a soft image
    /// nobody can find the cause of.
    #[must_use]
    pub fn primary_scene_view(&self) -> crate::view::SceneView {
        crate::view::SceneView::full(self.view_matrix, self.proj_matrix, self.camera_pos)
    }

    /// This frame's views. Never empty.
    fn scene_views(&self) -> Vec<crate::view::SceneView> {
        if self.views.is_empty() {
            vec![crate::view::SceneView::full(
                self.view_matrix,
                self.proj_matrix,
                self.camera_pos,
            )]
        } else {
            self.views.clone()
        }
    }

    /// Point the renderer at one view before recording it.
    ///
    /// Temporal passes are the reason `primary` exists. TAA, FSR and ReSTIR all
    /// carry a **history keyed to one camera**; a second view reusing it does
    /// not merely look wrong, it reprojects last frame's other viewport into
    /// this one and smears. So the secondary views run history-free, which is
    /// also what makes a four-up frame affordable — and it is a statement about
    /// what those buffers are, not a shortcut: giving every view its own
    /// history is four times the memory for three views nobody is looking at
    /// closely.
    fn apply_scene_view(
        &mut self,
        _ctx: &RenderContext,
        view: &crate::view::SceneView,
        primary: bool,
        slot: u64,
    ) {
        self.view_slot = slot;
        self.set_view(view.view, view.proj, view.camera_pos);
        if !primary {
            self.taa_pass.set_enabled(false);
            self.fsr_pass.enabled = false;
        }
        if let Some(debug_view) = view.debug_view {
            self.shading_debug = debug_view as f32;
        }
        self.editor_overlays_enabled = self.frame_view_state.overlays && view.overlays;
    }

    /// Put back everything `apply_scene_view` borrowed for a secondary view.
    ///
    /// Without this a frame that drew a second viewport would leave TAA off for
    /// every frame after it — the classic shape of a bug that only appears once
    /// you have used a feature and then stopped.
    fn restore_primary_view(&mut self, views: &[crate::view::SceneView]) {
        if views.len() > 1 {
            self.taa_pass.set_enabled(self.frame_view_state.taa);
            self.fsr_pass.enabled = self.frame_view_state.fsr;
        }
        self.shading_debug = self.frame_view_state.shading_debug;
        self.editor_overlays_enabled = self.frame_view_state.overlays;
        // Overlays and picking below the loop read the primary camera, and the
        // last view recorded was not it.
        if let Some(primary) = views.first() {
            self.set_view(primary.view, primary.proj, primary.camera_pos);
        }
    }

    /// Record one view of the scene: everything from clustered lighting to the
    /// tone-mapped image landing in this view's rectangle of the swapchain.
    ///
    /// MORROWIND-J step 3. This was the middle two thirds of
    /// [`Self::render_with_game_ui`] and is unchanged in substance — it is a
    /// method rather than a loop body in place so that the diff is a *move*
    /// rather than a reindent of eighteen hundred lines, and so that the
    /// per-view state it needs arrives as an argument instead of being read off
    /// `self` by whatever set it last.
    ///
    /// Returns whether a frame capture was armed for this view, which is the
    /// one fact the chrome below the loop still needs.
    #[allow(clippy::too_many_lines)]
    fn record_scene_view(
        &mut self,
        ctx: &RenderContext,
        encoder: &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
        view: &crate::view::SceneView,
        primary: bool,
        slot: u64,
    ) -> bool {
        // Rebound by value so the body below reads exactly as it did when it
        // was inline — `&mut encoder` throughout, rather than a thousand lines
        // of `&mut *encoder`.
        let mut encoder = encoder;
        self.apply_scene_view(ctx, view, primary, slot);
        // ── Phase 13C: Clustered lighting assignment ───────────────────────
        // Bit 4 must be uniform for the whole draw: a per-pixel `traced.a`
        // skip of PCSS is varying, DXC flattens it, and every terrain pixel
        // still pays the 16+24 filter (and contact march) even when ReSTIR
        // already wrote the sun. `active()` is the CPU's "this frame's vis
        // target is live" flag.
        let mut shading_mode = self.shading_mode;
        if self.restir_pass.active() && self.raytrace_pass.tlas().is_some() {
            shading_mode |= 16;
        }
        if self.grain_masks.stf_enabled() {
            shading_mode |= 32;
        }
        // Phase TSUSHIMA-F: the energy terms — multiple-scattering
        // compensation on both the direct lobe and the IBL, Hammon's rough
        // diffuse, and micro-shadowing. On by default;
        // `SOMNIUM_TERRAIN_BRDF=0` is the A/B rail.
        //
        // A `shading_mode` bit rather than a pipeline override because these
        // are a handful of ALU on values already in registers, not a march.
        // The occupancy argument that put hex and POM behind `override`s does
        // not apply, and this avoids a tenth entry in `ShadingSpec::constants`
        // and a new variant in a budget `context.md` tracks.
        // Three bits, not one: the terms pull in different directions and a
        // single switch cannot attribute a change to any of them.
        // `SOMNIUM_TERRAIN_BRDF=0` still turns all three off in one go.
        if self.brdf_multiscatter {
            shading_mode |= 64;
        }
        if self.brdf_rough_diffuse {
            shading_mode |= 128;
        }
        if self.brdf_micro_shadow {
            shading_mode |= 256;
        }
        self.global_pool.cluster_grid.assign_and_upload(
            &ctx.queue,
            &self.local_lights,
            self.view_matrix,
            self.proj_matrix,
            self.render_width,
            self.render_height,
            0.1,    // near
            1000.0, // far
            shading_mode,
        );
        self.local_lights.clear();

        // Phase CONTROL-O: the same grid geometry, the same matrices, the same
        // near and far. Binned here rather than where the ECS is walked so a
        // decal and a light in the same froxel genuinely are in the same
        // froxel — a submission-time binning would be one frame stale, which
        // shows as decals popping at tile boundaries under a fast camera.
        let decals = std::mem::take(&mut self.decals);
        self.decal_grid.assign_and_upload(
            &ctx.queue,
            &decals,
            self.view_matrix,
            self.proj_matrix,
            self.render_width,
            self.render_height,
            0.1,
            1000.0,
        );
        self.decals = decals;
        self.decals.clear();
        // ── 0. Upload view buffer ────────────────────────────────────────────
        //
        // Through the encoder, not `write_buffer`: see `stage_view_buffer`.
        //
        // **Unjittered, and deliberately.** `self.view_proj` carries the TAA /
        // FSR sub-pixel offset, and before the staging fix below it never
        // reached a shader: the editor overlays upload the unjittered matrix
        // after the scene, and a `write_buffer` staged later in the frame wins
        // for the whole frame. So the scene has always drawn unjittered, and
        // ordering the uploads correctly turned that on for the first time —
        // which shows up as the whole viewport shaking, hardest from a high
        // camera where a pixel of terrain covers metres of ground.
        //
        // Turning jitter on is a rendering change with its own A/B to run and
        // its own record to write. MORROWIND-J step 3 is about drawing several
        // views, and it promised a one-viewport frame identical to the one
        // before it. This keeps that promise; see MORROWIND-J for the finding.
        // **One view takes the path it always took.**
        //
        // MORROWIND-J step 3 promised that a one-viewport frame is the frame
        // that came before it, and the staged-copy mechanism below is only
        // needed when several views share a command buffer. Using it
        // unconditionally is how this change reached the editor as a viewport
        // that shook — the copy lands *inside* the encoder where the plain
        // write landed at the top of the submit, and every pass that reads the
        // view buffer sees a different matrix as a result.
        //
        // So the default path is byte-for-byte the old one, and the new
        // mechanism is confined to the case that cannot work without it.
        //
        // The multi-view branch uploads the **unjittered** matrix, because that
        // is what a one-view frame effectively renders with: its jittered write
        // is overwritten by the overlays' unjittered one before any pass runs.
        // Matching it is what keeps a tile the same picture as the whole
        // viewport, rather than the same picture plus a shake.
        if self.views.len() > 1 {
            self.stage_view_buffer(
                &ctx.queue,
                encoder,
                self.view_slot,
                self.view_proj_unjittered,
            );
        } else {
            self.write_view_buffer(&ctx.queue, self.view_proj);
        }

        // ── 0.5 Phase 19: refresh the environment cubemap ────────────────────
        // No-ops unless the sun actually moved, so this is free in the common
        // case. The sky is captured from the same procedural function the
        // background uses, keeping reflections consistent with what is drawn.
        // Phase 24C: the scattering LUTs the sky march reads. Built once —
        // they depend on the atmosphere's composition, not on sun or camera.
        self.atmosphere_pass.ensure_built(&ctx.device, &ctx.queue);

        self.ibl_pass.generate_if_needed(
            &ctx.device,
            &ctx.queue,
            self.light_direction,
            self.light_color,
        );

        // ── 1. Compute cascades and upload light buffer ───────────────────────
        // Fitted from the UNJITTERED inverse.
        //
        // `inv_view_proj` above is the jittered one, which is right for
        // reconstructing world position from a jittered depth buffer but wrong
        // here: it makes the cascade frusta shift by the sub-pixel jitter every
        // frame, so every shadow-map texel lands somewhere slightly different in
        // world space and every shadow edge crawls. TAA cannot average that
        // away, because it is a real change in the scene rather than a sampling
        // difference — which is why the shimmer vanished when TAA was switched
        // off: `jitter_ndc` returns zero when TAA is disabled, so the cascades
        // stopped moving.
        let cascade_candidates =
            compute_cascades(self.light_direction, self.view_proj_unjittered.inverse());
        let shadow_cache_enabled = std::env::var("SOMNIUM_SHADOW_CACHE").as_deref() != Ok("0");
        let cascade_cache_frame = self.cascade_shadow_cache.begin_frame(
            cascade_candidates,
            self.light_direction,
            shadow_cache_enabled,
        );
        let cascades = cascade_cache_frame.cascades;
        self.cascade_view_projs = std::array::from_fn(|i| cascades[i].view_proj);

        let shadow_debug = if self.shading_debug != 0.0 {
            self.shading_debug
        } else {
            std::env::var("SOMNIUM_SHADOW_DEBUG")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0)
        };
        let gpu_light = GpuDirectionalLight {
            direction: self.light_direction.to_array(),
            _pad0: 0.0,
            color: self.light_color.to_array(),
            _pad1: 0.0,
            view_proj: std::array::from_fn(|i| cascades[i].view_proj.to_cols_array_2d()),
            cascade_splits: [
                cascades[0].split_depth,
                cascades[1].split_depth,
                cascades[2].split_depth,
                cascades[3].split_depth,
            ],
            shadow_map_size: ATLAS_SIZE as f32,
            ibl_intensity: self.ibl_intensity,
            sun_angular_radius: self.sun_angular_radius,
            // TEMP: shadow_factor debug visualisation.
            _pad2: shadow_debug,
            // Phase 25M-2: physical moon direction from simplified orbital model.
            moon_direction: crate::shadow::moon_direction(
                self.light_direction,
                (self.time as f64) / 86400.0,
            )
            .to_array(),
            moon_intensity: self.moon_intensity,
        };
        ctx.queue.write_buffer(
            &self.global_pool.light_buffer,
            0,
            bytemuck::bytes_of(&gpu_light),
        );

        // ── 1.5 Terrain becomes ordinary draws (Phase 25A-2) ─────────────────
        //
        // Before the sort, because from here on terrain is indistinguishable
        // from any other geometry: it goes into the same instance buffer, the
        // same indirect arguments, the same GPU frustum and Hi-Z culling, the
        // same shadow pass — which is also the first time terrain casts a
        // shadow — and the same visibility buffer, where `shading.wgsl` picks
        // it up. There is no terrain pass left to run at the end of the frame.
        //
        // Phase CR-B: camera-frustum failures skip `draw_queue` (and therefore
        // vis / GPU 15B). They still occupy `shadow_only_queue` when they hit a
        // cascade, so off-screen ground can still shadow into view (15B's
        // contract). CR-E cascade-culls that list; never the camera frustum.
        self.profiler.cpu_begin("Terrain");
        self.shadow_only_queue.clear();
        self.terrain_lod_by_vertex.clear();
        let cam_planes = crate::culling::frustum_planes(self.view_proj_unjittered);
        let cascade_planes: [_; crate::shadow::NUM_CASCADES] =
            self.cascade_view_projs.map(crate::culling::frustum_planes);
        let cpu_frustum = self.cpu_frustum_active();
        let cascade_cull = self.cascade_caster_cull && !cascade_cull_env_off();
        let mut cpu_culled = 0u32;
        let mut shadow_geometry_changed = false;
        for &(id, model) in &self.terrain_queue {
            let shoreline_regions = self.water_bodies.shoreline_lod_regions(id);
            let terrain = &mut self.terrains[id as usize];
            terrain.model = model;
            let local_cam = model.inverse().transform_point3(self.camera_pos);
            terrain.select_lods(local_cam, &shoreline_regions);
            self.rebuilt_chunks.clear();
            terrain.rebuild_dirty_chunks(&ctx.queue, &mut self.geometry, &mut self.rebuilt_chunks);
            shadow_geometry_changed |= !self.rebuilt_chunks.is_empty();
            terrain.ensure_index_blocks(&ctx.queue, &mut self.geometry);
            terrain.splatmap.upload_dirty(&ctx.queue);

            // Phase 25B: terrain enters the ray-traced scene. A chunk's BLAS is
            // registered the first time its heights are written and rebuilt
            // whenever they change, which is exactly the set the rebuild above
            // just reported — so a sculpt stroke moves the traced surface with
            // the drawn one instead of leaving a shadow of the old ground.
            //
            // Always the full-detail, unstitched geometry, never the frame's
            // LOD: a BLAS is sized once, and a traced shadow that changed shape
            // as chunks swapped LOD would be worse than one slightly finer than
            // what is drawn.
            // `SOMNIUM_RT_TERRAIN=0` keeps terrain out of the acceleration
            // structures while changing nothing else, which is the A/B that
            // isolates what 25B added: with it, terrain both casts and receives
            // traced shadows; without it, the TLAS holds the same scene 24J saw.
            if self.raytrace_pass.supported()
                && !self.rebuilt_chunks.is_empty()
                && self.debug_toggles.is_on("rt_terrain")
            {
                if let Some((rt_index_offset, rt_index_count)) = terrain.rt_index_block() {
                    let vertex_count = terrain.chunk_vertex_capacity();
                    for &vertex_offset in &self.rebuilt_chunks {
                        self.raytrace_pass.register_mesh(
                            &ctx.device,
                            vertex_offset,
                            vertex_count,
                            rt_index_offset,
                            rt_index_count,
                        );
                        self.raytrace_pass.mark_geometry_dirty(vertex_offset);
                    }
                }
            }
            self.terrain_materials.write(
                &ctx.queue,
                terrain.terrain_index,
                &terrain.gpu_material_for_camera(local_cam),
            );

            let material_id = terrain.material_id;
            for chunk in &terrain.chunks {
                if chunk.vertex_offset == crate::terrain::UNALLOCATED {
                    continue;
                }
                let Some((index_offset, index_count)) =
                    terrain.index_block(chunk.lod, chunk.edge_mask)
                else {
                    continue;
                };
                // Pushed straight onto the opaque queue rather than through
                // `submit`, which would need a second mutable borrow of self —
                // terrain's material is registered opaque, so the routing
                // `submit` does would be a no-op anyway.
                //
                // PORTAL-0-D: record the packed LOD word here, where the chunk
                // and its terrain are both in hand, rather than recovering it
                // by search once per draw command later.
                {
                    let verts = terrain.desc.chunk_cells + 1;
                    let lod_base = terrain.desc.lod_base_range.round().clamp(1.0, 255.0) as u32;
                    let start = (terrain.lod_morph_start.clamp(0.0, 1.0) * 1023.0) as u32;
                    let morph_on = u32::from(terrain.lod_morph);
                    self.terrain_lod_by_vertex.insert(
                        chunk.vertex_offset,
                        (u32::from(chunk.lod) & 15)
                            | ((verts & 511) << 4)
                            | (morph_on << 13)
                            | ((lod_base & 255) << 14)
                            | ((start & 1023) << 22),
                    );
                }
                let cmd = DrawCommand {
                    casts_shadow: true,
                    sort_key: crate::command::SortKey::new(
                        0,
                        material_id as u16,
                        chunk.vertex_offset,
                    ),
                    vertex_offset: chunk.vertex_offset,
                    index_offset,
                    index_count,
                    material_id,
                    transform: model,
                };
                let in_camera = !cpu_frustum
                    || crate::culling::chunk_in_frustum(
                        &cam_planes,
                        model,
                        chunk.aabb_min,
                        chunk.aabb_max,
                    );
                if in_camera {
                    self.draw_queue.push(cmd);
                } else {
                    cpu_culled += 1;
                    let (wmin, wmax) =
                        crate::culling::transform_aabb(model, chunk.aabb_min, chunk.aabb_max);
                    let in_cascade = !cascade_cull
                        || crate::culling::aabb_in_any_frustum(&cascade_planes, wmin, wmax);
                    if cmd.casts_shadow && in_cascade {
                        self.shadow_only_queue.push(cmd);
                    }
                }
            }
        }
        if shadow_geometry_changed {
            self.invalidate_shadow_casters();
        }
        self.profiler.counters.terrain_cpu_culled = cpu_culled;
        self.profiler.cpu_end();

        for &(id, model) in &self.terrain_queue {
            let Some(terrain) = self.terrains.get(id as usize) else {
                continue;
            };
            let local_cam = model.inverse().transform_point3(self.camera_pos);
            let mut mat = terrain.gpu_material_for_camera(local_cam);
            let terrain_index = terrain.terrain_index;
            let edit_revision = terrain.edit_revision;
            if let Some(clipmap) = self.clipmaps.get_mut(id as usize) {
                if clipmap.enabled && !crate::terrain::clipmap::TerrainClipmap::env_forced_off() {
                    let forward = self
                        .view_matrix
                        .inverse()
                        .transform_vector3(glam::Vec3::NEG_Z);
                    clipmap.update(
                        crate::terrain::clipmap::focus_xz(
                            self.camera_pos.to_array(),
                            forward.to_array(),
                        ),
                        edit_revision,
                    );
                }
                clipmap.fill_gpu(&mut mat);
            }
            self.terrain_materials
                .write(&ctx.queue, terrain_index, &mat);
        }

        // ── 2. Sort draw queue ───────────────────────────────────────────────
        // This has to happen before the instance buffer is built. Instance `i`
        // is what draw `i` pulls its model matrix and geometry offsets from, so
        // reordering the queue afterwards pairs every draw with a different
        // mesh's offsets — which renders as triangles stretched between
        // unrelated parts of the geometry pool.
        self.draw_queue.sort_by_key(|cmd| cmd.sort_key);

        // ── 3. Build and upload instance buffer ──────────────────────────────
        self.profiler.cpu_begin("Instances");
        self.instances.clear();
        for cmd in &self.draw_queue {
            self.instances.add_instance(gpu_instance_from_cmd(
                &self.terrain_lod_by_vertex,
                cmd,
                shadow_debug,
            ));
        }
        for cmd in &self.shadow_only_queue {
            self.instances.add_instance(gpu_instance_from_cmd(
                &self.terrain_lod_by_vertex,
                cmd,
                shadow_debug,
            ));
        }
        // Phase 21: blended draws share the same instance buffer, appended
        // after the opaque ones (vis + shadow-only). The visibility pass only
        // draws the opaque vis range; the transparent pass indexes into the tail.
        let frame_layout =
            frame_instance_layout(self.draw_queue.len(), self.shadow_only_queue.len());
        let transparent_base = frame_layout.transparent_base;
        let mut transparent_draws: Vec<crate::pass::transparent::TransparentDraw> =
            Vec::with_capacity(self.transparent_queue.len());
        for (i, cmd) in self.transparent_queue.iter().enumerate() {
            self.instances
                .add_instance(crate::instance::GpuInstanceData {
                    model_matrix: cmd.transform.to_cols_array_2d(),
                    material_id: cmd.material_id,
                    mesh_vertex_offset: cmd.vertex_offset,
                    mesh_index_offset: cmd.index_offset,
                    _padding: 0,
                });
            let origin = cmd.transform.w_axis.truncate();
            transparent_draws.push(crate::pass::transparent::TransparentDraw {
                instance_index: transparent_base + i as u32,
                index_count: cmd.index_count,
                depth_sq: (origin - self.camera_pos).length_squared(),
            });
        }
        crate::pass::transparent::sort_back_to_front(&mut transparent_draws);
        self.instances.upload(&ctx.queue);
        self.profiler.cpu_end();

        // ── 3.5 Phase 15A: build this frame's indirect draw arguments ────────
        // Argument `i` lines up with instance `i`, which the sort above keeps true.
        if self.gpu_driven {
            // Phase 15F: each draw expands into one argument per cluster, with
            // a parallel array of bounds for the cull shader. A mesh with no
            // clusters (voxel chunks) or with no recorded AABB falls back to a
            // single whole-mesh argument that is never culled — safer than
            // guessing at its extent.
            // Phase 17D: single-sided draws first, then double-sided, with the
            // boundary recorded. The visibility pass issues one indirect call
            // per range so each gets the right cull mode. Argument order does
            // not have to match the draw queue — `first_instance` carries the
            // instance explicitly, and the cull shader reads it from there.
            // Phase 17G: clustering pays for a big mesh drawn once — parts of
            // it are meaningfully cullable. It is backwards for a small mesh
            // drawn thousands of times: a 6 400-triangle grass tuft expands to
            // 51 arguments, so a painted field costs 100 000 draws a frame to
            // cull sub-parts of things a few pixels across. Count how often
            // each mesh appears and fall back to one whole-mesh argument once
            // it is clearly being instanced.
            self.profiler.cpu_begin("Cluster cull");
            self.instanced_counts.clear();
            for cmd in &self.draw_queue {
                *self
                    .instanced_counts
                    .entry(cmd.vertex_offset)
                    .or_insert(0u32) += 1;
            }

            self.cluster_args.clear();
            self.cull_aabbs.clear();
            for pass_two_sided in [false, true] {
                if pass_two_sided {
                    self.single_sided_args = self.cluster_args.len();
                }
                for (i, cmd) in self.draw_queue.iter().enumerate() {
                    if self.is_double_sided(cmd.material_id) != pass_two_sided {
                        continue;
                    }
                    let heavily_instanced = self
                        .instanced_counts
                        .get(&cmd.vertex_offset)
                        .is_some_and(|n| *n > MAX_INSTANCES_FOR_CLUSTERING);
                    // Skip cluster expansion once a mesh is clearly instanced,
                    // but keep one argument per draw. Folding copies into
                    // `instance_count > 1` made the cull shader (which writes
                    // 0 or 1) keep only the first tree and drop the rest.
                    let meshlets = if self.meshlet_draws && !heavily_instanced {
                        self.geometry.mesh_meshlets(cmd.vertex_offset)
                    } else {
                        None
                    };
                    let start = self.cull_aabbs.len();
                    crate::indirect::push_cluster_args(
                        i as u32,
                        cmd.index_count,
                        1,
                        meshlets,
                        self.geometry.mesh_aabb(cmd.vertex_offset),
                        &mut self.cluster_args,
                        &mut self.cull_aabbs,
                    );
                    // Normal-cone rejection assumes the vis pass culls back
                    // faces. Two-sided foliage keeps those faces, so a trunk
                    // cluster that faces away is still the bark you should see
                    // from the other side of a second tree.
                    if pass_two_sided {
                        for aabb in &mut self.cull_aabbs[start..] {
                            aabb.cone[3] = 2.0;
                        }
                    }
                }
            }
            self.indirect
                .upload(&ctx.device, &ctx.queue, &self.cluster_args);
            let counted_draws = self.counted_draws_active();
            self.cull_pass.update(
                &ctx.device,
                &ctx.queue,
                &self.cull_aabbs,
                // Un-jittered: a visibility decision must not depend on a
                // sub-pixel sampling offset. With the jittered matrix the
                // frustum planes — and the Hi-Z occlusion test behind them —
                // moved every frame, so any cluster sitting on the threshold
                // was culled on some frames and drawn on others. That is
                // geometry appearing and disappearing at jitter frequency,
                // which reads as the mesh vibrating, and it hits foliage
                // hardest because it is thousands of small clusters all near
                // the threshold at once.
                self.view_proj_unjittered,
                !self.culling_enabled,
                self.hiz_pass.size(),
                self.hiz_pass.mip_count(),
                self.hiz_ready && !self.occlusion_off,
                self.camera_pos,
                self.single_sided_args,
                counted_draws,
            );
            self.profiler.cpu_end();
        }

        // ── 5. Shadow Pass (4 cascades into the atlas) ───────────────────────
        //
        // Phase 24AE: cull casters too small to be worth a shadow before any of
        // them reach the atlas. Phase CR-E also drops casters that miss every
        // cascade volume. See `rebuild_shadow_casters`.
        self.rebuild_shadow_casters();
        let dirty_cascades = self.cascade_shadow_cache.finish_frame(
            self.cascade_shadow_revisions,
            cascade_cache_frame.view_dirty,
            shadow_cache_enabled,
        );
        self.profiler.counters.shadow_cascades_rendered =
            dirty_cascades.into_iter().map(u32::from).sum();
        self.prepare_virtual_shadow_cache(&ctx.queue, &cascades);
        self.profiler.counters.shadow_casters =
            u32::try_from(self.shadow_caster_scratch.len()).unwrap_or(u32::MAX);
        if !self.virtual_shadow_work.is_empty() {
            let clear_atlas = self
                .virtual_shadow_gpu
                .as_mut()
                .is_some_and(crate::shadow::virtual_map::VirtualShadowGpu::take_full_clear);
            self.profiler.begin(&mut encoder, "VSM Pages");
            self.shadow_pass.record_virtual(
                &ctx.queue,
                &mut encoder,
                self.virtual_shadow_gpu
                    .as_ref()
                    .expect("VSM work requires physical resources"),
                &self.global_pool.bind_group,
                &self.shadow_caster_scratch,
                &self.virtual_shadow_work,
                clear_atlas,
            );
            self.profiler.end(&mut encoder);
        }
        self.profiler.begin(&mut encoder, "Shadows");
        match self.directional_shadow_technique() {
            crate::shadow::virtual_map::ShadowTechnique::Cascaded => self.shadow_pass.record(
                &mut encoder,
                &self.shadow_resources.atlas_view,
                &self.global_pool.bind_group,
                &self.shadow_caster_scratch,
                dirty_cascades,
            ),
            crate::shadow::virtual_map::ShadowTechnique::Virtual => {
                // Keep a coarse CSM render as the authored policy's
                // last-resort page-miss fallback. Sparse page raster happened
                // above; opaque, terrain, and water consumers sample it later.
                self.shadow_pass.record(
                    &mut encoder,
                    &self.shadow_resources.atlas_view,
                    &self.global_pool.bind_group,
                    &self.shadow_caster_scratch,
                    dirty_cascades,
                );
            }
        }
        self.profiler.end(&mut encoder);

        // ── 5.5 Phase 15B/15E2: two-phase GPU instance culling ───────────────
        //
        //   cull phase 1   frustum, then occlusion against LAST frame's pyramid
        //   visibility     draw the survivors (clears the targets)
        //   Hi-Z build     pyramid now reflects what is actually on screen
        //   cull phase 2   re-test only what phase 1 rejected on occlusion
        //   visibility     draw whatever became visible (loads, never clears)
        //   Hi-Z build     final pyramid, which next frame's phase 1 reads
        //
        // Reprojecting the previous frame alone would drop geometry the moment
        // the camera moves. The second phase is what makes that safe: anything
        // wrongly rejected gets a look at fresh depth within the same frame.
        let cull_active = self.gpu_driven && !self.indirect.is_empty();
        let counted_draws = self.counted_draws_active();

        if cull_active {
            // Phase DOOM-A: bracketed. This was one of the passes the §17.7
            // table listed as landing in `unattributed`, which is why the row
            // existed at all.
            self.profiler.begin(&mut encoder, "Cull (phase 1)");
            self.cull_pass.record(
                &ctx.device,
                &mut encoder,
                &self.instances.buffer,
                &self.indirect.buffer,
                &self.hiz_pass.view,
                0,
                self.indirect.len(),
                counted_draws,
            );
            self.profiler.end(&mut encoder);
        }

        if self.cull_stats && cull_active {
            self.snapshot_indirect(&ctx.device, &mut encoder, 0);
        }

        // ── 6. Visibility Pass (phase 1) ─────────────────────────────────────
        self.profiler.begin(&mut encoder, "Visibility (phase 1)");
        self.record_visibility(&mut encoder, true);
        self.profiler.end(&mut encoder);

        // ── 6.5 Hi-Z pyramid from phase 1 depth ──────────────────────────────
        self.profiler.begin(&mut encoder, "Hi-Z");
        self.hiz_pass.record(&mut encoder);
        self.profiler.end(&mut encoder);

        if cull_active {
            // ── 6.7 Cull phase 2 ─────────────────────────────────────────────
            self.profiler.begin(&mut encoder, "Cull (phase 2)");
            self.cull_pass.record(
                &ctx.device,
                &mut encoder,
                &self.instances.buffer,
                &self.indirect.buffer,
                &self.hiz_pass.view,
                1,
                self.indirect.len(),
                counted_draws,
            );
            self.profiler.end(&mut encoder);

            if self.cull_stats {
                self.snapshot_indirect(&ctx.device, &mut encoder, 1);
            }

            // ── 6.8 Visibility Pass (phase 2) — disocclusions ────────────────
            self.profiler.begin(&mut encoder, "Visibility (phase 2)");
            self.record_visibility(&mut encoder, false);
            self.profiler.end(&mut encoder);

            // ── 6.9 Final pyramid, for the next frame's phase 1 ──────────────
            self.profiler.begin(&mut encoder, "Hi-Z (phase 2)");
            self.hiz_pass.record(&mut encoder);
            self.profiler.end(&mut encoder);
        }

        // Occlusion culling stays off until a pyramid has been built from real
        // geometry. wgpu zero-fills a new texture, and zero is the near plane,
        // which would read as "everything is occluded" on the first frame.
        self.hiz_ready = true;

        // The Phase 25A-1 terrain depth prepass stood here. It is gone with
        // 25A-2: terrain is drawn by the visibility pass above, which fills the
        // same depth buffer before the acceleration-structure build, ReSTIR and
        // GTAO read it. Keeping the prepass as well would draw every chunk a
        // second time, from whatever LOD state the previous frame left behind.

        // ── 6.5 Acceleration structures (Phase 24J) ──────────────────────────
        // The top level is rebuilt each frame from the same draw queue the
        // raster path uses, so the traced scene and the drawn one cannot drift
        // apart. Bottom-level structures are rebuilt only where the geometry
        // changed — a sculpt stroke, or a mesh's first frame (Phase 25B).
        if self.raytrace_pass.supported() {
            let instances: Vec<(u32, u32, glam::Mat4)> = self
                .draw_queue
                .iter()
                .enumerate()
                .map(|(i, cmd)| {
                    (
                        u32::try_from(i).unwrap_or(0),
                        cmd.vertex_offset,
                        cmd.transform,
                    )
                })
                .collect();
            self.profiler.begin(&mut encoder, "TLAS build");
            self.raytrace_pass.build(
                &ctx.device,
                &mut encoder,
                &self.geometry.vertex_buffer,
                &self.geometry.index_buffer,
                &instances,
            );
            self.profiler.end(&mut encoder);

            // Phase 24K: traced direct lighting. Here because it needs the TLAS
            // built above and the depth the visibility pass filled, and because
            // shading below consumes its result.
            if let Some(tlas) = self.raytrace_pass.tlas() {
                self.profiler.begin(&mut encoder, "ReSTIR DI");
                self.restir_pass.record(
                    &ctx.device,
                    &ctx.queue,
                    &mut encoder,
                    tlas,
                    &self.vis_pass.depth_view,
                    self.view_proj,
                    self.light_direction,
                    self.sun_angular_radius,
                    self.render_width,
                    self.render_height,
                );
                self.profiler.end(&mut encoder);

                // Phase 24L. After the DI pass, and for the same reasons: the
                // TLAS is built and the visibility pass has filled depth and
                // the id buffer this reads its primary normals from.
                self.profiler.begin(&mut encoder, "ReSTIR GI");
                self.restir_gi_pass.record(
                    &ctx.device,
                    &ctx.queue,
                    &mut encoder,
                    &self.global_pool.bind_group,
                    tlas,
                    &self.vis_pass.depth_view,
                    &self.vis_pass.view,
                    self.view_proj,
                    self.camera_pos,
                    self.light_direction,
                    self.light_color,
                    self.render_width,
                    self.render_height,
                );
                self.profiler.end(&mut encoder);
            }
        }

        {
            self.profiler.cpu_begin("Lighting extra");
            let mesh_sdf: Vec<crate::pass::lighting_extra::MeshSdfDraw> = self
                .draw_queue
                .iter()
                .filter_map(|cmd| {
                    let (min, max) = self.geometry.mesh_aabb(cmd.vertex_offset)?;
                    Some(crate::pass::lighting_extra::MeshSdfDraw {
                        model: cmd.transform,
                        local_min: min,
                        local_max: max,
                        vertex_offset: cmd.vertex_offset,
                        base_color: self.materials_pool.get(cmd.material_id).map_or(
                            [0.5; 3],
                            |material| {
                                [
                                    material.base_color[0],
                                    material.base_color[1],
                                    material.base_color[2],
                                ]
                            },
                        ),
                        brick: self.geometry.mesh_sdf(cmd.vertex_offset),
                    })
                })
                .collect();
            self.profiler.begin(&mut encoder, "Lighting extra");
            let traced_scene_revision = if (self.lighting_extra_pass.flags_bits()
                & (crate::pass::lighting_extra::FLAG_PATH
                    | crate::pass::lighting_extra::FLAG_SPECULAR))
                != 0
            {
                self.traced_scene_revision()
            } else {
                0
            };
            let sdf_scene_revision = if self.ddgi_pass.enabled() {
                self.sdf_scene_revision()
            } else {
                traced_scene_revision
            };
            self.lighting_extra_pass.record(
                &ctx.device,
                &ctx.queue,
                &mut encoder,
                &self.global_pool.bind_group,
                self.raytrace_pass.tlas(),
                &self.vis_pass.depth_view,
                &self.vis_pass.view,
                self.restir_gi_pass
                    .radiance_view()
                    .expect("ReSTIR GI always allocates its radiance target"),
                &self.ibl_pass.cube_view,
                &self.ibl_pass.sampler,
                self.view_proj,
                self.view_matrix,
                self.proj_matrix,
                traced_scene_revision,
                sdf_scene_revision,
                self.camera_pos,
                self.render_width,
                self.render_height,
                &mesh_sdf,
            );
            self.profiler.end(&mut encoder);
            self.profiler.cpu_end();

            // MORROWIND-AB: this pass is deliberately outside the ray-query
            // guard. It consumes the portable software SDF populated above.
            self.profiler.begin(&mut encoder, "DDGI");
            self.ddgi_pass.record(
                &ctx.device,
                &ctx.queue,
                &mut encoder,
                self.lighting_extra_pass.volume_view(),
                &self.ibl_pass.cube_view,
                &self.ibl_pass.sampler,
                self.camera_pos,
                self.light_direction,
                self.light_color,
                self.lighting_extra_pass
                    .sdf_scene_revision()
                    .unwrap_or(sdf_scene_revision),
            );
            self.profiler.end(&mut encoder);

            let mut lighting_params = self.lighting_extra_pass.shading_params();
            // The existing SH gather interprets z/w as cell size and
            // half-grid extent. Publish the actual DDGI lattice rather than
            // the 64^3 SDF volume's dimensions.
            self.ddgi_pass.publish_shading_lattice(&mut lighting_params);
            self.shading_pass
                .set_lighting_extra(&ctx.queue, lighting_params);
        }

        // Outside the ray-tracing guards on purpose: a pass that stopped running
        // still owns a stale target, and that is exactly the case that needs
        // clearing.
        // Bracketed (DOOM-A) because a full-resolution clear is not free at
        // maximized Native, and a pass that has been switched off still paying
        // for its own target is exactly the kind of cost that hides in a row
        // called `unattributed`.
        self.profiler.begin(&mut encoder, "ReSTIR clear");
        self.restir_pass.clear_if_inactive(&mut encoder);
        self.restir_gi_pass.clear_if_inactive(&mut encoder);
        self.profiler.end(&mut encoder);

        // ── 6.9 GTAO (Phase 24I) ─────────────────────────────────────────────
        // After the visibility pass has filled depth, before shading reads it.
        self.gtao_pass
            .ensure_bind_groups(&ctx.device, &self.vis_pass.depth_view);
        // ── 6.8 Velocity (Phase 24AD) ────────────────────────────────────────
        // After the visibility pass has finished writing depth and before
        // anything that walks backwards through time reads it.
        self.profiler.begin(&mut encoder, "Velocity");
        self.velocity_pass.record(
            &ctx.queue,
            &mut encoder,
            self.view_proj_unjittered,
            self.render_width,
            self.render_height,
        );
        self.profiler.end(&mut encoder);

        self.profiler.begin(&mut encoder, "GTAO");
        self.gtao_pass.record(
            &mut encoder,
            &ctx.queue,
            self.proj_matrix,
            self.render_width,
            self.render_height,
            0.1,
        );

        // ── 6.95 Froxel volumetrics (Phases 24U, 25I) ────────────────────────
        //
        // After the shadow pass, whose atlas it samples for light shafts, and
        // before shading, which consumes the volume. The atmosphere LUTs are
        // already built by `ensure_built` above.
        self.volumetric_pass.ensure_bind_group(
            &ctx.device,
            self.atmosphere_pass.transmittance_view(),
            self.atmosphere_pass.multiscatter_view(),
            self.atmosphere_pass.sampler(),
            &self.global_pool.light_buffer,
            &self.shadow_resources.atlas_depth_view,
        );
        self.profiler.end(&mut encoder);
        self.profiler.begin(&mut encoder, "Volumetrics");
        self.volumetric_pass.record(
            &mut encoder,
            &ctx.queue,
            self.view_proj_unjittered.inverse(),
            self.view_proj_unjittered,
            self.view_matrix,
            self.camera_pos,
            self.light_direction,
            self.light_color,
        );
        self.profiler.end(&mut encoder);
        self.shading_pass
            .set_volumetric_range(&ctx.queue, self.volumetric_pass.max_distance());

        // ── 6.9 Volumetric clouds (Phase CONTROL-M) ──────────────────────────
        //
        // Marched here rather than after shading because it reads the froxel
        // volume for its own aerial perspective, and because a compute
        // dispatch issued before the shading draw can overlap with it. The
        // *composite* has to wait until shading has drawn the sky, and does —
        // see 7.4 below.
        self.profiler.begin(&mut encoder, "Clouds");
        self.cloud_pass.ensure_bind_groups(
            &ctx.device,
            self.atmosphere_pass.transmittance_view(),
            self.atmosphere_pass.multiscatter_view(),
            self.atmosphere_pass.sampler(),
            &self.vis_pass.depth_view,
            &self.volumetric_pass.view,
        );
        self.cloud_pass.record(
            &mut encoder,
            &ctx.queue,
            self.view_proj_unjittered.inverse(),
            self.camera_pos,
            self.light_direction,
            self.light_color,
            self.volumetric_pass.max_distance(),
        );
        self.profiler.end(&mut encoder);

        // ── 6.95 Terrain clipmap generate (Phase DF) ─────────────────────────
        // World XZ, no FSR jitter. Generate paints array layers as color
        // attachments; shade samples the same images (group 2).
        self.profiler.begin(&mut encoder, "Terrain clipmap");
        let mut work: Vec<(usize, u32)> = Vec::new();
        for i in 0..self.clipmaps.len() {
            let enabled = self.clipmaps[i].enabled
                && !crate::terrain::clipmap::TerrainClipmap::env_forced_off();
            if !enabled {
                continue;
            }
            let Some(terrain) = self.terrains.get(i) else {
                continue;
            };
            if !self.clipmaps[i].has_dirty() && !terrain.has_pending_virtual_texture() {
                continue;
            }
            work.push((i, terrain.terrain_index));
        }
        if !work.is_empty() {
            // Every `record` this frame takes its own slice of the generate
            // pass's uniform buffer. wgpu applies queue writes in call order
            // just before the frame's passes, so two calls writing at offset 0
            // did not take turns -- the detail stack was generated with the
            // macro stack's rectangle and centre.
            self.clipmap_pass.begin_frame(&ctx.device, work.len());
            let mut budget = crate::terrain::clipmap::MAX_GEN_TEXELS;
            for (i, terrain_index) in work {
                // Coverage before sharpness. Detail used to take the whole
                // budget first, and on a cold cache it exhausted it every
                // frame, so the macro stack -- the only one that covers the
                // whole view -- was starved for the ten-odd frames the detail
                // rings took to fill. Everything it would have shaded spent
                // those frames on the flat macro-map fallback instead.
                let (detail, macro_jobs) = if self.clipmaps[i].macro_covers_view() {
                    let detail = self.clipmaps[i].take_jobs(true, &mut budget);
                    let macro_jobs = self.clipmaps[i].take_jobs(false, &mut budget);
                    (detail, macro_jobs)
                } else {
                    let macro_jobs = self.clipmaps[i].take_jobs(false, &mut budget);
                    let detail = self.clipmaps[i].take_jobs(true, &mut budget);
                    (detail, macro_jobs)
                };
                let mut feedback_jobs = Vec::with_capacity(detail.len() + macro_jobs.len());
                feedback_jobs.extend_from_slice(&detail);
                feedback_jobs.extend_from_slice(&macro_jobs);
                let vt_uploaded = self.terrains.get_mut(i).is_some_and(|terrain| {
                    terrain.feedback_virtual_texture(&ctx.queue, &feedback_jobs)
                });
                let virtual_texture = self
                    .terrains
                    .get(i)
                    .filter(|terrain| terrain.virtual_texture_enabled)
                    .map_or([-1, -1, -1, 0], |terrain| {
                        terrain.texture_ids.virtual_texture
                    });
                if let Some(terrain) = self.terrains.get(i) {
                    let model = self
                        .terrain_queue
                        .iter()
                        .find(|t| t.0 as usize == i)
                        .map(|t| t.1)
                        .unwrap_or(glam::Mat4::IDENTITY);
                    let local_cam = model.inverse().transform_point3(self.camera_pos);
                    let mut mat = terrain.gpu_material_for_camera(local_cam);
                    self.clipmaps[i].fill_gpu(&mut mat);
                    self.terrain_materials
                        .write(&ctx.queue, terrain_index, &mat);
                }
                self.clipmap_pass.record(
                    &ctx.device,
                    &ctx.queue,
                    &mut encoder,
                    &self.global_pool.bind_group,
                    &self.clipmaps[i],
                    terrain_index,
                    &detail,
                    true,
                    virtual_texture,
                );
                self.clipmap_pass.record(
                    &ctx.device,
                    &ctx.queue,
                    &mut encoder,
                    &self.global_pool.bind_group,
                    &self.clipmaps[i],
                    terrain_index,
                    &macro_jobs,
                    false,
                    virtual_texture,
                );
                // A later feedback batch can replace mean/ancestor samples
                // baked by an earlier batch. Recompose after the current jobs
                // have rendered so page arrival cannot leave stale texels.
                if vt_uploaded {
                    self.clipmaps[i].invalidate();
                }
                if clipmap_trace() {
                    let frame =
                        CLIPMAP_TRACE_FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let (detail_ready, macro_ready) = self.clipmaps[i].ready_masks();
                    let vt = self
                        .terrains
                        .get(i)
                        .map(|t| *t.virtual_texture_stats())
                        .unwrap_or_default();
                    tracing::info!(
                        frame,
                        detail_ready = format!("{detail_ready:08b}"),
                        macro_ready = format!("{macro_ready:04b}"),
                        detail_jobs = detail.len(),
                        macro_jobs = macro_jobs.len(),
                        queued_texels = self.clipmaps[i].pending_texels(),
                        vt_resident = vt.resident_pages,
                        vt_pending = vt.pending_pages,
                        vt_uploads = vt.uploads,
                        vt_evictions = vt.evictions,
                        invalidated = vt_uploaded,
                        "DF-TRACE"
                    );
                }
            }
        }
        self.profiler.end(&mut encoder);

        // ── 6.98 Pixel census (Phase DOOM-B) ─────────────────────────────────
        //
        // Immediately before shading, reading the same visibility buffer and
        // depth that shading is about to read, so the classification cannot
        // disagree with the pass it is describing. Default off.
        self.census_pass.ensure_bind_group(
            &ctx.device,
            &self.vis_pass.view,
            &self.vis_pass.depth_view,
            self.render_width,
            self.render_height,
        );
        self.profiler.begin(&mut encoder, "Census");
        self.census_pass
            .record(&mut encoder, &ctx.queue, &self.global_pool.bind_group);
        self.profiler.end(&mut encoder);

        // ── 6.99 Tile classification (Phase DOOM-C) ──────────────────────────
        //
        // Same inputs as the census and the same `pc_classify`, one dispatch
        // later: the census counts pixels, this routes tiles.
        // Allocated even when binning is off: group 3 is part of the shading
        // pipeline layout either way, so the bind group has to exist for the
        // fullscreen draw as well. Only the dispatch is conditional.
        if self.classify_pass.ensure(
            &ctx.device,
            &self.vis_pass.view,
            &self.vis_pass.depth_view,
            self.render_width,
            self.render_height,
        ) || self.shading_pass.tile_bind_group().is_none()
        {
            self.shading_pass
                .set_tile_buffer(&ctx.device, self.classify_pass.tiles_buffer());
        }
        if self.classify_pass.enabled {
            self.shading_pass.write_tile_params(
                &ctx.queue,
                self.classify_pass.tiles_x(),
                crate::pass::classify::TILE_SIZE,
                self.classify_pass.tile_count(),
                self.render_width,
                self.render_height,
            );
            self.profiler.begin(&mut encoder, "Classify");
            self.classify_pass
                .record(&mut encoder, &ctx.queue, &self.global_pool.bind_group);
            self.profiler.end(&mut encoder);
        }

        // Compact PSO when hex/POM/PCSS are off. Recreate is a hitch, not a
        // per-frame cost; Island stays on COMPACT and never pays it.
        {
            let restir_sun = self.restir_pass.active() && self.raytrace_pass.tlas().is_some();
            let mut spec = crate::pass::shading::ShadingSpec {
                hex: false,
                pom: false,
                pcss: (self.shading_mode & 2) != 0 && !restir_sun,
                contact: (self.shading_mode & 4) != 0 && !restir_sun,
                clipmap: false,
                debug: self.shading_debug != 0.0,
                terrain_scan: crate::terrain::textures::TERRAIN_HERO_LAYERS,
                live_terrain: false,
                // Phase DOOM-B. Read once at startup and held constant for the
                // process: an ablation that could change mid-run would recreate
                // the PSO and put a shader compile inside the window being
                // timed.
                ablate: self.shade_ablate,
            };
            // Phase DF audit: a clipmapped terrain shades from the cache, which
            // already holds strongest-four + hex + height-blend and never
            // marches POM. Folding its `hex_tiling` / `parallax_scale` into the
            // spec kept both of those compiled into the shading shader for a
            // path that cannot execute, and occupancy is the union of what is
            // in the module — the same trap the runtime checkboxes fell into.
            //
            // The clipmap test must match `TerrainClipmap::fill_gpu` exactly.
            // If it says "clipmapped" where `fill_gpu` writes
            // `clipmap_enabled = 0`, the live path is deleted from under a
            // terrain that still needs it and the ground renders unshaded.
            let clipmap_forced_off = crate::terrain::clipmap::TerrainClipmap::env_forced_off();
            for &(id, _) in &self.terrain_queue {
                let Some(t) = self.terrains.get(id as usize) else {
                    continue;
                };
                let clipmapped = self
                    .clipmaps
                    .get(id as usize)
                    .is_some_and(|c| c.enabled && !clipmap_forced_off);
                if clipmapped {
                    spec.clipmap = true;
                    continue;
                }
                spec.live_terrain = true;
                spec.hex |= t.hex_tiling;
                spec.pom |= t.parallax_scale > 0.0;
                if !t.hero_bank_only {
                    spec.terrain_scan = crate::terrain::textures::TERRAIN_LAYER_COUNT;
                }
            }
            // No terrain queued at all: keep the live path so the next frame's
            // first terrain does not have to wait on a pipeline rebuild.
            if self.terrain_queue.is_empty() {
                spec.live_terrain = true;
            }
            self.shading_pass.ensure_pipeline(&ctx.device, spec);

            // ── Phase DOOM-E: the aerial split ───────────────────────────────
            //
            // Two fullscreen draws instead of one, separated by a depth test
            // against the scene depth the visibility pass already filled. The
            // near draw keeps everything closer than `aerial_split`; the aerial
            // draw takes the rest, including the sky at the cleared far value.
            //
            // What the aerial pipeline drops is what only resolves close up:
            // hex tiling, which exists to break the repetition of a tiled layer
            // and cannot be seen at distance, and the parallax march, which
            // `gpu_material_for_camera` already fades out with height. Both are
            // *deleted from the pipeline*, not skipped at runtime — a per-pixel
            // sample-count branch is exactly what XV-Zeta §11.1 forbids, and a
            // second pipeline is how the same effect is had without one.
            //
            // Enabled only when the near spec actually has something to drop.
            // Compiling a second pipeline identical to the first would cost a
            // shader compile and a second full-screen pass to save nothing.
            self.aerial_split_active = self.aerial_split_enabled
                && (spec.hex || spec.pom)
                && self.shade_ablate == crate::pass::shading::ablate::OFF;
            if self.aerial_split_active {
                let aerial = crate::pass::shading::ShadingSpec {
                    hex: false,
                    pom: false,
                    terrain_scan: if self.aerial_hero_bank {
                        crate::terrain::textures::TERRAIN_HERO_LAYERS
                    } else {
                        spec.terrain_scan
                    },
                    ..spec
                };
                self.shading_pass
                    .ensure_split_pipelines(&ctx.device, spec, aerial);
            }

            // Phase DOOM-C/E: the same spec, narrowed per bin.
            //
            // This is the whole point of the phase. `spec` above is the union
            // of everything any pixel on screen might need, which is what the
            // one fullscreen draw has to be compiled for. A tile that contains
            // only sky needs almost none of it, and a tile of terrain 400 m away
            // needs less than one at the player's feet.
            if self.classify_pass.enabled {
                use crate::pass::classify as bins;
                for bin in 0..bins::BIN_COUNT {
                    let mut s = spec;
                    match bin {
                        // Sky, mesh and foliage never reach the terrain
                        // material, so the 32-wide splat scan, hex tiling, POM
                        // and the clipmap sampler all leave the module. Sky
                        // additionally never samples a shadow map.
                        bins::BIN_SKY => {
                            s.live_terrain = false;
                            s.clipmap = false;
                            s.hex = false;
                            s.pom = false;
                            s.pcss = false;
                            s.contact = false;
                            s.terrain_scan = crate::terrain::textures::TERRAIN_HERO_LAYERS;
                        }
                        bins::BIN_MESH | bins::BIN_FOLIAGE => {
                            s.live_terrain = false;
                            s.clipmap = false;
                            s.hex = false;
                            s.pom = false;
                            s.terrain_scan = crate::terrain::textures::TERRAIN_HERO_LAYERS;
                        }
                        // DOOM-E. Terrain past the aerial split drops the two
                        // things that only resolve close up: hex tiling, which
                        // exists to break tiling repetition the eye can only
                        // see near the camera, and the parallax march, which
                        // `gpu_material_for_camera` already fades out with
                        // distance. Both are *deleted* from this pipeline
                        // rather than skipped at runtime — a per-pixel
                        // sample-count branch is what XV-Zeta §11.1 forbids,
                        // and a separate pipeline is precisely the way to get
                        // the same effect without one.
                        bins::BIN_TERRAIN_AERIAL => {
                            s.hex = false;
                            s.pom = false;
                        }
                        // Near terrain and mixed tiles keep the full spec.
                        _ => {}
                    }
                    self.shading_pass.ensure_bin_pipeline(&ctx.device, bin, s);
                }
            }
        }

        // ── 7. Shading Pass → HDR texture ────────────────────────────────────
        self.profiler.begin(&mut encoder, "Shading");
        // Phase DOOM-A: reserved *before* the pass, because the reservation
        // needs `&mut self.profiler` and everything inside the block below
        // borrows `self` immutably. The pass that dominates the frame is the
        // first one worth a fragment-invocation count: for a fullscreen
        // triangle it should be within a rounding of the pixel count, and any
        // large excess is the 2×2 derivative quads at the screen edge rather
        // than overdraw.
        let shading_stats = self.profiler.reserve_stats("Shading");
        // Phase DOOM-E: clip-space depth of the split distance.
        //
        // Projected on the CPU rather than derived in the shader because the
        // projection is already here and the alternative is another uniform
        // nobody can check. Note this is depth *along the view axis*, while the
        // census and the tile classifier measure radial distance — they differ
        // slightly toward the screen corners, which is immaterial for a level
        // of detail split and would be worth stating if it ever were not.
        let split_depth = {
            let clip = self.proj_matrix * glam::Vec4::new(0.0, 0.0, -self.aerial_split, 1.0);
            if clip.w.abs() > 1e-6 {
                (clip.z / clip.w).clamp(0.0, 1.0)
            } else {
                1.0
            }
        };
        if self.aerial_split_active {
            self.shading_pass.write_split_params(
                &ctx.queue,
                split_depth,
                self.render_width,
                self.render_height,
            );
        }
        {
            let stats_set = self.profiler.stats_query_set();
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shading Pass"),
                multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.postprocess_pass.hdr_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.background_color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                // Read-only: the visibility pass owns this buffer and several
                // later passes read it. Attached only when the split is live,
                // because a pipeline with no depth state cannot be used in a
                // pass that has an attachment.
                depth_stencil_attachment: self.aerial_split_active.then(|| {
                    wgpu::RenderPassDepthStencilAttachment {
                        view: &self.vis_pass.depth_view,
                        depth_ops: None,
                        stencil_ops: None,
                    }
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            if let (Some(qs), Some(index)) = (stats_set, shading_stats) {
                rpass.begin_pipeline_statistics_query(qs, index);
            }
            rpass.set_bind_group(0, &self.global_pool.bind_group, &[]);
            rpass.set_bind_group(1, &self.shading_pass.bind_group, &[]);
            rpass.set_bind_group(2, &self.shading_pass.clipmap_bind_group, &[]);

            // Phase DOOM-C: one indirect draw per bin, each an instanced quad
            // per tile against that bin's pipeline. The fragment shader is the
            // same code the fullscreen path runs, which is what makes the
            // capture comparison between the two a real gate.
            let binned = self.classify_pass.enabled;
            match (binned, self.shading_pass.tile_bind_group()) {
                (true, Some(tile_bg)) => {
                    for bin in 0..crate::pass::classify::BIN_COUNT {
                        let Some(pipeline) = self.shading_pass.bin_pipeline(bin) else {
                            continue;
                        };
                        rpass.set_pipeline(pipeline);
                        rpass.set_bind_group(
                            3,
                            tile_bg,
                            &[crate::pass::shading::ShadingPass::tile_params_offset(bin)],
                        );
                        // Instance count lives on the GPU — the classifier
                        // wrote it and the CPU never learns it, which is the
                        // point: reading it back would cost a stall to save an
                        // empty draw.
                        rpass.draw_indirect(
                            self.classify_pass.draw_args_buffer(),
                            crate::pass::classify::ClassifyPass::args_offset(bin),
                        );
                    }
                }
                _ => {
                    // Group 3 is in the pipeline layout either way, so it still
                    // has to be bound; on the un-split path `vs_main` reads
                    // only `split_depth` out of it.
                    if let Some(tile_bg) = self.shading_pass.tile_bind_group() {
                        rpass.set_bind_group(3, tile_bg, &[0]);
                    }
                    match (
                        self.aerial_split_active,
                        self.shading_pass.split_near_pipeline(),
                        self.shading_pass.split_aerial_pipeline(),
                    ) {
                        // Phase DOOM-E. Two draws of the same triangle at the
                        // split depth; early-Z rejects the half each pipeline
                        // does not own before any fragment of the expensive
                        // shader runs.
                        (true, Some(near), Some(aerial)) => {
                            rpass.set_pipeline(near);
                            rpass.draw(0..3, 0..1);
                            rpass.set_pipeline(aerial);
                            rpass.draw(0..3, 0..1);
                        }
                        _ => {
                            rpass.set_pipeline(&self.shading_pass.pipeline);
                            rpass.draw(0..3, 0..1);
                        }
                    }
                }
            }
            if shading_stats.is_some() && stats_set.is_some() {
                rpass.end_pipeline_statistics_query();
            }
        }
        self.profiler.end(&mut encoder);

        // ── 7.35 Frame capture, if one was asked for ─────────────────────────
        //
        // Here rather than at the end of the frame: this is the last point at
        // which the HDR target holds exactly the shading pass's own output,
        // before water, transparents, the editor grid, TAA and tone mapping.
        // An A/B of the shading path should not have to see through any of
        // those.
        let capture_now = self.capture.tick();
        let capture_after_water =
            std::env::var("SOMNIUM_CAPTURE_AFTER_WATER").as_deref() == Ok("1");
        let capture_after_taa = std::env::var("SOMNIUM_CAPTURE_AFTER_TAA").as_deref() == Ok("1");
        if capture_now && !capture_after_water && !capture_after_taa {
            // The switches an A/B is meant to be varying, recorded with the
            // capture so a null result can be told from a switch that never
            // reached the pass — which is what the first run of this test hit.
            tracing::info!(
                "CAPTURE-STATE gtao={} gtao_intensity={} ibl_intensity={} taa={} restir={}",
                self.gtao_pass.enabled,
                self.gtao_pass.intensity,
                self.ibl_intensity,
                self.taa_pass.enabled(),
                self.restir_pass.enabled,
            );
            // What the terrain draws actually carry. "Submitted but no pixels"
            // has several causes that look identical from outside — no draws,
            // an empty index range, offsets pointing at unwritten pool space —
            // and this separates them without another build.
            let terrain_draws: Vec<&DrawCommand> = self
                .draw_queue
                .iter()
                .filter(|d| self.terrain_material_ids.contains(&d.material_id))
                .collect();
            if let Some(d) = terrain_draws.first() {
                let aabb = self.geometry.mesh_aabb(d.vertex_offset);
                tracing::info!(
                    "CAPTURE-TERRAIN draws={} tlas_instances={} first: v_off={} i_off={} i_count={} origin={:?} aabb={:?}",
                    terrain_draws.len(),
                    self.raytrace_pass.instance_count(),
                    d.vertex_offset,
                    d.index_offset,
                    d.index_count,
                    d.transform.w_axis.truncate().to_array(),
                    aabb,
                );
            } else {
                tracing::info!("CAPTURE-TERRAIN draws=0");
            }
            self.capture.record(
                &ctx.device,
                &mut encoder,
                &self.postprocess_pass.hdr_texture,
                &self.vis_pass.texture,
                self.render_width,
                self.render_height,
            );
        }

        // The terrain pass stood here (7.3). Terrain now shades in the pass
        // above, with GTAO, contact shadows, traced visibility, IBL and aerial
        // perspective reaching it for the first time — and with one copy of
        // `sample_shadow` and the cluster lookup instead of two. It still
        // writes depth before the water pass, because the visibility pass does.

        // ── 7.4 Cloud composite (Phase CONTROL-M) ────────────────────────────
        //
        // After the sky is in the HDR target and before water and transparents,
        // so a cloud is behind a wave and in front of the sky, which is where
        // clouds are. Before TAA too: the clouds land in the buffer TAA already
        // resolves, so they inherit 24F's jittered-matrix reprojection instead
        // of growing a second, naive history.
        self.profiler.begin(&mut encoder, "Cloud composite");
        self.cloud_pass
            .composite(&mut encoder, &self.postprocess_pass.hdr_view);
        self.profiler.end(&mut encoder);

        // ── 7.5 Water Pass → HDR texture ─────────────────────────────────────
        self.water_pass.clear_surface(&mut encoder);
        if !self.water_queue.is_empty() {
            // Phase 22: water refracts what is behind it, so it needs the scene
            // colour as a texture. A pass cannot sample its own render target,
            // hence the copy — taken here so it holds opaque geometry, terrain
            // and transparents, i.e. everything that can legitimately be seen
            // through the surface.
            encoder.copy_texture_to_texture(
                self.postprocess_pass.hdr_texture.as_image_copy(),
                self.postprocess_pass.scene_copy_texture.as_image_copy(),
                self.postprocess_pass.hdr_texture.size(),
            );

            self.profiler.begin(&mut encoder, "Water prepass");
            self.water_pass.record_prepass(
                &ctx.device,
                &ctx.queue,
                &mut encoder,
                &self.vis_pass.depth_view,
                &self.global_pool.view_proj_buffer,
                &self.vis_pass.depth_view,
                &self.global_pool.light_buffer,
                &self.shadow_resources.atlas_depth_view,
                &self.shadow_resources.comparison_sampler,
                &self.ibl_pass.cube_view,
                &self.ibl_pass.sampler,
                &self.postprocess_pass.scene_copy_view,
                self.velocity_pass.view(),
                self.view_proj_unjittered,
                self.time,
                &self.geometry.vertex_buffer,
                &self.geometry.index_buffer,
                self.water_textures_bind_group.as_ref(),
                &self.water_bodies,
                &self.water_queue,
            );
            self.profiler.end(&mut encoder);

            self.profiler.begin(&mut encoder, "Water reflection");
            let rt_strength = self.water_queue[0].2.volume_params[2];
            let tlas_overflowed = self.raytrace_pass.overflowed();
            let traced = if let Some(tlas) = self.raytrace_pass.tlas() {
                self.water_reflection_pass.record(
                    &ctx.device,
                    &ctx.queue,
                    &mut encoder,
                    &self.global_pool.bind_group,
                    tlas,
                    self.water_pass.surface_view(),
                    self.water_pass.roughness_view(),
                    self.velocity_pass.view(),
                    &self.ibl_pass.cube_view,
                    &self.ibl_pass.sampler,
                    &self.shadow_resources.atlas_depth_view,
                    &self.shadow_resources.comparison_sampler,
                    self.view_matrix,
                    self.view_proj,
                    self.camera_pos,
                    rt_strength,
                    tlas_overflowed,
                    self.render_width,
                    self.render_height,
                )
            } else {
                false
            };
            self.profiler.end(&mut encoder);

            self.profiler.begin(&mut encoder, "Water shade");
            self.water_pass.record_shade(
                &ctx.device,
                &ctx.queue,
                &mut encoder,
                &self.postprocess_pass.hdr_view,
                &self.vis_pass.depth_view,
                &self.global_pool.view_proj_buffer,
                &self.vis_pass.depth_view,
                &self.global_pool.light_buffer,
                &self.shadow_resources.atlas_depth_view,
                &self.shadow_resources.comparison_sampler,
                &self.ibl_pass.cube_view,
                &self.ibl_pass.sampler,
                &self.postprocess_pass.scene_copy_view,
                if traced {
                    self.water_reflection_pass.current_view()
                } else {
                    self.water_reflection_pass.dummy_view()
                },
                &self.geometry.vertex_buffer,
                &self.geometry.index_buffer,
                self.water_textures_bind_group.as_ref(),
                &self.water_bodies,
                &self.water_queue,
            );
            self.profiler.end(&mut encoder);
        }

        if capture_now && capture_after_water && !capture_after_taa {
            tracing::info!(
                "CAPTURE-WATER bodies={} draws={}",
                self.water_bodies.active_count(),
                self.water_queue.len(),
            );
            self.capture.record(
                &ctx.device,
                &mut encoder,
                &self.postprocess_pass.hdr_texture,
                &self.vis_pass.texture,
                self.render_width,
                self.render_height,
            );
        }

        // ── 7.6 Phase 21: blended geometry → HDR texture ─────────────────────
        // After opaque shading, terrain and water have filled the target, so
        // blended surfaces composite over a complete image. Depth-tested
        // against the opaque depth, never writing it.
        self.profiler.begin(&mut encoder, "Transparent");
        if self.oit_pass.enabled {
            // MORROWIND-AC. Same queue, same depth, same shading — the draws
            // are simply not required to be in any order, so the sort above is
            // wasted work rather than wrong work and is left in place: turning
            // OIT off mid-session must not need a re-sort.
            self.oit_pass.begin(&mut encoder);
            self.transparent_pass.record_weighted(
                &mut encoder,
                self.oit_pass.accum_view(),
                self.oit_pass.reveal_view(),
                &self.vis_pass.depth_view,
                &self.global_pool.bind_group,
                &transparent_draws,
            );
            self.oit_pass
                .composite(&mut encoder, &self.postprocess_pass.hdr_view);
        } else {
            self.transparent_pass.record(
                &mut encoder,
                &self.postprocess_pass.hdr_view,
                &self.vis_pass.depth_view,
                &self.global_pool.bind_group,
                &transparent_draws,
            );
        }

        // ── 7.7 Grid Overlay → HDR texture ───────────────────────────────────
        if self.editor_overlays_enabled && self.grid_enabled {
            self.grid_pass
                .record(&mut encoder, &self.postprocess_pass.hdr_view);
        }

        // ── 7.75 Ray-traced shadow debug (Phase 24J) ─────────────────────────
        // After shading, not before it. The shading pass clears the HDR target,
        // so a debug view written earlier is simply overwritten — which is
        // exactly what happened on the first attempt, and looks identical to
        // the acceleration structures silently not working.
        if self.raytrace_pass.supported() {
            if let Some(tlas) = self.raytrace_pass.tlas() {
                self.rt_debug_pass.record(
                    &ctx.device,
                    &ctx.queue,
                    &mut encoder,
                    tlas,
                    &self.vis_pass.depth_view,
                    &self.postprocess_pass.hdr_view,
                    self.view_proj,
                    self.light_direction,
                    self.render_width,
                    self.render_height,
                );
            }
        }

        // ── 7.8 TAA resolve (Phase 24F) ──────────────────────────────────────
        // Between the last thing that writes HDR and the metering, so exposure
        // is measured on the resolved image rather than on a jittered one.
        self.taa_pass.ensure_bind_groups(
            &ctx.device,
            &self.postprocess_pass.hdr_view,
            &self.vis_pass.depth_view,
            self.velocity_pass.view(),
            self.water_pass.surface_view(),
        );
        // TAA deliberately reprojects between unjittered matrices so a static
        // scene has zero velocity. See `TaaPass::record`.
        self.profiler.end(&mut encoder);

        // ── 7.85 Motion blur (Phase 24Z) ─────────────────────────────────────
        // Before TAA rather than after: TAA's history is what stabilises the
        // gather's dither, and blurring the resolved image instead would smear
        // a frame that has already been blended with its own past.
        self.profiler.begin(&mut encoder, "Motion Blur");
        self.motion_blur_pass.record(
            &ctx.device,
            &ctx.queue,
            &mut encoder,
            &self.postprocess_pass.hdr_view,
            &self.postprocess_pass.hdr_texture,
            self.velocity_pass.view(),
            &self.vis_pass.depth_view,
            self.render_width,
            self.render_height,
        );
        self.profiler.end(&mut encoder);

        self.profiler.begin(&mut encoder, "TAA");
        if self
            .taa_pass
            .record(
                &mut encoder,
                &ctx.queue,
                self.view_proj_unjittered,
                self.render_width,
                self.render_height,
            )
            .is_some()
        {
            // Copy back into the HDR target so every later pass keeps reading
            // one target, rather than a view that alternates each frame.
            let written = self.taa_pass.last_written();
            encoder.copy_texture_to_texture(
                self.taa_pass.resolved_texture(written).as_image_copy(),
                self.postprocess_pass.hdr_texture.as_image_copy(),
                self.postprocess_pass.hdr_texture.size(),
            );
        }
        // DOOM-A: the `TAA` scope used to stay open through Underwater, the
        // capture copy, auto-exposure and depth of field, so all four were
        // reported as TAA. Closing it here is a *reattribution*, not a change
        // in cost — expect the TAA row to fall and three new rows to appear
        // holding the difference.
        self.profiler.end(&mut encoder);

        // Phase IV-G: choose the finite body under the camera from the same
        // CPU query contract used by gameplay. Mesh-local XZ is centred, while
        // body query coordinates use the authored bounds origin.
        let active_water = self.water_queue.iter().find_map(|entry| {
            let (water_id, model, material, _, _, _) = *entry;
            let body = self.water_bodies.get(water_id)?;
            let local = model.inverse().transform_point3(self.camera_pos);
            let center = glam::Vec2::new(
                (body.descriptor.bounds[0] + body.descriptor.bounds[2]) * 0.5,
                (body.descriptor.bounds[1] + body.descriptor.bounds[3]) * 0.5,
            );
            let sample =
                body.sample_surface(glam::Vec2::new(local.x, local.z) + center, self.time)?;
            let signed_distance = local.y - (sample.height - body.descriptor.surface_level);
            Some((water_id, model, material, signed_distance))
        });
        self.underwater_body = active_water.map(|entry| entry.0);
        self.camera_submersion = active_water.map_or(0.0, |entry| {
            let t = ((0.08 - entry.3) / 0.16).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        });
        if let Some((water_id, model, material, signed_distance)) = active_water {
            encoder.copy_texture_to_texture(
                self.postprocess_pass.hdr_texture.as_image_copy(),
                self.postprocess_pass.scene_copy_texture.as_image_copy(),
                self.postprocess_pass.hdr_texture.size(),
            );
            if let Some(body) = self.water_bodies.get(water_id) {
                self.profiler.begin(&mut encoder, "Underwater");
                self.underwater_pass.record(
                    &ctx.device,
                    &ctx.queue,
                    &mut encoder,
                    &self.postprocess_pass.hdr_view,
                    &self.postprocess_pass.scene_copy_view,
                    &self.vis_pass.depth_view,
                    &self.global_pool.view_proj_buffer,
                    &self.global_pool.light_buffer,
                    water_id,
                    body,
                    model,
                    material,
                    self.time,
                    signed_distance,
                );
                self.profiler.end(&mut encoder);
            }
        }

        if capture_now && capture_after_taa {
            tracing::info!(
                "CAPTURE-TAA water_bodies={} water_draws={} taa={} underwater={:?} submersion={:.3}",
                self.water_bodies.active_count(),
                self.water_queue.len(),
                self.taa_pass.enabled(),
                self.underwater_body,
                self.camera_submersion,
            );
            self.capture.record(
                &ctx.device,
                &mut encoder,
                &self.postprocess_pass.hdr_texture,
                &self.vis_pass.texture,
                self.render_width,
                self.render_height,
            );
        }

        // ── 7.9 Auto-exposure: meter the finished HDR frame ──────────────────
        // Runs after everything that writes HDR and before tone mapping, so it
        // meters exactly the image being exposed. The reading lands one frame
        // late by construction — that is what adaptation is.
        if self.auto_exposure {
            self.profiler.begin(&mut encoder, "Auto exposure");
            self.auto_exposure_pass.record(
                &mut encoder,
                &ctx.queue,
                self.render_width,
                self.render_height,
                self.frame_delta_time,
                self.exposure_compensation,
            );
            self.profiler.end(&mut encoder);
        }

        // ── 7.93 Depth of field (Phase 24Z) ──────────────────────────────────
        // Before bloom, so the blurred highlights bloom rather than the sharp
        // ones — that ordering is what produces the soft circular flare a real
        // lens gives an out-of-focus light.
        self.dof_pass.ensure_bind_group(
            &ctx.device,
            &self.postprocess_pass.hdr_view,
            &self.vis_pass.depth_view,
        );
        self.profiler.begin(&mut encoder, "DoF");
        if let Some(result) = self.dof_pass.record(
            &mut encoder,
            &ctx.queue,
            self.render_width,
            self.render_height,
            0.1,
            1000.0,
        ) {
            encoder.copy_texture_to_texture(
                result.as_image_copy(),
                self.postprocess_pass.hdr_texture.as_image_copy(),
                self.postprocess_pass.hdr_texture.size(),
            );
        }
        self.profiler.end(&mut encoder);

        // ── 7.95 Bloom (Phase 24T) ───────────────────────────────────────────
        // After TAA, so the chain is built from a resolved image rather than a
        // jittered one; a blur of unstable input broadcasts that instability
        // across everything it touches.
        self.profiler.begin(&mut encoder, "Bloom");
        self.bloom_pass.record(
            &ctx.device,
            &ctx.queue,
            &mut encoder,
            &self.postprocess_pass.hdr_view,
            (self.render_width, self.render_height),
        );

        // ── 7.97 FSR 3 temporal reconstruct to display resolution ───────────
        // Replaces Somnium TAA and the bilinear present blit. HDR in, HDR out;
        // tone map runs after, at window size. RCAS is inside this dispatch.
        self.profiler.end(&mut encoder);
        self.profiler.begin(&mut encoder, "FSR");
        let fsr_ok = self.fsr_pass.record(
            &ctx.device,
            &ctx.queue,
            &mut encoder,
            &self.postprocess_pass.hdr_texture,
            &self.vis_pass.depth_texture,
            self.velocity_pass.texture(),
            self.exposure,
            self.proj_matrix,
            self.frame_delta_time,
            false,
        );
        if fsr_ok {
            self.postprocess_pass
                .bind_color(&ctx.device, self.fsr_pass.output_view());
        } else {
            self.postprocess_pass.bind_scene(&ctx.device);
        }

        // ── 8. Post-process Pass: HDR → swapchain (tone map + vignette) ──────
        // A TAA debug view must reach the screen unmodified: exposure would
        // crush a 0/1 flag image to black, and a tone curve would grade the
        // very values being inspected.
        let debugging = self.taa_pass.debugging();
        self.postprocess_pass.set_params(
            &ctx.queue,
            if debugging { 1.0 } else { self.exposure },
            if debugging {
                0.0
            } else {
                self.vignette_strength
            },
            if debugging {
                0.0
            } else {
                self.chromatic_aberration
            },
            if debugging { 3 } else { self.tonemapper },
            self.auto_exposure && !debugging,
            if debugging {
                0.0
            } else {
                self.bloom_pass.intensity()
            },
            // Grading is a look, and a debug view is not one — it must reach
            // the screen as the numbers it holds.
            if debugging {
                crate::pass::postprocess::Grading::default()
            } else {
                self.grading
            },
        );
        // Phase 15A2: with FXAA on, tone-map into the LDR intermediate and let
        // FXAA resolve it to the swapchain. Editor overlays draw afterwards, so
        // gizmos and UI text stay pixel-sharp.
        // Phase 24F: TAA supersedes FXAA. Running both means edges get
        // processed twice — TAA resolves them from accumulated samples, then
        // FXAA blurs along whatever gradients remain, dragging dark pixels into
        // bright neighbours. That contributed to the rim around silhouettes,
        // and there is nothing left for FXAA to usefully do once TAA is on.
        self.profiler.end(&mut encoder);
        self.profiler.begin(&mut encoder, "Post + present");
        // MORROWIND-AC. `AntiAliasing` is one authored value, so these are
        // consequences rather than a precedence chain over three booleans. FSR
        // still wins when it is *effective* — the pass declines itself on a
        // device without the features, and the authored value must not override
        // a decline — which is the one piece of precedence that survives.
        let smaa_active = self.smaa_pass.active() && !fsr_ok;
        let fxaa_active = self.fxaa_enabled && !self.taa_pass.enabled() && !fsr_ok;
        // Phase 24AC: when CAS is running it owns the swapchain, and whatever
        // would have written there writes into its input instead. Placed here
        // rather than at the very end of the frame on purpose — the gizmos, the
        // outline and the UI draw into the surface *after* this, and sharpening
        // a 1-pixel gizmo line or a font glyph would only ring it.
        // FSR RCAS already sharpened the HDR reconstruct; stacking CAS rings.
        let cas_active = self.cas_pass.active() && !fsr_ok;
        // MORROWIND-J step 3. A view with a rectangle always goes through the
        // present blit, whatever the resolution: the blit is the only thing in
        // the chain that can land the image somewhere other than the whole
        // surface, and it is what stops the second view of a four-up frame
        // clearing the first.
        let tiled = view.rect.is_some();
        let upscale = tiled
            || (!fsr_ok
                && (self.render_width != ctx.config.width
                    || self.render_height != ctx.config.height));
        {
            // Scene-sized colour target. Native writes this straight to the
            // swapchain; a lower preset lands here and is blitted after CAS.
            // FSR already wrote display-sized HDR, so tone map goes to the
            // window (or CAS/FXAA at window size) with no extra blit.
            let scene_present: &wgpu::TextureView = if upscale {
                self.present_pass.src_view()
            } else {
                &surface_view
            };
            let ldr_target: &wgpu::TextureView = if cas_active {
                self.cas_pass.input_view()
            } else {
                scene_present
            };
            if smaa_active {
                self.profiler.begin(&mut encoder, "SMAA");
                self.smaa_pass
                    .update(&ctx.queue, self.ldr_width, self.ldr_height);
                self.postprocess_pass
                    .record(&mut encoder, &self.smaa_pass.ldr_view);
                self.smaa_pass.record(&mut encoder, ldr_target);
                self.profiler.end(&mut encoder);
            } else if fxaa_active {
                self.fxaa_pass
                    .update(&ctx.queue, self.ldr_width, self.ldr_height);
                self.postprocess_pass
                    .record(&mut encoder, &self.fxaa_pass.ldr_view);
                self.fxaa_pass.record(&mut encoder, ldr_target);
            } else {
                self.postprocess_pass.record(&mut encoder, ldr_target);
            }
            if cas_active {
                self.profiler.begin(&mut encoder, "CAS");
                self.cas_pass
                    .record(&ctx.queue, &mut encoder, scene_present);
                self.profiler.end(&mut encoder);
            }
        }
        if upscale {
            self.present_pass
                .record_into(&mut encoder, &surface_view, view.rect);
        }

        // DOOM-A: `Post + present` closes here. Everything below is editor
        // chrome, and chrome that is billed to the post chain is chrome nobody
        // ever looks at — the whole reason to separate them is that the scene
        // budget and the editor budget are answerable to different questions.
        self.profiler.end(&mut encoder);

        capture_now
    }

    /// Empty every per-frame submission queue.
    ///
    /// Must run on *every* path out of `render`, including the ones that bail
    /// before drawing.
    fn clear_frame_queues(&mut self) {
        self.draw_queue.clear();
        self.shadow_only_queue.clear();
        self.water_queue.clear();
        self.terrain_queue.clear();
        self.transparent_queue.clear();
        self.light_gizmo_queue.clear();
        self.line_gizmo_queue.clear();
    }

    /// Compact revision of every input that changes a reference path sample.
    ///
    /// This intentionally runs only while a traced 2-D lighting estimator is
    /// active. Hashing transforms for a foliage-heavy frame is negligible next
    /// to ray tracing but needless overhead for the normal raster renderer.
    fn traced_scene_revision(&self) -> u64 {
        fn mix(hash: &mut u64, value: u64) {
            *hash ^= value.wrapping_add(0x9e37_79b9_7f4a_7c15);
            *hash = hash.rotate_left(27).wrapping_mul(0x94d0_49bb_1331_11eb);
        }

        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        mix(&mut hash, self.materials_pool.revision());
        for value in self
            .light_direction
            .to_array()
            .into_iter()
            .chain(self.light_color.to_array())
        {
            mix(&mut hash, u64::from(value.to_bits()));
        }
        mix(&mut hash, u64::from(self.moon_intensity.to_bits()));
        for light in &self.local_lights {
            for chunk in bytemuck::bytes_of(light).chunks_exact(8) {
                mix(&mut hash, u64::from_ne_bytes(chunk.try_into().unwrap()));
            }
        }
        for cmd in &self.draw_queue {
            mix(&mut hash, u64::from(cmd.vertex_offset));
            mix(&mut hash, u64::from(cmd.index_offset));
            mix(&mut hash, u64::from(cmd.index_count));
            mix(&mut hash, u64::from(cmd.material_id));
            for value in cmd.transform.to_cols_array() {
                mix(&mut hash, u64::from(value.to_bits()));
            }
        }
        for terrain in &self.terrains {
            mix(&mut hash, terrain.edit_revision);
            mix(&mut hash, u64::from(terrain.wetness.to_bits()));
            mix(&mut hash, u64::from(terrain.parallax_scale.to_bits()));
            mix(&mut hash, u64::from(terrain.hex_tiling));
            mix(&mut hash, u64::from(terrain.height_blend));
        }
        hash
    }

    /// Revision of geometry/material inputs that require rebuilding the SDF.
    /// Lighting is deliberately absent: DDGI handles light changes in probe
    /// history, while rebuilding 64³ geometry for a moving sun is pure waste.
    /// The SDF pass waits for a changed revision to settle for one frame, so a
    /// continuous animation does not force a 64³ CPU rebuild every frame.
    fn sdf_scene_revision(&self) -> u64 {
        fn mix(hash: &mut u64, value: u64) {
            *hash ^= value.wrapping_add(0x9e37_79b9_7f4a_7c15);
            *hash = hash.rotate_left(27).wrapping_mul(0x94d0_49bb_1331_11eb);
        }
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        mix(&mut hash, self.shadow_caster_content_revision);
        mix(&mut hash, self.materials_pool.revision());
        for cmd in &self.draw_queue {
            mix(&mut hash, u64::from(cmd.vertex_offset));
            mix(&mut hash, u64::from(cmd.index_offset));
            mix(&mut hash, u64::from(cmd.index_count));
            mix(&mut hash, u64::from(cmd.material_id));
            for value in cmd.transform.to_cols_array() {
                mix(&mut hash, u64::from(value.to_bits()));
            }
        }
        hash
    }

    /// Which draws are worth rendering into the shadow atlas this frame.
    ///
    /// # Why this exists
    ///
    /// The shadow pass issued every draw in the queue, four times, once per
    /// cascade. With foliage painted across a hillside that is 8 599 draws and
    /// 52.9 million triangles per cascade, and the profiler put it at **24.5 ms
    /// of a 42 ms frame** — more than half the frame spent drawing depth for
    /// grass whose shadow is a sub-pixel speckle. The main view already stops
    /// drawing distant foliage (17G); the shadow pass never learned to.
    ///
    /// # The test
    ///
    /// Unreal's `r.Shadow.RadiusThreshold`, from `ShadowSetup.cpp`:
    ///
    /// ```text
    /// draw = radius² > threshold² · distance²
    /// ```
    ///
    /// which is `radius / distance > threshold` — the caster's **projected
    /// screen radius**. Two things about it are worth stating because they are
    /// easy to get wrong:
    ///
    /// - The distance is from the **camera**, not from the light. The question
    ///   is not "is this near the sun" but "would anyone see the shadow it
    ///   casts", and that is a screen-space question.
    /// - It is a size test, not a distance cut. A tree keeps casting at 200 m
    ///   because its radius is metres; a grass tuft stops at 30 m because its
    ///   radius is centimetres. One rule, and it scales itself to the object —
    ///   which is why UE uses it in place of a per-asset shadow distance.
    ///
    /// UE applies it to whole-scene (CSM) shadows only and skips it for virtual
    /// shadow maps, which need the draw for GPU-side caching. Somnium follows
    /// that split now: the measured CSM default keeps the threshold, while an
    /// effective VSM light retains every caster for its demanded pages (and
    /// therefore for the coarse CSM miss fallback rendered in the same frame).
    ///
    /// Phase CR-E: a caster that fails the **camera** frustum can still shadow
    /// into view. Those live in `shadow_only_queue` with instance indices after
    /// the vis draws. This method cascade-frustum-culls both lists; it never
    /// camera-culls a caster.
    /// Build the software-sparse page plan from receivers that survived the
    /// main-view submission.  This is screen-space demand at draw granularity:
    /// off-screen objects never request pages, and very large queues are
    /// deterministically reduced to 4k receiver samples before neighbourhood
    /// expansion. A later depth-reduction compute pass can feed the same cache
    /// at pixel-tile granularity without changing its allocation contract.
    fn prepare_virtual_shadow_cache(
        &mut self,
        queue: &wgpu::Queue,
        cascades: &[crate::shadow::cascade::CascadeData; crate::shadow::NUM_CASCADES],
    ) {
        use crate::shadow::virtual_map::{DirectionalClipmap, ShadowTechnique};

        self.virtual_shadow_work.clear();
        self.profiler.counters.virtual_shadow_pages = 0;
        self.profiler.counters.virtual_shadow_resident = 0;
        if self.directional_shadow_policy.technique != ShadowTechnique::Virtual
            || self.virtual_shadow_gpu.is_none()
        {
            if let Some(gpu) = &self.virtual_shadow_gpu {
                gpu.set_enabled(queue, false, self.directional_shadow_policy.csm_fallback);
            }
            return;
        }

        let pages_per_axis = self.virtual_shadow_cache.config().pages_per_axis();
        let clipmaps =
            std::array::from_fn::<_, { crate::shadow::NUM_CASCADES }, _>(|i| DirectionalClipmap {
                view_proj: cascades[i].view_proj,
                split_depth: cascades[i].split_depth,
                pages_per_axis,
            });
        let light_revision = virtual_shadow_light_revision(self.light_direction, cascades);
        let caster_revision = virtual_shadow_caster_revision(
            self.draw_queue.iter().chain(self.shadow_only_queue.iter()),
            self.shadow_caster_content_revision,
        );
        self.virtual_shadow_cache
            .begin_frame(light_revision, caster_revision);

        let stride = (self.draw_queue.len() / 4_096).max(1);
        for command in self.draw_queue.iter().step_by(stride) {
            // Demand is receiver-driven. A mesh that does not cast can still
            // receive a shadow, so `casts_shadow` must not suppress its page.
            let world = command.transform.transform_point3(glam::Vec3::ZERO);
            let view_depth = -(self.view_matrix * world.extend(1.0)).z;
            if let Some(page) = self.virtual_shadow_cache.request_screen_sample(
                self.directional_shadow_policy.light_id,
                world,
                view_depth.max(0.0),
                &clipmaps,
            ) {
                self.virtual_shadow_cache.request_neighbourhood(page, 1);
            }
        }
        // Water shades in its forward pass, so it is not represented in the
        // opaque draw queue above. Demand its receiver pages explicitly or a
        // lake would always take the CSM fallback while its shore used VSM.
        for (_, transform, _, _, _, _) in &self.water_queue {
            let world = transform.transform_point3(glam::Vec3::ZERO);
            let view_depth = -(self.view_matrix * world.extend(1.0)).z;
            if let Some(page) = self.virtual_shadow_cache.request_screen_sample(
                self.directional_shadow_policy.light_id,
                world,
                view_depth.max(0.0),
                &clipmaps,
            ) {
                self.virtual_shadow_cache.request_neighbourhood(page, 1);
            }
        }
        self.virtual_shadow_work = self.virtual_shadow_cache.resolve(&clipmaps);
        self.profiler.counters.virtual_shadow_pages =
            u32::try_from(self.virtual_shadow_work.len()).unwrap_or(u32::MAX);
        self.profiler.counters.virtual_shadow_resident = self.virtual_shadow_cache.stats().resident;
        if let Some(gpu) = &self.virtual_shadow_gpu {
            gpu.upload_page_table(
                queue,
                &self.virtual_shadow_cache,
                self.directional_shadow_policy.light_id,
            );
            gpu.set_enabled(queue, true, self.directional_shadow_policy.csm_fallback);
        }
    }

    fn rebuild_shadow_casters(&mut self) {
        let threshold = match self.directional_shadow_technique() {
            crate::shadow::virtual_map::ShadowTechnique::Cascaded => self.shadow_radius_threshold,
            crate::shadow::virtual_map::ShadowTechnique::Virtual => 0.0,
        };
        let cascade_cull = self.cascade_caster_cull && !cascade_cull_env_off();
        let cascade_planes: [_; crate::shadow::NUM_CASCADES] =
            self.cascade_view_projs.map(crate::culling::frustum_planes);
        self.shadow_caster_scratch.clear();
        // A material edit can change alpha-tested depth without changing a
        // command's material id. Conservatively invalidate every cascade on a
        // pool revision; command transforms/ranges below remain per-cascade.
        let global_shadow_revision =
            self.shadow_caster_content_revision ^ self.materials_pool.revision().rotate_left(17);
        self.cascade_shadow_revisions =
            [0xcbf2_9ce4_8422_2325u64 ^ global_shadow_revision; crate::shadow::NUM_CASCADES];
        let frame_layout =
            frame_instance_layout(self.draw_queue.len(), self.shadow_only_queue.len());
        for (i, cmd) in self.draw_queue.iter().enumerate() {
            consider_shadow_caster(
                cmd,
                u32::try_from(i).unwrap_or(0),
                threshold,
                cascade_cull,
                &cascade_planes,
                self.camera_pos,
                &self.geometry,
                &mut self.shadow_caster_scratch,
                &mut self.cascade_shadow_revisions,
            );
        }
        for (i, cmd) in self.shadow_only_queue.iter().enumerate() {
            consider_shadow_caster(
                cmd,
                frame_layout
                    .shadow_only_base
                    .saturating_add(u32::try_from(i).unwrap_or(u32::MAX)),
                threshold,
                cascade_cull,
                &cascade_planes,
                self.camera_pos,
                &self.geometry,
                &mut self.shadow_caster_scratch,
                &mut self.cascade_shadow_revisions,
            );
        }
    }
}

fn virtual_shadow_light_revision(
    direction: glam::Vec3,
    cascades: &[crate::shadow::cascade::CascadeData; crate::shadow::NUM_CASCADES],
) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut mix = |word: u32| {
        hash ^= u64::from(word);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for value in direction.to_array() {
        mix(value.to_bits());
    }
    for cascade in cascades {
        for value in cascade.view_proj.to_cols_array() {
            mix(value.to_bits());
        }
        mix(cascade.split_depth.to_bits());
    }
    hash
}

fn virtual_shadow_caster_revision<'a>(
    commands: impl IntoIterator<Item = &'a DrawCommand>,
    content_revision: u64,
) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64 ^ content_revision;
    let mut mix = |word: u32| {
        hash ^= u64::from(word);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for command in commands {
        mix(command.vertex_offset);
        mix(command.index_offset);
        mix(command.index_count);
        mix(command.material_id);
        mix(u32::from(command.casts_shadow));
        for value in command.transform.to_cols_array() {
            mix(value.to_bits());
        }
    }
    hash
}

/// Frozen per-frame instance-buffer partition.
///
/// Visibility draws come first, followed by camera-culled shadow casters and
/// finally blended draws. Centralising the two bases prevents one consumer
/// from silently drifting away from the upload order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FrameInstanceLayout {
    shadow_only_base: u32,
    transparent_base: u32,
}

fn frame_instance_layout(visible: usize, shadow_only: usize) -> FrameInstanceLayout {
    let shadow_only_base = u32::try_from(visible).expect("visible instance count exceeds u32");
    let transparent_base = u32::try_from(
        visible
            .checked_add(shadow_only)
            .expect("frame instance count overflow"),
    )
    .expect("opaque instance count exceeds u32");
    FrameInstanceLayout {
        shadow_only_base,
        transparent_base,
    }
}

fn cpu_frustum_env_off() -> bool {
    std::env::var("SOMNIUM_CPU_FRUSTUM").as_deref() == Ok("0")
}

fn cascade_cull_env_off() -> bool {
    std::env::var("SOMNIUM_CASCADE_CULL").as_deref() == Ok("0")
}

/// Frames counted by the clipmap trace, so a log line can be placed in time.
static CLIPMAP_TRACE_FRAME: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `SOMNIUM_CLIPMAP_TRACE=1` — one `DF-TRACE` line per generating frame.
///
/// The clipmap's failures are all invisible in a picture: a stack where only
/// the coarsest ring is ever ready renders as a plausible blur, and debug view
/// 34 calls it "a detail ring" because it is one. The ready mask, the queued
/// texel count and the source-page counters are the three numbers that say
/// which, and none of them had a way out of the renderer.
fn clipmap_trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| matches!(std::env::var("SOMNIUM_CLIPMAP_TRACE").as_deref(), Ok("1")))
}

/// PORTAL-0-D: `packed` is this frame's `terrain_lod_by_vertex`, built by the
/// terrain loop. A non-terrain draw is simply absent from it, which is the same
/// answer the old scan gave by falling off the end of every chunk list — but in
/// one hash lookup instead of one pass over every chunk of every terrain.
fn gpu_instance_from_cmd(
    packed: &std::collections::HashMap<u32, u32>,
    cmd: &DrawCommand,
    shadow_debug: f32,
) -> crate::instance::GpuInstanceData {
    let terrain_packed = packed.get(&cmd.vertex_offset).copied();
    let terrain_lod = if shadow_debug > 12.5 && shadow_debug < 13.5 {
        terrain_packed.map(|p| (p & 15) + 1).unwrap_or(0)
    } else {
        terrain_packed.unwrap_or(0)
    };
    crate::instance::GpuInstanceData {
        model_matrix: cmd.transform.to_cols_array_2d(),
        material_id: cmd.material_id,
        mesh_vertex_offset: cmd.vertex_offset,
        mesh_index_offset: cmd.index_offset,
        _padding: terrain_lod,
    }
}

fn consider_shadow_caster(
    cmd: &DrawCommand,
    instance_index: u32,
    threshold: f32,
    cascade_cull: bool,
    cascade_planes: &[[[f32; 4]; 6]],
    camera_pos: glam::Vec3,
    geometry: &crate::geometry::GeometryPool,
    out: &mut Vec<crate::pass::shadow::ShadowCaster>,
    cascade_revisions: &mut [u64; crate::shadow::NUM_CASCADES],
) {
    if !cmd.casts_shadow {
        return;
    }
    let caster = crate::pass::shadow::ShadowCaster {
        instance_index,
        index_count: cmd.index_count,
    };
    let Some((min, max)) = geometry.mesh_aabb(cmd.vertex_offset) else {
        out.push(caster);
        for revision in cascade_revisions {
            mix_shadow_caster_revision(revision, cmd);
        }
        return;
    };
    let min = glam::Vec3::from(min);
    let max = glam::Vec3::from(max);
    let (wmin, wmax) = crate::culling::transform_aabb(cmd.transform, min, max);
    let coverage: [bool; crate::shadow::NUM_CASCADES] = std::array::from_fn(|i| {
        !cascade_cull || crate::culling::aabb_in_frustum(&cascade_planes[i], wmin, wmax)
    });
    if !coverage.into_iter().any(|inside| inside) {
        return;
    }
    if threshold > 0.0 {
        let half = cmd.transform.transform_vector3((max - min) * 0.5);
        let radius = half.length();
        let centre = cmd.transform.transform_point3((min + max) * 0.5);
        let dist_sq = (centre - camera_pos).length_squared();
        if !crate::pass::shadow::casts_shadow(radius, dist_sq, threshold) {
            return;
        }
    }
    out.push(caster);
    for (i, inside) in coverage.into_iter().enumerate() {
        if inside {
            mix_shadow_caster_revision(&mut cascade_revisions[i], cmd);
        }
    }
}

fn mix_shadow_caster_revision(hash: &mut u64, command: &DrawCommand) {
    // Queue order is not shadow content. Foliage and off-camera terrain may be
    // collected through maps whose iteration order changes while the set does
    // not; folding each independent fingerprint with wrapping addition keeps
    // the revision commutative while still counting duplicate draws.
    let mut fingerprint = 0xcbf2_9ce4_8422_2325u64;
    let mut mix = |word: u32| {
        fingerprint ^= u64::from(word);
        fingerprint = fingerprint.wrapping_mul(0x0000_0100_0000_01b3);
    };
    mix(command.vertex_offset);
    mix(command.index_offset);
    mix(command.index_count);
    mix(command.material_id);
    for value in command.transform.to_cols_array() {
        mix(value.to_bits());
    }
    *hash = hash.wrapping_add(fingerprint);
}

/// Build a full mip chain by repeated 2×2 box filtering.
///
/// Returns `(width, height, rgba8)` per level, starting with the original.
/// Done on the CPU because the imported data is already in system memory and
/// this runs once per texture at import; a GPU blit chain would be faster but
/// needs a render pass per level.
///
/// Colour is averaged **weighted by alpha**. A plain box filter is wrong for
/// cutout atlases: foliage diffuse maps carry blade colour only where the mask
/// is opaque and leave the rest black, so an unweighted average drags that
/// black into the blades and grass turns darker — and bluer, once ambient sky
/// is the brightest thing left — the further away it gets. Weighting by alpha
/// makes the transparent background contribute nothing, which is what the
/// artist's authored colour means. For a fully opaque texture every weight is
/// equal and this reduces exactly to the plain box filter.
///
/// Alpha itself is then rescaled per level to preserve *coverage* — the
/// fraction of texels that survive alpha testing. Averaging a binary mask
/// pushes alpha toward its mean, so a fixed 0.5 cutoff eats thin geometry as
/// it recedes and foliage visibly thins out with distance.
fn build_mip_chain(data: &[u8], width: u32, height: u32) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = vec![(width, height, data.to_vec())];
    let (mut w, mut h) = (width, height);

    while w > 1 || h > 1 {
        let (pw, ph, prev) = levels.last().unwrap();
        let (pw, ph) = (*pw, *ph);
        let nw = (pw / 2).max(1);
        let nh = (ph / 2).max(1);
        let mut next = vec![0u8; (nw * nh * 4) as usize];

        for y in 0..nh {
            for x in 0..nw {
                // Source 2x2 block, clamped for odd dimensions.
                let x0 = (x * 2).min(pw - 1);
                let x1 = (x * 2 + 1).min(pw - 1);
                let y0 = (y * 2).min(ph - 1);
                let y1 = (y * 2 + 1).min(ph - 1);
                let texel = |tx: u32, ty: u32, c: usize| -> u32 {
                    prev[((ty * pw + tx) * 4) as usize + c] as u32
                };
                let block = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)];

                let alpha_sum: u32 = block.iter().map(|&(tx, ty)| texel(tx, ty, 3)).sum();
                let out = ((y * nw + x) * 4) as usize;
                for c in 0..3 {
                    // Fully transparent block: no authored colour to preserve,
                    // so fall back to the unweighted mean rather than emitting
                    // black, which would still bleed once alpha is rescaled.
                    next[out + c] = if alpha_sum == 0 {
                        (block.iter().map(|&(tx, ty)| texel(tx, ty, c)).sum::<u32>() / 4) as u8
                    } else {
                        let weighted: u32 = block
                            .iter()
                            .map(|&(tx, ty)| texel(tx, ty, c) * texel(tx, ty, 3))
                            .sum();
                        (weighted / alpha_sum) as u8
                    };
                }
                next[out + 3] = (alpha_sum / 4) as u8;
            }
        }

        levels.push((nw, nh, next));
        w = nw;
        h = nh;
    }

    preserve_alpha_coverage(&mut levels);
    levels
}

/// Alpha cutoff that [`preserve_alpha_coverage`] matches coverage against.
///
/// Mirrors the glTF default `alphaCutoff` and the value the loader assigns to
/// sidecar-masked materials.
const ALPHA_TEST_CUTOFF: u8 = 128;

/// Rescale each mip's alpha so the same fraction of texels passes alpha testing
/// as at mip 0 (Castaño's alpha-test mipmap scaling).
///
/// Left alone, downsampling a binary mask averages it toward grey, so fewer and
/// fewer texels clear the cutoff and masked geometry erodes with distance —
/// grass thins to nothing, tree canopies go patchy and then bald.
///
/// A no-op for textures that are opaque everywhere: coverage is already 1.0 at
/// every level, so the search settles on a scale of 1 and nothing moves.
fn preserve_alpha_coverage(levels: &mut [(u32, u32, Vec<u8>)]) {
    /// Fraction of texels at or above the cutoff once alpha is scaled.
    fn coverage(data: &[u8], scale: f32) -> f32 {
        let passing = data
            .chunks_exact(4)
            .filter(|t| (f32::from(t[3]) * scale) >= f32::from(ALPHA_TEST_CUTOFF))
            .count();
        passing as f32 / (data.len() / 4) as f32
    }

    let Some((_, _, base)) = levels.first() else {
        return;
    };
    let target = coverage(base, 1.0);
    // Nothing masked out (or nothing left): no coverage to defend.
    if target >= 1.0 || target <= 0.0 {
        return;
    }

    for (_, _, data) in levels.iter_mut().skip(1) {
        // Bisect on the scale factor. Coverage is monotonic in scale but a step
        // function, so solve numerically rather than in closed form; a handful
        // of iterations lands well inside one 8-bit quantisation step.
        let (mut lo, mut hi) = (0.0f32, 4.0f32);
        for _ in 0..12 {
            let mid = 0.5 * (lo + hi);
            if coverage(data, mid) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let scale = 0.5 * (lo + hi);
        for texel in data.chunks_exact_mut(4) {
            texel[3] = (f32::from(texel[3]) * scale).round().clamp(0.0, 255.0) as u8;
        }
    }
}

#[cfg(test)]
mod mip_tests {
    use super::{ALPHA_TEST_CUTOFF, build_mip_chain};

    /// A 2x2 cutout block: one opaque green texel, three transparent black.
    /// Unweighted averaging would give a quarter-strength muddy green; weighting
    /// by alpha has to hand back the authored green untouched.
    #[test]
    fn transparent_black_does_not_darken_the_visible_colour() {
        let mut data = vec![0u8; 2 * 2 * 4];
        data[0..4].copy_from_slice(&[80, 120, 40, 255]);
        let levels = build_mip_chain(&data, 2, 2);
        let (_, _, mip1) = &levels[1];
        assert_eq!(
            &mip1[0..3],
            &[80, 120, 40],
            "alpha-weighted average let the transparent background bleed in",
        );
    }

    /// Downsampling a binary mask averages alpha toward grey, which without
    /// rescaling drops texels below the cutoff and erodes the shape. The
    /// property that matters is that coverage never *shrinks*.
    ///
    /// It can overshoot, and at the smallest levels it must: once a mip is
    /// uniform, every texel passes or none do, so there is no scale that lands
    /// on a fraction. Rounding up keeps distant grass visible; rounding down
    /// would make it disappear, which is the failure this rescaling exists to
    /// prevent.
    #[test]
    fn coverage_never_erodes_down_the_chain() {
        // 4x4 checkerboard: half the texels opaque, half fully transparent.
        let mut data = vec![0u8; 4 * 4 * 4];
        for (i, texel) in data.chunks_exact_mut(4).enumerate() {
            texel[0..3].copy_from_slice(&[200, 200, 200]);
            texel[3] = if i % 2 == 0 { 255 } else { 0 };
        }
        let base_cov = 0.5;
        for (w, h, level) in build_mip_chain(&data, 4, 4) {
            let passing = level
                .chunks_exact(4)
                .filter(|t| t[3] >= ALPHA_TEST_CUTOFF)
                .count() as f32;
            let cov = passing / (w * h) as f32;
            assert!(
                cov >= base_cov,
                "mip {w}x{h} coverage {cov} eroded below {base_cov}",
            );
        }
    }

    /// Without rescaling the same checkerboard would vanish: the 2x2 level
    /// averages to 127, one step under the cutoff, and nothing survives.
    /// This pins the regression that rescaling exists to prevent.
    #[test]
    fn an_unrescaled_mask_would_have_vanished() {
        let mut data = vec![0u8; 4 * 4 * 4];
        for (i, texel) in data.chunks_exact_mut(4).enumerate() {
            texel[3] = if i % 2 == 0 { 255 } else { 0 };
        }
        let levels = build_mip_chain(&data, 4, 4);
        let (_, _, mip1) = &levels[1];
        assert!(
            mip1.chunks_exact(4).all(|t| t[3] >= ALPHA_TEST_CUTOFF),
            "half-covered mask fell under the cutoff after downsampling",
        );
    }

    /// The opaque case must be untouched: no alpha weighting artefacts, no
    /// coverage rescaling, identical to the plain box filter it replaced.
    #[test]
    fn fully_opaque_textures_keep_full_alpha() {
        let mut data = vec![0u8; 8 * 8 * 4];
        for (i, texel) in data.chunks_exact_mut(4).enumerate() {
            texel[0] = i as u8;
            texel[3] = 255;
        }
        for (_, _, level) in build_mip_chain(&data, 8, 8) {
            assert!(
                level.chunks_exact(4).all(|t| t[3] == 255),
                "coverage rescaling touched an opaque texture",
            );
        }
    }

    #[test]
    fn chain_halves_down_to_one_by_one() {
        let data = vec![255u8; 8 * 8 * 4];
        let levels = build_mip_chain(&data, 8, 8);
        let dims: Vec<(u32, u32)> = levels.iter().map(|(w, h, _)| (*w, *h)).collect();
        assert_eq!(dims, vec![(8, 8), (4, 4), (2, 2), (1, 1)]);
    }

    #[test]
    fn a_flat_colour_survives_downsampling() {
        let data = vec![100u8; 4 * 4 * 4];
        for (_, _, level) in build_mip_chain(&data, 4, 4) {
            assert!(
                level.iter().all(|&v| v == 100),
                "box filter shifted a flat colour"
            );
        }
    }

    #[test]
    fn non_square_and_odd_sizes_terminate() {
        // Odd dimensions must not loop forever or index out of bounds.
        let data = vec![7u8; 5 * 3 * 4];
        let levels = build_mip_chain(&data, 5, 3);
        assert_eq!(levels.last().unwrap().0, 1);
        assert_eq!(levels.last().unwrap().1, 1);
    }

    #[test]
    fn each_level_has_the_expected_byte_count() {
        let data = vec![0u8; 16 * 16 * 4];
        for (w, h, level) in build_mip_chain(&data, 16, 16) {
            assert_eq!(level.len(), (w * h * 4) as usize);
        }
    }
}

#[cfg(test)]
mod frame_instance_layout_tests {
    use super::{frame_instance_layout, gpu_instance_from_cmd, mix_shadow_caster_revision};
    use crate::command::{DrawCommand, SortKey};

    fn draw(vertex: u32, index: u32, material: u32, tx: f32) -> DrawCommand {
        DrawCommand {
            sort_key: SortKey::new(0, material as u16, vertex),
            vertex_offset: vertex,
            index_offset: index,
            index_count: 3,
            material_id: material,
            transform: glam::Mat4::from_translation(glam::Vec3::new(tx, 0.0, 0.0)),
            casts_shadow: true,
        }
    }

    #[test]
    fn visible_shadow_only_and_transparent_ranges_do_not_overlap() {
        let layout = frame_instance_layout(2, 1);
        assert_eq!(layout.shadow_only_base, 2);
        assert_eq!(layout.transparent_base, 3);
    }

    #[test]
    fn empty_middle_range_keeps_transparent_base_at_visible_end() {
        let layout = frame_instance_layout(7, 0);
        assert_eq!(layout.shadow_only_base, 7);
        assert_eq!(layout.transparent_base, 7);
    }

    #[test]
    fn shadow_content_revision_is_order_independent_but_transform_sensitive() {
        let a = draw(11, 101, 1, 10.0);
        let b = draw(22, 202, 2, 20.0);
        let mut ab = 7;
        mix_shadow_caster_revision(&mut ab, &a);
        mix_shadow_caster_revision(&mut ab, &b);
        let mut ba = 7;
        mix_shadow_caster_revision(&mut ba, &b);
        mix_shadow_caster_revision(&mut ba, &a);
        assert_eq!(ab, ba, "collection order is not shadow content");

        let moved = draw(22, 202, 2, 21.0);
        let mut changed = 7;
        mix_shadow_caster_revision(&mut changed, &a);
        mix_shadow_caster_revision(&mut changed, &moved);
        assert_ne!(ab, changed, "moving a caster must invalidate its depth");
    }

    #[test]
    fn final_consumer_indices_recover_each_draws_ids_on_both_visibility_paths() {
        let visible = draw(11, 101, 1, 10.0);
        let shadow_only = draw(22, 202, 2, 20.0);
        let transparent = draw(33, 303, 3, 30.0);
        let uploaded = [&visible, &shadow_only, &transparent]
            .map(|cmd| gpu_instance_from_cmd(&std::collections::HashMap::new(), cmd, 0.0));
        let layout = frame_instance_layout(1, 1);

        for gpu_driven in [false, true] {
            // GPU indirect and CPU fallback differ in how the draw is issued,
            // but both must deliver these same instance indices to the vertex
            // shader. Cluster reordering is safe only because it carries 0 as
            // first_instance rather than using its dispatch position.
            let visible_first_instance = if gpu_driven { 0 } else { 0 };
            let consumer_indices = [
                visible_first_instance,
                layout.shadow_only_base,
                layout.transparent_base,
            ];
            for (slot, expected) in consumer_indices.into_iter().zip([
                (11, 101, 1, 10.0),
                (22, 202, 2, 20.0),
                (33, 303, 3, 30.0),
            ]) {
                let instance = uploaded[slot as usize];
                assert_eq!(instance.mesh_vertex_offset, expected.0);
                assert_eq!(instance.mesh_index_offset, expected.1);
                assert_eq!(instance.material_id, expected.2);
                assert_eq!(instance.model_matrix[3][0], expected.3);
            }
        }
    }
}
