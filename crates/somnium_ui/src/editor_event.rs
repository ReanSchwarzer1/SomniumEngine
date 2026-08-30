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
    /// Authored spatial or non-spatial sound source.
    AudioEmitter,
    /// An authored path. The primitive roads, rivers and shaped emitters read.
    Spline,
    /// An audio emitter whose sound is heard along a path rather than from a
    /// point — a shoreline, a river, a road with traffic on it.
    ShorelineAudio,
    Particle,
    Terrain,
    VoxelTerrain,
    /// Runtime HUD, world-space panel, or projected overlay root.
    UiCanvas,
    /// CONTROL-L/M/N. One entity carrying the scene's day cycle, sky and
    /// weather. One row rather than three because they are one authored
    /// object: coverage drives precipitation drives wetness, and splitting
    /// them across three entities would make that chain a wiring exercise.
    Environment,
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
            Self::AudioEmitter => "Audio Emitter",
            Self::Spline => "Spline",
            Self::ShorelineAudio => "Shoreline Audio",
            Self::Particle => "Particle Emitter",
            Self::Terrain => "Terrain",
            Self::VoxelTerrain => "Voxel Terrain",
            Self::UiCanvas => "UI Canvas",
            Self::Environment => "Environment",
        }
    }
}

/// Renderer-owned terrain controls which cannot be represented as ECS fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerrainToolField {
    AerialDistance,
    PaintLayer,
    TileScale,
    Relief,
    Wetness,
    MacroStrength,
    DebugView,
    MorphStart,
}

/// Brush/runtime controls which deliberately remain outside component schemas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FoliageBrushField {
    Density,
    Radius,
    MaxSlope,
    Kind,
    ScaleMin,
    ScaleMax,
}

// ── Phase 16-D: scripting ───────────────────────────────────────────────────
//
// The Details panel's Scripts section is **generated** from whatever the
// script declared, so unlike every other section above it has no fixed
// field enum and no fixed row count. That is deliberate: hand-writing a
// per-script field UI is the failure mode Phase 16 exists to avoid, and it
// is a review failure rather than a shortcut.
//
// These types are plain data with no dependency on `somnium_script`, so
// the UI crate stays below the scripting crates in the graph. `app.rs`
// translates one into the other.

/// How one exported property is edited.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptFieldKind {
    /// A number, with the bounds the script declared.
    Number {
        /// Current value.
        value: f32,
        /// Declared minimum, if any.
        min: Option<f32>,
        /// Declared maximum, if any.
        max: Option<f32>,
    },
    /// A checkbox.
    Bool(bool),
    /// Read-only text. Used for the property kinds the editor cannot yet
    /// author — a string, an entity reference, an asset reference — so
    /// they are *visible* rather than silently missing.
    Text(String),
}

/// One exported property row.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptFieldRow {
    /// The name the script declared. Also the key the value is stored
    /// under, and what the editor sends back.
    pub name: String,
    /// How to edit it.
    pub kind: ScriptFieldKind,
    /// The script's own description, shown as a tooltip.
    pub description: Option<String>,
}

/// One attachment in the Details panel.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptAttachmentRow {
    /// File name of the script asset.
    pub asset_name: String,
    /// Lifecycle state, or why there is none.
    pub status: String,
    /// The author's enable flag.
    pub enabled: bool,
    /// Whether the error quarantine switched it off. Shown differently
    /// from an authored disable, because one is a bug report and the other
    /// is a choice.
    pub quarantined: bool,
    /// Declared properties, in declaration order.
    pub fields: Vec<ScriptFieldRow>,
}

/// The whole Scripts section for the current selection.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScriptInspectorState {
    /// Attachments in authored order.
    pub attachments: Vec<ScriptAttachmentRow>,
}

/// High-level editor commands produced by the native UI layer.
/// How a click combines with what is already selected.
///
/// Named rather than passed as two booleans because the three cases are
/// mutually exclusive and the two-boolean form makes `command()+Shift`
/// look like a fourth state that nothing implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    /// A plain click. Replaces the selection.
    Replace,
    /// `command()`-click. Adds or removes one entity.
    Toggle,
    /// `Shift`-click. Selects the inclusive range from the anchor.
    Range,
}

