/// Mesh primitive or light kinds available in the Create menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateKind {
    Cube,
    Sphere,
    Plane,
    Cylinder,
    DirectionalLight,
    PointLight,
    SpotLight,
    RectLight,
    DiscLight,
    TubeLight,
    Particle,
    Terrain,
    VoxelTerrain,
}

impl CreateKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cube => "Cube",
            Self::Sphere => "Sphere",
            Self::Plane => "Plane",
            Self::Cylinder => "Cylinder",
            Self::DirectionalLight => "Directional Light",
            Self::PointLight => "Point Light",
            Self::SpotLight => "Spot Light",
            Self::RectLight => "Area Light",
            Self::DiscLight => "Disc Light",
            Self::TubeLight => "Tube Light",
            Self::Particle => "Particle Emitter",
            Self::Terrain => "Terrain",
            Self::VoxelTerrain => "Voxel Terrain",
        }
    }
}

/// Which TRS component a NumericField targets (for SetInspectorValue).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorField {
    PosX,
    PosY,
    PosZ,
    RotX,
    RotY,
    RotZ,
    ScaleX,
    ScaleY,
    ScaleZ,
    // Light properties (Phase 13E) — only meaningful when the selected entity
    // has a `LightComponent`. Angles are edited in degrees.
    LightIntensity,
    LightRange,
    LightInnerAngle,
    LightOuterAngle,
    // Light colour (Phase 22C). Linear RGB, edited per channel — the sun's
    // colour is the main lever on the mood of a scene and was not reachable
    // from the editor at all.
    LightColorR,
    LightColorG,
    LightColorB,
    /// Colour temperature in Kelvin (Phase 24E). Drives the light's hue.
    LightColorTemperature,
    /// Directional moonlight illuminance in lux (Phase 25M-2).
    LightMoonIntensity,
    LightSourceRadius,
    LightAreaWidth,
    LightAreaHeight,
    // Post-processing (Phase 15A1) — only for entities with a
    // `PostProcessComponent`.
    /// Manual exposure value at ISO 100 (Phase 24A). Only used when auto
    /// exposure is off.
    PostExposure,
    /// Stops added on top of the metered exposure. Negative darkens — this is
    /// the control for "auto-exposure is right but I want it a stop down".
    PostExposureCompensation,
    /// Bloom strength (Phase 24T).
    PostBloomIntensity,
    /// Focus distance in metres (Phase 24Z).
    PostFocusDistance,
    /// Colour grading (Phase 24Y).
    PostTemperature,
    /// Green/magenta axis, the other half of a white balance.
    PostTint,
    PostContrast,
    PostSaturation,
    /// Lift/gamma/gain (Phase 24Y): shadows, midtones and highlights. These
    /// are the three handles a colourist actually reaches for, and until now
    /// they existed in the component with no way to touch them.
    PostLift,
    PostGamma,
    PostGain,
    /// Film grain (Phase 24Z).
    PostGrain,
    PostVignetteStrength,
    PostCaStrength,
    /// Scene-wide indirect-light strength (Phase 22C).
    PostIblIntensity,
    /// Fog extinction per metre (Phase 24U). 0 leaves aerial perspective only.
    PostFogDensity,
    /// Metres over which fog density falls to 1/e, so it pools in valleys.
    PostFogHeight,
    /// Henyey-Greenstein asymmetry; positive scatters forward toward the sun.
    PostFogAsymmetry,
    /// CAS (Phase 24AC): 0 = least ringing, 1 = maximum.
    PostCasSharpness,
    /// How far the sharpened image is blended in. 0 is off.
    PostCasStrength,
    /// Shutter fraction for motion blur (Phase 24Z). 0.5 is a 180 degree
    /// shutter, the film default.
    PostMotionBlurShutter,
    /// Strength of the traced indirect diffuse (Phase 24L). Every other effect
    /// has an amount dial; this one was the odd toggle out.
    PostGiIntensity,
    PostCacheIntensity,
    PostCacheCell,
    PostSpecRough,
    PostPathBounces,
    PostProbeIntensity,
    PostShaftIntensity,
    /// Physical camera (Phase 24A). Only meaningful with
    /// [`PostFxToggle::PhysicalCamera`] on; they also set the DoF blur, which
    /// is why aperture matters even when exposure is manual.
    PostAperture,
    /// Shutter speed as its denominator: 100 means 1/100 s.
    PostShutter,
    PostIso,
    /// GTAO (Phase 24I): sampling radius in metres, and how hard the occlusion
    /// is applied. Radius is the one that decides whether AO reads as contact
    /// darkening or as a broad dirty smear.
    PostAoRadius,
    PostAoIntensity,
    /// FSR RCAS sharpness, 0..=1.
    PostFsrSharpness,
    // Terrain layers (Phase 17C) — only for entities with a `TerrainComponent`.
    /// Which splat layer the paint brush writes, 0..=31.
    TerrainPaintLayer,
    /// World-space tiling of the currently selected paint layer.
    TerrainTile0,
    /// Phase 25H: multiplies every layer's authored relief depth. 0 switches
    /// parallax occlusion off.
    TerrainRelief,
    /// Global wetness 0..1 (XV-H).
    TerrainWetness,
    /// Unique-colour / macro blend strength 0..1 (XV-Zeta).
    TerrainMacroStrength,
    /// Debug visualisation code (same numbers as `SOMNIUM_SHADOW_DEBUG`).
    TerrainDebugView,
    /// CDLOD morph start as a 0..1 fraction of the LOD range (Phase 25C).
    TerrainMorphStart,
    // First-class lake body settings (Phase IV-C).
    WaterSurface,
    WaterMaxDepth,
    WaterClarity,
    WaterAmplitude,
    WaterRoughness,
    WaterSsrStrength,
    WaterRtReflect,
    WaterReflectDebug,
    WaterWaveLengthA,
    WaterWaveLengthB,
    WaterWaveSpeed,
    WaterWaveSteepness,
    WaterWindSpeed,
    WaterFoamDecay,
    WaterFoamThreshold,
    WaterSpectrumBlend,
    WaterEdgeScale,
    WaterAnisotropy,
    WaterCausticStrength,
    // Vessel buoyancy (Phase IV). Visible when the selected entity has a
    // `BuoyantVessel`.
    VesselBuoyancy,
    VesselDrag,
    VesselAngularDrag,
    VesselThrust,
    VesselDraft,
    VesselRighting,
    // Foliage (Phase 17C) — only for entities with a `FoliageComponent`.
    FoliageDensity,
    FoliageSeed,
    FoliageSlope,
    FoliageLayer,
    FoliageScaleMin,
    FoliageScaleMax,
    /// Metres from the camera past which foliage stops casting shadows
    /// (Phase 24AE). Nearer than the draw distance on purpose.
    FoliageShadowDistance,
    FoliageCullDistance,
    FoliageLodDistance,
    FoliageImpostorDistance,
    WaterWaveDirAX,
    WaterWaveDirAZ,
    WaterWaveDirBX,
    WaterWaveDirBZ,
    WaterAbsorptionMag,
    WaterScatteringMag,
}

