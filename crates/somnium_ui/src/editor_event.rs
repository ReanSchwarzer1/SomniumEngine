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
    // Terrain layers (Phase 17C) — only for entities with a `TerrainComponent`.
    /// Which splat layer the sculpt brush paints, 0..=3.
    TerrainPaintLayer,
    /// World-space tiling of each splat layer's texture.
    TerrainTile0,
    TerrainTile1,
    TerrainTile2,
    TerrainTile3,
    /// Phase 25H: multiplies every layer's authored relief depth. 0 switches
    /// parallax occlusion off.
    TerrainRelief,
    // First-class lake body settings (Phase IV-C).
    WaterSurface,
    WaterMaxDepth,
    WaterClarity,
    WaterAmplitude,
    WaterRoughness,
    WaterSsrStrength,
    WaterWaveLengthA,
    WaterWaveLengthB,
    WaterWaveSpeed,
    WaterWaveSteepness,
    WaterWindSpeed,
    WaterFoamDecay,
    WaterFoamThreshold,
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
    /// Viewport toolbar camera-speed slider moved. Value is normalized `0..=1`
    /// (the engine maps it exponentially to a world speed).
    SetCameraSpeed(f32),
    /// Show or hide the profiler overlay (Phase 29). Also starts and stops the
    /// GPU timestamp collection, because a profiler nobody is looking at should
    /// not be spending queries.
    ToggleProfiler,
    /// File > Import Model — opens a native file picker and imports a glTF/GLB
    /// model into the scene at the world origin (Phase 19B).
    ImportModel,
}