/// One Outliner row, as core sees it.
///
/// Replaced the four-tuple in CONTROL-F. Typed filters, badges and the
/// script-error dot all need facts the tuple had nowhere to put, and adding a
/// fifth and sixth positional field would have made the call site unreadable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlinerRow {
    /// Entity index.
    pub id: u32,
    /// Display name.
    pub name: String,
    /// Depth in the hierarchy, for indentation.
    pub depth: u8,
    /// Whether this row has children to expand.
    pub has_children: bool,
    /// `EditorFlags::hidden`.
    pub hidden: bool,
    /// `EditorFlags::locked`.
    pub locked: bool,
    /// Any attached script failed to compile.
    pub script_error: bool,
    /// Lowercase component tags — `light`, `mesh`, `terrain`, `script`, … —
    /// which is what makes `type:light` a filter rather than a name search.
    pub tags: Vec<&'static str>,
}

/// `app.rs` drains these after each frame and applies them to the ECS world.
#[derive(Debug, Clone)]
pub enum EditorEvent {
    /// One completed editor drop. The core maps this semantic operation to
    /// exactly one command/undo record.
    CompleteDrop(crate::drag_drop::DropRequest),
    SelectEntity(Option<u32>),
    /// A modifier-aware Outliner or viewport click. The core owns the row
    /// order, so it — not the widget — resolves what a `Shift` range means.
    ModifySelection {
        id: u32,
        mode: SelectionMode,
    },
    /// Replace the whole selection at once: marquee, paste, "select children".
    SelectEntities(Vec<u32>),
    CreateEntity(CreateKind),
    DeleteSelected,
    DuplicateSelected,
    /// Copy the selection and everything beneath it into the entity clipboard.
    CopySelected,
    /// Paste the entity clipboard under the primary selection.
    PasteClipboard,
    /// Select every entity in the scene.
    SelectAll,
    /// Write one Seam-4 setting. Core refuses it if the environment has taken
    /// the field over, and says which variable did.
    SetSetting {
        component: somnium_ecs::reflect::StableId,
        field: somnium_ecs::reflect::FieldId,
        value: somnium_ecs::reflect::ReflectValue,
    },
    /// Restore every setting to its declared default.
    ResetAllSettings,
    /// Open a source file at a line in the configured external editor, or
    /// reveal it in the OS file browser when none is configured.
    OpenSource {
        file: String,
        line: u32,
        column: u32,
    },
    /// Put text on the system clipboard.
    CopyText(String),
    /// Move to a position in the undo history. `0` is the state before
    /// anything happened.
    JumpToHistory(usize),
    /// Publish the undo history so the panel can draw it.
    RequestHistory,
    /// Write a setting addressed by field *name*.
    ///
    /// The by-`FieldId` form is what a generated row emits, because it already
    /// holds the schema address. A hand-built control like the snap cluster
    /// knows the name and not the id, and resolving the name in core is
    /// cheaper than making every such control carry a schema lookup.
    SetSettingByName {
        component: somnium_ecs::reflect::StableId,
        field_name: &'static str,
        value: somnium_ecs::reflect::ReflectValue,
    },
    /// Flip a boolean setting addressed by field name.
    ToggleSetting {
        component: somnium_ecs::reflect::StableId,
        field_name: &'static str,
    },
    /// Ask for a different project folder — the 27-G picker, unblocked.
    OpenProjectPicker,
    /// Select a named debug visualisation. `"lit"` returns to the ordinary
    /// image, which is why the code that used to mean "off" needs no special
    /// case.
    SetDebugView(&'static str),
    /// Flip one named renderer pipeline switch.
    ToggleRenderSwitch(&'static str),
    /// Point the editor camera down a world axis: 0 top, 1 front, 2 side,
    /// 3 back to a perspective three-quarter view.
    ViewPreset(u8),
    /// Store the current camera pose in slot 1..=9.
    SetCameraBookmark(u8),
    /// Recall slot 1..=9.
    RecallCameraBookmark(u8),
    /// Orbit around the selection rather than around the camera itself.
    ToggleOrbitSelection,
    /// `command()`+right-click: list everything selectable under the cursor.
    OpenPiercingMenu,
    /// Choose one entity from the piercing menu.
    PickPierced(u32),
    /// `Esc` during a viewport rubber-band: drop it, change nothing.
    CancelMarquee,
    /// Toggle one Outliner badge. `lock` picks the column; `value` is `None`
    /// for a plain toggle and `Some` for a drag that is setting a whole run.
    ToggleEntityFlag {
        entity: u32,
        lock: bool,
        value: Option<bool>,
    },
    /// Frame the selection with the editor camera.
    FocusSelection,
    /// Begin an in-place rename of the primary selection.
    RenameSelected,
    /// Commit a rename. `entity` is an entity index, as everywhere else in
    /// this enum.
    RenameEntity {
        entity: u32,
        name: String,
    },
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
    /// Ask the engine to put up a file dialog and load whatever is chosen.
    ///
    /// The dialog lives in the engine layer because that is where `rfd` and
    /// the project root already are; the UI only knows that the author asked.
    OpenScenePicker,
    /// Put `path`'s asset into the first field of the selection that accepts
    /// its kind. The keyboard-and-menu route to the same place a drop lands,
    /// and the one that works when a drag does not.
    AssignAssetToSelection(String),
    /// The one schema-driven component property write path.
    SetComponentField {
        entity: Entity,
        component: StableId,
        field: FieldId,
        value: ReflectValue,
        gesture: GestureId,
        live: bool,
    },
    /// CONTROL-L: scrub the day cycle's clock from the viewport context bar.
    ///
    /// Addressed by *hour* rather than by `(entity, component, field)` because
    /// the context bar has no selection to address through — the day cycle is a
    /// scene singleton and the core is the only thing that knows which entity
    /// carries it. The core still routes this through the one generic field
    /// write, so it undoes exactly like a Details edit of the same field.
    SetTimeOfDayHour {
        hour: f32,
        live: bool,
    },
    /// CONTROL-M: apply a named sky preset to the scene's Sky component.
    ///
    /// One event carrying an id rather than one per preset: the core owns what
    /// the name means, and adding a fifth sky must not add a fifth variant
    /// here, a fifth arm in the dispatcher and a fifth row in the menu.
    SetSkyPreset(&'static str),
    /// CONTROL-N: apply a named weather state to the scene's Weather
    /// component. The transition itself is the driver's job, not this event's.
    SetWeatherPreset(&'static str),
    /// `live` marks an in-progress drag-scrub: apply it to the scene, but do
    /// not record an undo entry yet. The gesture ends with one non-live event
    /// carrying the final value, and that is what becomes a single undo step.
    SetTerrainToolValue {
        field: TerrainToolField,
        value: f32,
        live: bool,
    },
    SetFoliageBrushValue {
        field: FoliageBrushField,
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
    /// Hex anti-tiling on the selected terrain. Default on for Coastal.
    ToggleTerrainHex,
    /// Parallax occlusion on the selected terrain. The expensive POM march.
    ToggleTerrainParallax,
    /// Nested material clipmaps (Phase DF). Default off until DF-E gates pass.
    ToggleTerrainClipmap,
    /// CPU camera-frustum early-out (Phase CR-B). Default on. Independent of F10.
    /// The bool is the checkbox value, not a toggle — applying Check as a flip
    /// turned a default-on flag off the first time the inspector refreshed.
    SetCpuFrustum(bool),
    /// Phase DOOM-F: let the renderer scale the internal 3D resolution to hold
    /// the Camera entity's frame budget. Off by default.
    SetDynamicResolution(bool),
    /// Phase DOOM-E: shade terrain past `aerial_split` with a second, narrower
    /// pipeline. Off by default — measured invisible and 2.3 ms slower on its
    /// own, and a real look change with the 16-layer scan.
    SetTerrainAerial(bool),
    /// Phase DOOM-E: also cut the aerial pipeline's layer scan to the hero bank.
    SetTerrainAerialHeroBank(bool),
    /// Phase DOOM-B: count pixels per shading class. A diagnostic, ~0.08 ms.
    SetPixelCensus(bool),
    /// Phase DOOM-C: route tiles to per-bin shading pipelines. Off by default —
    /// correct but measured slower than the fullscreen draw at every tile size.
    SetShadeBins(bool),
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
    /// Set a post-processing effect on the selected Post Processing entity.
    /// Carrying the checkbox value makes UI synchronization idempotent.
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
    /// Cancel a job advertised by the status bar.
    CancelJob(u64),
    ToggleWaterUnderwater,
    /// Title-bar close — same path as the native window X.
    CloseWindow,

    // ── Phase 16-D: scripting ───────────────────────────────────────────
    /// Attach the `.luau` file at this path to the selected entity.
    AttachScript(String),
    /// Create a new `.luau` file in the current content folder from the
    /// strict-mode template, and attach it to the selection if there is
    /// one.
    CreateScript,
    /// Remove the attachment at this position in the selected entity's
    /// authored list.
    DetachScript(usize),
    /// Move an attachment earlier (`-1`) or later (`+1`) in execution
    /// order.
    ReorderScript {
        /// Position in the authored list.
        index: usize,
        /// Which way to move it.
        delta: i32,
    },
    /// Switch an attachment on or off. Carries the value rather than
    /// toggling, so refreshing the panel is idempotent.
    SetScriptEnabled {
        /// Position in the authored list.
        index: usize,
        /// The checkbox value.
        enabled: bool,
    },
    /// Edit one exported number. `live` follows the same convention as
    /// Reflected property scrubs apply live but do not
    /// recorded, and the gesture's final value is one undo step.
    SetScriptNumber {
        /// Position in the authored list.
        index: usize,
        /// Declared property name.
        field: String,
        /// New value.
        value: f32,
        /// Mid-drag.
        live: bool,
    },
    /// Edit one exported boolean.
    SetScriptBool {
        /// Position in the authored list.
        index: usize,
        /// Declared property name.
        field: String,
        /// New value.
        value: bool,
    },
    /// Recompile every script asset from disk, carrying declared state
    /// across. A file that no longer compiles leaves its instances
    /// running and publishes diagnostics.
    ReloadScripts,

    // ── Content Drawer authoring ────────────────────────────────────────
    /// Create a folder named `name` inside `parent` (a content-relative
    /// directory; empty means the content root).
    CreateContentFolder {
        /// Directory the new folder goes in.
        parent: String,
        /// Leaf name, as typed.
        name: String,
    },
    /// Create a `.luau` file from the strict-mode template inside
    /// `parent`, and attach it to the selection if there is one.
    CreateContentScript {
        /// Directory the new script goes in.
        parent: String,
        /// Leaf name, as typed. A missing `.luau` extension is added.
        name: String,
    },
    /// Create an editable `.sommat` asset in `parent`.
    CreateContentMaterial {
        parent: String,
        name: String,
    },
    /// Rename a file or folder. `path` is absolute.
    RenameContentItem {
        /// What to rename.
        path: String,
        /// New leaf name, as typed.
        name: String,
    },
    /// Reveal a content item in the OS file browser.
    ///
    /// Deliberately the only "open" this phase ships: opening a `.luau`
    /// in an IDE is a later sub-phase, and pretending to do it by
    /// launching whatever is associated with the extension would be a
    /// worse experience than saying where the file is.
    ShowContentItemInFolder(String),
    /// Open an asset in its configured OS editor.
    EditContentAsset(String),
    /// Copy an asset beside itself and commit the new reference through the
    /// same reflected undo path.
    MakeAssetUnique {
        source: String,
        entity: Entity,
        component: StableId,
        field: FieldId,
    },
    /// Assign one material to a whole selection as one undo step.
    AssignMaterial {
        entities: Vec<Entity>,
        asset: somnium_asset::database::AssetId,
    },
}
use somnium_ecs::Entity;
use somnium_ecs::reflect::{FieldId, ReflectValue, StableId};

/// Identity shared by all live updates and the final commit of one gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GestureId(pub u64);