/// Colour property a swatch / picker writes (Phase 26-F Iris).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorField {
    Light,
    WaterDeep,
    WaterShallow,
    WaterEdge,
    WaterAbsorption,
    WaterScattering,
    ParticleStart,
    ParticleEnd,
    MaterialBase,
}

/// Which post-processing effect a toggle click targets (Phase 15A1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostFxToggle {
    Vignette,
    ChromaticAberration,
    Fxaa,
    /// Meter the scene each frame instead of using a fixed EV100.
    AutoExposure,
    /// Banded cel shading in place of PBR.
    CelShading,
    /// Bloom (Phase 24T).
    Bloom,
    /// Screen-space ambient occlusion (Phase 24I).
    Gtao,
    /// Depth of field (Phase 24Z).
    DepthOfField,
    /// Temporal anti-aliasing (Phase 24F).
    Taa,
    /// Ray-traced direct lighting (Phase 24K).
    Restir,
    /// Ray-traced indirect diffuse (Phase 24L).
    RestirGi,
    /// Contrast adaptive sharpening (Phase 24AC).
    Cas,
    /// Motion blur (Phase 24Z, on 24AD's velocity).
    MotionBlur,
    /// Froxel volumetrics: aerial perspective and fog (Phases 24U, 25I).
    Volumetrics,
    /// Shadow-test the fog per froxel, which is what draws light shafts.
    LightShafts,
    /// Drive exposure from aperture/shutter/ISO instead of a raw EV100
    /// (Phase 24A). With this off the three camera rows do nothing.
    PhysicalCamera,
    /// Percentage-closer soft shadows in the shading pass. Default on.
    Pcss,
    /// Screen-space contact shadows. Default on.
    ContactShadows,
    /// Ray-traced water reflections (Phase VV — Halcyon).
    RtReflect,
    /// Ray-traced water refraction (Phase VV+1). Default off.
    RtRefract,
    /// World-space radiance cache (Phase 24M). Default off.
    WorldCache,
    /// Scene-wide ray-traced specular (Phase 24N). Default off.
    SpecularGi,
    /// Offline path tracer (Phase 24O). Default off.
    PathTracer,
    /// Mesh-SDF cone trace (Phase 24P). Default off.
    MeshSdf,
    /// Probe/env fallback into the world cache (Phase 24Q). Default off.
    Probes,
    /// Analytic UV gradients (Phase 25N). Default on.
    AnalyticGrad,
    /// AMD FSR 3 temporal upscale. Default on; owns AA while enabled.
    Fsr,
}

