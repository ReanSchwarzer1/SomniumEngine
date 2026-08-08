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
            Self::Cube             => "Cube",
            Self::Sphere           => "Sphere",
            Self::Plane            => "Plane",
            Self::Cylinder         => "Cylinder",
            Self::DirectionalLight => "Directional Light",
            Self::PointLight       => "Point Light",
            Self::SpotLight        => "Spot Light",
            Self::Particle         => "Particle Emitter",
            Self::Terrain          => "Terrain",
            Self::VoxelTerrain     => "Voxel Terrain",
        }
    }
}

/// Which TRS component a NumericField targets (for SetInspectorValue).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorField {
    PosX, PosY, PosZ,
    RotX, RotY, RotZ,
    ScaleX, ScaleY, ScaleZ,
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
    // Post-processing (Phase 15A1) — only for entities with a
    // `PostProcessComponent`.
    /// Manual exposure value at ISO 100 (Phase 24A). Only used when auto
    /// exposure is off.
    PostExposure,
    /// Stops added on top of the metered exposure. Negative darkens — this is
    /// the control for "auto-exposure is right but I want it a stop down".
    PostExposureCompensation,
    PostVignetteStrength,
    PostCaStrength,
    /// Scene-wide indirect-light strength (Phase 22C).
    PostIblIntensity,
    // Terrain layers (Phase 17C) — only for entities with a `TerrainComponent`.
    /// Which splat layer the sculpt brush paints, 0..=3.
    TerrainPaintLayer,
    /// World-space tiling of each splat layer's texture.
    TerrainTile0,
    TerrainTile1,
    TerrainTile2,
    TerrainTile3,
    // Foliage (Phase 17C) — only for entities with a `FoliageComponent`.
    FoliageDensity,
    FoliageSeed,
    FoliageSlope,
    FoliageLayer,
    FoliageScaleMin,
    FoliageScaleMax,
}

/// Which post-processing effect a toggle click targets (Phase 15A1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostFxToggle {
    Vignette,
    ChromaticAberration,
    Fxaa,
    /// Meter the scene each frame instead of using a fixed EV100.
    AutoExposure,
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
    SaveScene,
    NewScene,
    LoadScene(String),
    /// `live` marks an in-progress drag-scrub: apply it to the scene, but do
    /// not record an undo entry yet. The gesture ends with one non-live event
    /// carrying the final value, and that is what becomes a single undo step.
    SetInspectorValue { field: InspectorField, value: f32, live: bool },
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
    /// File > Import Model — opens a native file picker and imports a glTF/GLB
    /// model into the scene at the world origin (Phase 19B).
    ImportModel,
}