/// High-level editor commands produced by the native UI layer.
/// `app.rs` drains these after each frame and applies them to the ECS world.
#[derive(Debug, Clone)]
pub enum EditorEvent {
    SelectEntity(Option<u32>),
    CreateEntity(CreateKind),
    DeleteSelected,
    DuplicateSelected,
    Undo,
    Redo,
    ToggleShadingMode,
    /// Start or resume deterministic game/physics time.
    PlaySimulation,
    /// Freeze game/physics time while keeping the editor interactive.
    PauseSimulation,
    /// Return to edit mode and reset the simulation clock.
    StopSimulation,
    /// Hide editor chrome and fill the monitor with the 3D view. Esc toggles off.
    ToggleImmersiveViewport,
    /// 0 translate, 1 rotate, 2 scale — same as T / R / S.
    SetGizmoMode(u8),
    /// Arm or disarm terrain sculpt (F6).
    ToggleTerrainEdit,
    SaveScene,
    NewScene,
    LoadScene(String),
    /// `live` marks an in-progress drag-scrub: apply it to the scene, but do
    /// not record an undo entry yet. The gesture ends with one non-live event
    /// carrying the final value, and that is what becomes a single undo step.
    SetInspectorValue {
        field: InspectorField,
        value: f32,
        live: bool,
    },
    /// Select a terrain sculpt/paint tool (Phase 14F). Index maps to
    /// `BrushMode`: 0 Raise, 1 Lower, 2 Smooth, 3 Flatten, 4 Noise, 5 Paint.
    SetTerrainTool(u8),
    /// Palette click: set the paint layer (XV-I). The engine also arms
    /// terrain paint and turns foliage paint off (XV-Zeta).
    SetTerrainPaintLayer(u8),
    /// Arm or disarm terrain layer paint from the inspector (XV-Zeta).
    ToggleTerrainPaint,
    /// Hex anti-tiling on the selected terrain. Default on.
    ToggleTerrainHex,
    /// Nested material clipmaps (Phase DF). Default off until DF-E gates pass.
    ToggleTerrainClipmap,
    /// CPU camera-frustum early-out (Phase CR-B). Default on. Independent of F10.
    /// The bool is the checkbox value, not a toggle — applying Check as a flip
    /// turned a default-on flag off the first time the inspector refreshed.
    SetCpuFrustum(bool),
    /// CDLOD vertex morphing on the selected terrain (Phase 25C). Default off.
    ToggleTerrainMorph,
    /// Toggle whether painted foliage is shown (Phase 17C).
    ToggleFoliage,
    /// Arm the foliage brush, so dragging in the viewport paints (Phase 17F).
    ToggleFoliagePaint,
    /// Flip the brush between adding and erasing.
    ToggleFoliageErase,
    /// Place one instance per click instead of a spread — how trees go down.
    ToggleFoliageSingle,
    /// Pick which palette entry the foliage brush paints (Phase 17F).
    SelectFoliageKind(u8),
    /// Flip a post-processing effect on the selected Post Processing entity.
    TogglePostFx(PostFxToggle),
    /// Cycle the tone-mapping curve (AgX → ACES → Reinhard).
    CycleTonemapper,
    /// Set the tone-mapping curve by index (0 AgX, 1 ACES, 2 Reinhard).
    SetTonemapper(u8),
    /// Viewport toolbar camera-speed slider moved. Value is normalized `0..=1`
    /// (the engine maps it exponentially to a world speed).
    SetCameraSpeed(f32),
    /// Internal 3D resolution preset (0 Native, 1 1440p, 2 1080p, 3 900p, 4 720p).
    /// The swapchain and UI stay at the window size; scene passes render smaller
    /// and upscale.
    SetViewportResolution(u8),
    /// Show or hide the profiler overlay (Phase 29). Also starts and stops the
    /// GPU timestamp collection, because a profiler nobody is looking at should
    /// not be spending queries.
    ToggleProfiler,
    /// File > Import Model — opens a native file picker and imports a glTF/GLB
    /// model into the scene at the world origin (Phase 19B).
    ImportModel,
    /// Live vs commit matches [`SetInspectorValue`]. `rgba` is linear.
    SetInspectorColor {
        field: ColorField,
        rgba: [f32; 4],
        live: bool,
    },
    /// Restore the colour captured when the picker opened; no undo entry.
    CancelInspectorColor {
        field: ColorField,
        rgba: [f32; 4],
    },
    ToggleWaterUnderwater,
    /// Title-bar close — same path as the native window X.
    CloseWindow,
}
