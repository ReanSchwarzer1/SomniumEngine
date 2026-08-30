//! # Somnium Core
//!
//! The foundational crate for the **Somnium Engine** — a modular,
//! high-performance, cross-platform 3D/2D game engine built in Rust.
//!
//! This crate provides the application lifecycle, platform event
//! abstraction, timing, configuration, and the primary [`GameApp`]
//! trait that all game applications implement.
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────┐
//! │         User Game (impl GameApp)         │
//! ├──────────────────────────────────────────┤
//! │   EngineContext  │  EngineEvent (enum)   │
//! ├──────────────────────────────────────────┤
//! │   Engine<G>  (winit ApplicationHandler)  │
//! ├──────────────────────────────────────────┤
//! │          winit / Operating System        │
//! └──────────────────────────────────────────┘
//! ```
//!
//! The engine decouples OS-level events from game logic through a
//! translation layer ([`event::translate_window_event`]), so that game
//! code never depends on `winit` types directly. This enables testing
//! with synthetic events and painless platform-layer swaps in the future.
//!
//! ## Reference Architecture
//!
//! The application lifecycle design draws inspiration from:
//!
//! - **Unreal Engine 5** (`FEngineLoop`: `PreInit → Init → Tick → Exit`)
//!   © Epic Games, Inc. — see `example_repo/UnrealEngine-release/`.
//!   Our `GameApp` trait mirrors UE5's phased lifecycle while using
//!   Rust's trait system instead of C++ virtual dispatch.
//!
//! - **Unreal Engine 5** (`GenericApplication` / `GenericApplicationMessageHandler`)
//!   © Epic Games, Inc. — platform abstraction with message handler
//!   delegation. Our event translation layer follows this pattern of
//!   decoupling OS events from game logic.

#![warn(missing_docs)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

/// Core application lifecycle and event loop management.
pub mod a11y_bridge;
pub mod app;
mod audio_scene;
pub mod autosave;
pub mod character;
pub mod clipboard;
pub mod config;
pub mod context;
/// Phase CONTROL-O: deferred decals.
pub mod decal;
pub mod editor_commands;
mod editor_gizmo;
pub mod error;
pub mod event;
pub mod i18n;
pub mod input_actions;
pub mod jobs;
pub mod landscape;
pub mod light_units;
pub mod log_capture;
pub mod map;
pub mod reflect_registry;
pub mod spline;
/// The `.somnium` container: a framed header the Content Drawer can read
/// without parsing the scene, and the three-format routing that goes with it.
///
/// Re-exported rather than defined here — it lives in `somnium_asset` because
/// that crate owns file containers and because the drawer's preview generator
/// needs it, and the dependency edge runs this way.
pub use somnium_asset::scene_file;

/// MORROWIND-T: CPU integer-grid floating origin and camera-relative values.
pub mod floating_origin;
pub mod scene_schema;
pub mod scene_serial;
pub mod script_bridge;
pub mod script_cook;
pub mod script_decls;
pub mod script_host;
pub mod script_input;
pub mod selection;
pub mod settings;
/// Phase CONTROL-M: the sky and its cloud layer.
pub mod sky;
pub mod sun;
pub mod time;
/// Phase CONTROL-L: the day cycle.
pub mod time_of_day;
/// Phase CONTROL-N: weather and the wetness it leaves.
pub mod weather;
/// MORROWIND-S: deterministic cell streaming and one-file-per-actor storage.
pub mod world_partition;

// ── Re-exports for ergonomic top-level access ──────────────────────────────

pub use app::{Engine, GameApp};
pub use character::RigidBodyComponent;
pub use config::EngineConfig;
pub use context::{EngineContext, SimulationClock, SimulationState};
pub use editor_commands::{
    AssignMaterialCmd, CreateEntityCmd, CreateLandscapeCmd, DeleteEntityCmd, EditorCommand,
    EntitySnapshot, ReparentCmd, SetLightCmd, SetNameCmd, SetTransformCmd, UndoStack,
};
pub use error::EngineError;
pub use event::{EngineEvent, InputState};
pub use landscape::{
    BuiltLandscape, DEFAULT_LANDSCAPE_VERSION, DefaultLandscapePreset, create_default_landscape,
    create_island_landscape,
};
pub use map::{
    DEFAULT_MAP_PATH, MapKind, MapLoadResult, load_map, parse_map_file, parse_map_kind_json,
    spawn_map,
};
pub use scene_serial::{parse_scene, save_scene};
pub use script_host::{
    AnimationParameterRouter, HostServices, ScriptHost, ScriptLogLine, SyncReport,
    apply_animation_parameter,
};
pub use script_input::{ScriptInputTracker, WorldCheckpoint};
pub use spline::SplineComponent;
pub use time::TimeState;

// Re-export input types so game code does not need a direct `winit` dependency.
pub use winit::event::ElementState;
pub use winit::event::MouseButton;
pub use winit::event::WindowEvent;
pub use winit::keyboard::{KeyCode, PhysicalKey};

// MORROWIND-E2: the runtime UI, re-exported so a game reaches its HUD through
// the crate it already depends on. `UiCanvas` is the tree, `GameUiFrame` is the
// moment it gets drawn in, and a game that needs neither pays nothing for them.
pub use somnium_ui::{Easing, GameUiFrame, Motion, Spring, Transition, UiCanvas};
// MORROWIND-I. A game declaring a role on its own widget, or turning reduced
// motion on from an options screen, reaches both through the crate it depends
// on already.
pub use a11y_bridge::A11yBridge;
pub use somnium_ui::a11y::{A11ySettings, A11yTree, Announcement, Politeness, Role, Toggled};

// Re-export core ECS types so game code can use them from `somnium_core`.
pub use somnium_ecs::{Component, ComponentBundle, Entity, PersistentId, World};
pub use somnium_ecs::{ComponentId, ComponentSet};

/// ECS Component for a mesh instance.
#[derive(Debug, Clone, Copy, Default)]
pub struct MeshComponent {
    /// Base offset of this mesh's vertices in the global vertex buffer.
    pub vertex_offset: u32,
    /// Base offset of this mesh's indices in the global index buffer.
    pub index_offset: u32,
    /// Number of indices to draw for this mesh.
    pub index_count: u32,
}
impl somnium_ecs::Component for MeshComponent {}

/// ECS Component for a material.
#[derive(Debug, Clone, Copy, Default)]
pub struct MaterialComponent {
    /// Durable authored reference written to the scene.
    pub asset: somnium_asset::database::AssetId,
    /// Renderer pool slot reconstructed from `asset`; never serialized.
    pub runtime_id: u32,
}
impl somnium_ecs::Component for MaterialComponent {}

/// Mixer destination for an authored sound emitter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AudioBus {
    /// Environmental and gameplay effects.
    #[default]
    Sfx,
    /// Music and score.
    Music,
    /// Spoken dialogue.
    Dialogue,
    /// Interface feedback.
    Ui,
}

/// Distance falloff used by an authored sound emitter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AudioAttenuationModel {
    /// Full volume inside `min_distance`, fading linearly to zero at max.
    Linear,
    /// Physically inspired inverse-square falloff, clamped at max distance.
    #[default]
    InverseSquare,
    /// An authored curve whose time axis is distance and value is gain.
    Authored,
    /// No distance attenuation (panning and Doppler may still apply).
    None,
}

/// Authored, inspectable spatial sound source.
///
/// Position and forward direction come from [`Transform`]. Runtime voice
/// handles deliberately live outside the ECS and are rebuilt when Play starts.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioEmitterComponent {
    /// Whether this emitter participates in playback.
    pub enabled: bool,
    /// Durable content-drawer reference to an audio file.
    pub audio: somnium_asset::database::AssetId,
    /// Start automatically when Play begins.
    pub autoplay: bool,
    /// Repeat until the emitter is disabled or Play stops.
    pub looping: bool,
    /// Linear source gain before mixer-bus volume and spatial attenuation.
    pub volume: f32,
    /// Route the sound through the matching mixer bus.
    pub bus: AudioBus,
    /// Enable position, distance, cone, occlusion, panning, and Doppler.
    pub spatial: bool,
    /// Distance falloff model.
    pub attenuation: AudioAttenuationModel,
    /// Radius at which the source still has full gain.
    pub min_distance: f32,
    /// Radius at which the source becomes silent.
    pub max_distance: f32,
    /// Distance-to-gain curve used by `Authored` attenuation.
    pub attenuation_curve: somnium_ecs::curve::Curve,
    /// Enable directional cone attenuation along local forward (-Z).
    pub cone_enabled: bool,
    /// Fully audible cone half-angle in degrees.
    pub cone_inner_degrees: f32,
    /// Cone half-angle at which `cone_outer_gain` is reached.
    pub cone_outer_degrees: f32,
    /// Gain outside the outer cone.
    pub cone_outer_gain: f32,
    /// Caller-authored transmission factor after an occlusion query.
    pub occlusion: f32,
    /// Per-emitter multiplier for Doppler pitch shift.
    pub doppler_scale: f32,
}

impl Default for AudioEmitterComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            audio: somnium_asset::database::AssetId::NONE,
            autoplay: true,
            looping: true,
            volume: 1.0,
            bus: AudioBus::Sfx,
            spatial: true,
            attenuation: AudioAttenuationModel::InverseSquare,
            min_distance: 2.0,
            max_distance: 40.0,
            attenuation_curve: somnium_ecs::curve::Curve::from_keys(vec![
                somnium_ecs::curve::CurveKey::new(0.0, 1.0),
                somnium_ecs::curve::CurveKey::new(40.0, 0.0),
            ]),
            cone_enabled: false,
            cone_inner_degrees: 45.0,
            cone_outer_degrees: 90.0,
            cone_outer_gain: 0.2,
            occlusion: 1.0,
            doppler_scale: 1.0,
        }
    }
}

impl somnium_ecs::Component for AudioEmitterComponent {}

/// ECS Component for spatial transformation.
#[derive(Debug, Clone, Copy)]
pub struct Transform {
    /// Local-space position.
    pub translation: glam::Vec3,
    /// Local-space orientation.
    pub rotation: glam::Quat,
    /// Local-space scale (per-axis).
    pub scale: glam::Vec3,
}

impl Transform {
    /// Create a transform at `translation` with identity rotation and unit scale.
    pub fn from_translation(translation: glam::Vec3) -> Self {
        Self {
            translation,
            rotation: glam::Quat::IDENTITY,
            scale: glam::Vec3::ONE,
        }
    }

    /// Compose this TRS into a 4×4 model matrix.
    pub fn to_matrix(&self) -> glam::Mat4 {
        glam::Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}
impl somnium_ecs::Component for Transform {}

/// Light type selector for `LightComponent`.
///
/// `Directional` — infinite-range sun light (Phase 11).
/// `Point` / `Spot` — local lights with range & falloff (Phase 13C).
/// `Rect` / `Disc` / `Tube` — area lights (Phase 24R).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightType {
    /// Infinite-range sun light (direction only).
    Directional,
    /// Local omnidirectional light with range falloff.
    Point,
    /// Local cone light with range falloff and inner/outer angles.
    Spot,
    /// Rectangular area light (Phase 24R, LTC). Width/height are half-extents.
    Rect,
    /// Disc area light (Phase 24R). `source_radius` is the disc radius; forward is the normal.
    Disc,
    /// Capsule / tube area light (Phase 24R). `area_width` is half-length; `source_radius` is radius.
    Tube,
}

/// Authored shadow implementation for a light.
///
/// Virtual lazily requests the sparse physical-page path. The renderer
/// preserves the choice but takes its explicit CSM fallback whenever resource
/// creation, page raster, or sampling is unavailable on the active adapter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LightShadowTechnique {
    /// Conventional cascaded shadow maps.
    #[default]
    Cascaded,
    /// Experimental virtual shadow-map request with a cascaded fallback.
    Virtual,
}

/// ECS component that marks an entity as a light source.
///
/// Direction comes from the entity's `Transform.rotation`, and two opposite
/// vectors are in play — mixing them up aims a spot light backwards:
///
/// - `forward = rotation * Vec3::NEG_Z` — the direction light **travels**.
///   This is the spot cone's axis (`GpuLocalLight::direction_ws`).
/// - `-forward` — the direction **toward** the light, which is what the
///   directional BRDF wants for `N·L` (`set_directional_light`).
///
/// For a directional light, `Transform.translation` is ignored.
///
/// Phase 13C additions:
/// - `range` — attenuation radius for point/spot lights (meters).
/// - `inner_angle` / `outer_angle` — spot cone angles in **radians**.
///   Inner is the fully-lit core; outer is where intensity fades to zero.
///
/// ```rust
/// use somnium_core::LightComponent;
/// // Directional
/// LightComponent::directional(5.0);
/// // Point (white, intensity 3, range 10m)
/// LightComponent::point(3.0, 10.0);
/// // Spot (white, intensity 5, range 15m, 25° inner / 35° outer)
/// LightComponent::spot(5.0, 15.0, 25.0_f32.to_radians(), 35.0_f32.to_radians());
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightComponent {
    /// Which kind of light this is (directional / point / spot).
    pub light_type: LightType,
    /// Per-light CSM/VSM selection. Directional lights support both authored
    /// values; local-light shadow rendering currently falls back to CSM/off.
    pub shadow_technique: LightShadowTechnique,
    /// Linear-RGB color of the light.
    pub color: glam::Vec3,
    /// Physical light output (Phase 24A).
    ///
    /// Units depend on `light_type`, matching how the real fixture is spec'd:
    /// - [`LightType::Directional`] — **illuminance in lux**, e.g. 100 000 for
    ///   direct midday sun, 1 000 overcast, 0.05 under a full moon.
    /// - [`LightType::Point`] / [`LightType::Spot`] — **luminous power in
    ///   lumens**, the number printed on a bulb's box (~800 for a 60 W).
    ///
    /// Presets live in [`light_units::lux`] and [`light_units::lumens`]. This
    /// used to be an arbitrary multiplier, which is why no value of it could
    /// mean "night" — see [`light_units`].
    pub intensity: f32,
    /// Colour temperature in Kelvin (Phase 24E).
    ///
    /// When non-zero this drives `color`, so the light's hue comes from one
    /// physically meaningful dial instead of three coupled RGB channels that
    /// can express colours no real emitter produces. Presets in
    /// [`light_units::kelvin`]. Zero keeps whatever `color` holds, so lights
    /// authored before this existed still behave.
    pub color_temperature_k: f32,
    /// Radius of the emitting surface in metres (Phase 24V).
    ///
    /// Distinct from `range`, which is how far the light reaches. This is how
    /// big the source itself is, and it governs highlight size and shadow
    /// softness. A bare bulb is roughly 0.03, a softbox 0.5.
    pub source_radius: f32,
    /// Attenuation radius for point/spot lights. Ignored for directional.
    pub range: f32,
    /// Spot inner cone half-angle (radians). Fully-lit region.
    pub inner_angle: f32,
    /// Spot outer cone half-angle (radians). Zero intensity at this edge.
    pub outer_angle: f32,
    /// Directional moonlight illuminance in lux (Phase 25M-2). Default 0.010 lux.
    pub moon_intensity: f32,
    /// Rect-light half-width in metres (Phase 24R). Ignored for other kinds.
    pub area_width: f32,
    /// Rect-light half-height in metres (Phase 24R). Ignored for other kinds.
    pub area_height: f32,
}

impl LightComponent {
    /// Linear-RGB tint, from colour temperature when one is set.
    #[must_use]
    pub fn tint(&self) -> glam::Vec3 {
        if self.color_temperature_k > 0.0 {
            light_units::kelvin_to_rgb(self.color_temperature_k)
        } else {
            self.color
        }
    }

    /// Photometric intensity converted to the quantity shading expects and
    /// scaled by the light's linear-RGB tint.
    ///
    /// Directional lights hand over illuminance unchanged; point and spot
    /// lights convert luminous power to intensity because shading divides by
    /// distance squared and therefore needs candela rather than lumens.
    #[must_use]
    pub fn photometric_color(&self) -> glam::Vec3 {
        let scale = match self.light_type {
            LightType::Directional => self.intensity,
            LightType::Point | LightType::Rect | LightType::Disc | LightType::Tube => {
                light_units::point_candela(self.intensity)
            }
            LightType::Spot => light_units::spot_candela(self.intensity, self.outer_angle),
        };
        self.tint() * scale
    }

    /// Convenience constructor for a white directional light, in **lux**.
    pub fn directional(intensity: f32) -> Self {
        Self {
            light_type: LightType::Directional,
            shadow_technique: LightShadowTechnique::Cascaded,
            color: glam::Vec3::ONE,
            intensity,
            color_temperature_k: 0.0,
            source_radius: 0.03,
            range: 0.0,
            inner_angle: 0.0,
            outer_angle: 0.0,
            moon_intensity: 0.010,
            area_width: 0.0,
            area_height: 0.0,
        }
    }

    /// Convenience constructor for a white point light, in **lumens**.
    pub fn point(intensity: f32, range: f32) -> Self {
        Self {
            light_type: LightType::Point,
            shadow_technique: LightShadowTechnique::Cascaded,
            color: glam::Vec3::ONE,
            intensity,
            color_temperature_k: 0.0,
            source_radius: 0.03,
            range,
            inner_angle: 0.0,
            outer_angle: 0.0,
            moon_intensity: 0.0,
            area_width: 0.0,
            area_height: 0.0,
        }
    }

    /// Convenience constructor for a white spot light, in **lumens**.
    pub fn spot(intensity: f32, range: f32, inner_angle: f32, outer_angle: f32) -> Self {
        Self {
            light_type: LightType::Spot,
            shadow_technique: LightShadowTechnique::Cascaded,
            color: glam::Vec3::ONE,
            intensity,
            color_temperature_k: 0.0,
            source_radius: 0.03,
            range,
            inner_angle,
            outer_angle,
            moon_intensity: 0.0,
            area_width: 0.0,
            area_height: 0.0,
        }
    }

    /// Convenience constructor for a white rectangular area light, in **lumens**.
    pub fn rect(intensity: f32, range: f32, half_width: f32, half_height: f32) -> Self {
        Self {
            light_type: LightType::Rect,
            shadow_technique: LightShadowTechnique::Cascaded,
            color: glam::Vec3::ONE,
            intensity,
            color_temperature_k: 0.0,
            source_radius: half_width.max(half_height),
            range,
            inner_angle: 0.0,
            outer_angle: 0.0,
            moon_intensity: 0.0,
            area_width: half_width,
            area_height: half_height,
        }
    }

    /// Convenience constructor for a white disc area light, in **lumens**.
    pub fn disc(intensity: f32, range: f32, radius: f32) -> Self {
        Self {
            light_type: LightType::Disc,
            shadow_technique: LightShadowTechnique::Cascaded,
            color: glam::Vec3::ONE,
            intensity,
            color_temperature_k: 0.0,
            source_radius: radius.max(0.05),
            range,
            inner_angle: 0.0,
            outer_angle: 0.0,
            moon_intensity: 0.0,
            area_width: 0.0,
            area_height: 0.0,
        }
    }

    /// Convenience constructor for a white tube area light, in **lumens**.
    ///
    /// `half_length` is metres along the entity forward axis from the centre;
    /// `radius` is the tube's cross-section.
    pub fn tube(intensity: f32, range: f32, half_length: f32, radius: f32) -> Self {
        Self {
            light_type: LightType::Tube,
            shadow_technique: LightShadowTechnique::Cascaded,
            color: glam::Vec3::ONE,
            intensity,
            color_temperature_k: 0.0,
            source_radius: radius.max(0.02),
            range,
            inner_angle: 0.0,
            outer_angle: 0.0,
            moon_intensity: 0.0,
            area_width: half_length.max(0.05),
            area_height: 0.0,
        }
    }
}

impl somnium_ecs::Component for LightComponent {}

/// ECS Component for an entity's display name.
///
/// Stored as a fixed-length null-terminated UTF-8 byte array so the component
/// satisfies the ECS `Copy` requirement. Names longer than 63 bytes are silently
/// truncated.
#[derive(Clone, Copy)]
pub struct Name(pub [u8; 64]);

impl Name {
    /// Create a `Name`, truncating to 63 bytes if necessary.
    pub fn new(s: &str) -> Self {
        let mut buf = [0u8; 64];
        let bytes = s.as_bytes();
        let len = bytes.len().min(63);
        buf[..len].copy_from_slice(&bytes[..len]);
        Self(buf)
    }

    /// Borrow the name as a `&str` (up to the null terminator).
    pub fn as_str(&self) -> &str {
        let end = self.0.iter().position(|&b| b == 0).unwrap_or(64);
        std::str::from_utf8(&self.0[..end]).unwrap_or("???")
    }
}

impl std::fmt::Debug for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Name({:?})", self.as_str())
    }
}

impl somnium_ecs::Component for Name {}

// ─── Phase 11.5F: Mesh kind tag ───────────────────────────────────────────

/// Records which procedural mesh type backs this entity's `MeshComponent`.
///
/// Stored alongside `MeshComponent` so the scene serializer can recreate the
/// mesh geometry on load. The value has no GPU-side meaning — it is purely
/// for serialization bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshKind {
    /// Unit cube.
    Cube,
    /// UV sphere.
    Sphere,
    /// Flat XZ plane.
    Plane,
    /// Capped cylinder.
    Cylinder,
}
impl somnium_ecs::Component for MeshKind {}

// ─── Phase 14: Heightmap terrain ────────────────────────────────────────────

/// Marks an entity as a heightmap terrain (Phase 14A-1).
///
/// Heightmap, splatmap, and layer data live OUTSIDE the ECS in the renderer's
/// `TerrainData` storage (like `GeometryPool` owns mesh data) — this component
/// only identifies the terrain and mirrors its configuration. `terrain_id`
/// indexes `SomniumRenderer::terrains`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainComponent {
    /// Index into the renderer's terrain storage.
    pub terrain_id: u32,
    /// Cells (quads) per chunk edge.
    pub chunk_cells: u32,
    /// Number of chunks along X.
    pub grid_x: u32,
    /// Number of chunks along Z.
    pub grid_z: u32,
    /// World-space distance between adjacent vertices (metres).
    pub cell_size: f32,
    /// World-space multiplier applied to raw heightmap values.
    pub height_scale: f32,
    /// Stream material source pages into a bounded cache before composing the runtime clipmap.
    pub virtual_texturing: bool,
    /// GPU budget for paired albedo/surface source pages, in MiB.
    pub virtual_texture_cache_mib: u32,
    /// Maximum source pages uploaded during one frame.
    pub virtual_texture_uploads_per_frame: u32,
    /// Source pages currently mapped by the runtime cache.
    pub virtual_texture_resident_pages: u32,
    /// Requested pages still waiting for an upload slot.
    pub virtual_texture_pending_pages: u32,
    /// Resident-page feedback hits since the cache was created.
    pub virtual_texture_hits: u32,
    /// Non-resident page requests since the cache was created.
    pub virtual_texture_misses: u32,
    /// Physical slots reused since the cache was created.
    pub virtual_texture_evictions: u32,
}
impl somnium_ecs::Component for TerrainComponent {}

/// Authored controls and live diagnostics for the terrain's world partition.
///
/// This component is the editor/runtime bridge: the terrain remains the
/// spatial surface while [`world_partition::WorldPartition`] owns streamed
/// actors. Runtime counters are refreshed by the engine and are read-only in
/// generated Details.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldPartitionComponent {
    /// Enable camera-driven cell streaming.
    pub enabled: bool,
    /// Edge length of one spatial-hash cell, in metres.
    pub cell_size: f32,
    /// Radius around the active camera that requests cells.
    pub load_radius: f32,
    /// Scheduling priority mapped onto the shared job system.
    pub source_priority: u32,
    /// Manually force one cell resident.
    pub pin_cell: bool,
    /// Manual pin coordinate.
    pub pin_x: i64,
    /// Manual pin coordinate.
    pub pin_y: i64,
    /// Manual pin coordinate.
    pub pin_z: i64,
    /// Cells currently requested by camera or editor pin.
    pub wanted_cells: u32,
    /// Cells with actors installed in the ECS.
    pub loaded_cells: u32,
    /// Cells with an asynchronous load/unload in flight.
    pub pending_cells: u32,
    /// Live actors owned by streamed cells.
    pub resident_actors: u32,
    /// Human-readable runtime state for the Details panel.
    pub status: String,
}

impl Default for WorldPartitionComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            cell_size: 64.0,
            load_radius: 128.0,
            source_priority: 128,
            pin_cell: false,
            pin_x: 0,
            pin_y: 0,
            pin_z: 0,
            wanted_cells: 0,
            loaded_cells: 0,
            pending_cells: 0,
            resident_actors: 0,
            status: "Waiting for camera".into(),
        }
    }
}

impl somnium_ecs::Component for WorldPartitionComponent {}

/// Space in which an authored UI canvas is attached.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UiCanvasSpace {
    /// Viewport-sized HUD/menu canvas.
    #[default]
    Screen,
    /// Quad attached to the entity transform.
    World,
    /// Screen-sized marker projected from the entity position.
    Overlay,
}

/// Discoverable ECS attachment for a runtime [`UiCanvas`].
///
/// The component describes placement and resolution. The game owns the widget
/// tree, as required by the runtime UI seam; Hello Engine supplies a visible
/// starter tree so Create → UI Canvas has an immediate result.
///
/// [`UiCanvas`]: somnium_ui::UiCanvas
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiCanvasComponent {
    /// Whether the game should draw this canvas.
    pub enabled: bool,
    /// Placement mode used by the game-owned widget tree.
    pub space: UiCanvasSpace,
    /// Logical reference width for screen canvases, or world width in metres.
    pub width: f32,
    /// Logical reference height for screen canvases, or world height in metres.
    pub height: f32,
    /// Offscreen density used by world canvases.
    pub pixels_per_unit: f32,
    /// Face the active camera when attached in world space.
    pub billboard: bool,
    /// Stable draw layer value.
    pub layer: i64,
}

impl Default for UiCanvasComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            space: UiCanvasSpace::Screen,
            width: 1920.0,
            height: 1080.0,
            pixels_per_unit: 100.0,
            billboard: true,
            layer: 10,
        }
    }
}

impl somnium_ecs::Component for UiCanvasComponent {}

/// Phase 17A: scatters foliage over the terrain on the same entity.
///
/// Mirrors `somnium_renderer::terrain::foliage::FoliageParams` rather than
/// holding it, so `somnium_core` does not depend on renderer types for a
/// component the editor has to be able to serialise and diff.
///
/// The instances themselves are deliberately **not** ECS entities. A terrain
/// scatters thousands of them and they are regenerated on every sculpt stroke,
/// which would flood the outliner and the undo stack — the same reason voxel
/// chunks stay out of the world. They are submitted as draw commands instead,
/// which also means they inherit the Phase 15 culling pipeline for free.
#[derive(Debug, Clone, PartialEq)]
pub struct FoliageComponent {
    /// Off by default: an empty terrain is the neutral thing to create, and
    /// scattering is a deliberate act.
    pub enabled: bool,
    /// Candidates per square metre, before slope and layer rejection.
    pub density: f32,
    /// Placement seed. Changing it reshuffles the layout.
    pub seed: u32,
    /// Ground steeper than this grows nothing (degrees).
    pub max_slope_deg: f32,
    /// Splat layer this foliage grows on, and the weight it needs there.
    pub layer: u8,
    /// Minimum weight of `layer` under a candidate for it to be kept.
    pub min_layer_weight: f32,
    /// Lower bound of the random uniform scale.
    pub scale_min: f32,
    /// Upper bound of the random uniform scale.
    pub scale_max: f32,
    /// Radius of the scattered disc around the camera, in metres. `0` covers
    /// the whole terrain, which only makes sense for small ones.
    pub radius: f32,
    /// Phase 17G: painted instances beyond this distance from the camera are
    /// not submitted at all.
    ///
    /// Ground cover is invisible long before it is far away — a tuft a few
    /// centimetres across is sub-pixel at a hundred metres — so drawing it is
    /// pure cost. Culling on the CPU keeps it out of the instance buffer and
    /// the indirect arguments entirely, which the GPU cull cannot do since the
    /// draw has to exist before it can be rejected.
    pub cull_distance: f32,
    /// CONTROL-K: instance scale as a function of normalised distance from the
    /// camera, where `0` is the camera and `1` is [`Self::cull_distance`].
    ///
    /// Empty — the default — means no falloff at all, which is exactly the
    /// behaviour before this field existed. A curve ending at zero shrinks
    /// ground cover out instead of popping it, which is the whole reason
    /// `cull_distance` is a hard edge worth softening.
    pub lod_falloff: somnium_ecs::curve::Curve,
    /// Phase 24AE: painted instances beyond this distance from the camera are
    /// still drawn, but stop casting shadows.
    ///
    /// A separate, *nearer* cut than [`Self::cull_distance`], and the reason it
    /// exists is what the profiler showed: a grass field fills the frame long
    /// before it reaches the draw-distance cut, and every one of those tufts
    /// was costing four cascades of depth for a shadow that reads as noise a
    /// few metres out. The automatic screen-radius test only rescues you once
    /// the *camera* is far away, which is not how anyone plays.
    ///
    /// `0` means "never stop", which is the A/B against the old behaviour.
    pub foliage_shadow_distance: f32,
    /// Past this **horizontal** distance leaf/cutout parts are dropped (Phase 25P).
    /// `0` keeps every part.
    pub lod_distance: f32,
    /// Past this **horizontal** distance only solid parts remain (Phase 25P).
    /// Not a billboard — the dummy camera-facing quad was deleted. `0` keeps
    /// every remaining part.
    pub impostor_distance: f32,
    /// Ceiling on instances, enforced by coarsening the scatter grid.
    pub max_instances: u32,
}

impl Default for FoliageComponent {
    fn default() -> Self {
        Self {
            enabled: false,
            // Dense enough to read as ground cover. Affordable only because
            // the scatter is a disc around the camera rather than the whole
            // terrain — see `radius`.
            density: 3.0,
            seed: 1,
            max_slope_deg: 35.0,
            layer: 0,
            min_layer_weight: 0.35,
            scale_min: 0.6,
            scale_max: 1.5,
            radius: 45.0,
            cull_distance: 120.0,
            lod_falloff: somnium_ecs::curve::Curve::empty(),
            // A third of the draw distance. Grass shadows stop reading as
            // individual blades within a few metres and as texture within a
            // few tens; past that they are noise that costs four cascades.
            foliage_shadow_distance: 40.0,
            lod_distance: 45.0,
            impostor_distance: 90.0,
            max_instances: 18_000,
        }
    }
}
/// Per-entity editor state: hidden in the viewport, locked against selection.
///
/// A component rather than a side table, so it serialises with the scene, undoes
/// through the ordinary reflected path, and survives a copy/paste — all three of
/// which a `HashSet<Entity>` on the editor would have got wrong. Absent means
/// "visible and unlocked", which is why the overwhelming majority of entities
/// carry no such component at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EditorFlags {
    /// Not submitted for drawing. Purely an authoring state: play and the
    /// runtime ignore it.
    pub hidden: bool,
    /// Cannot be picked, dragged or transformed in the viewport. The Outliner
    /// still selects it, because otherwise a locked object becomes unreachable.
    pub locked: bool,
}

/// Scene data this build does not understand, kept verbatim so saving cannot
/// destroy it — CONTROL-J, following Stride's `IUnloadable`.
///
/// Before this, `scene_from_json` skipped an unregistered component with a
/// warning and dropped an unknown field with a warning. A load-then-save in a
/// build missing a component therefore **destroyed that component's data
/// permanently**, and the only sign was a line in a log nobody was reading.
///
/// The rule is simple and total: anything the registry cannot name is stored
/// as opaque JSON on the entity it came from, and written back byte-for-byte
/// on the next save. A build that gains the component later reads its own data
/// again; a build that never gains it still does not corrupt anybody else's.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RetainedUnknowns {
    /// Whole components, keyed by their stable id: the untouched
    /// `{ "version": …, "fields": … }` body exactly as it was read.
    pub components: std::collections::BTreeMap<String, serde_json::Value>,
    /// Fields of *known* components that the schema no longer declares, keyed
    /// `component.field`. A renamed field survives a round trip through the
    /// build that renamed it.
    pub fields: std::collections::BTreeMap<String, serde_json::Value>,
}

impl somnium_ecs::Component for RetainedUnknowns {}

impl RetainedUnknowns {
    /// Whether anything is being carried.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.components.is_empty() && self.fields.is_empty()
    }

    /// Record a whole component this build cannot name.
    pub fn keep_component(&mut self, name: &str, body: serde_json::Value) {
        self.components.insert(name.to_owned(), body);
    }

    /// Record one field of a component this build *can* name.
    pub fn keep_field(&mut self, component: &str, field: &str, value: serde_json::Value) {
        self.fields.insert(format!("{component}.{field}"), value);
    }

    /// The retained fields belonging to `component`, as `(field, value)`.
    pub fn fields_of<'a>(
        &'a self,
        component: &'a str,
    ) -> impl Iterator<Item = (&'a str, &'a serde_json::Value)> + 'a {
        self.fields.iter().filter_map(move |(key, value)| {
            // Split at the *last* dot: a stable id is dotted
            // (`somnium.Name`) and a field name is not, so splitting at the
            // first one would read the owner as `somnium`.
            let (owner, field) = key.rsplit_once('.')?;
            (owner == component).then_some((field, value))
        })
    }
}

impl somnium_ecs::Component for EditorFlags {}

impl somnium_ecs::Component for FoliageComponent {}

/// Marks an entity as a voxel-terrain world (Phase 14 / 13E follow-up).
///
/// Like [`TerrainComponent`] this is only a handle: the voxel chunks, their GPU
/// allocations, and the streaming state live outside the ECS (in the game
/// layer's voxel driver), because chunks stream in and out constantly and would
/// otherwise flood the outliner and undo stack. The component exists so the
/// voxel world is created explicitly from **Create → Voxel Terrain**, shows up
/// in the outliner, and can be selected and deleted like any other entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoxelTerrainComponent {
    /// Streaming radius in chunks around the camera.
    pub radius_chunks: u32,
    /// Terrain generator seed.
    pub seed: u32,
}

impl Default for VoxelTerrainComponent {
    fn default() -> Self {
        Self {
            radius_chunks: 5,
            seed: 1337,
        }
    }
}
impl somnium_ecs::Component for VoxelTerrainComponent {}

// ─── Phase 20B: Editor camera speed ─────────────────────────────────────────

/// Slowest editor camera speed (m/s) at slider position 0.
pub const MIN_CAMERA_SPEED: f32 = 0.5;
/// Fastest editor camera speed (m/s) at slider position 1.
///
/// Generous on purpose: imported scenes are often authored at wildly different
/// scales, and crawling across a city-sized glTF at 5 m/s is unusable.
pub const MAX_CAMERA_SPEED: f32 = 500.0;
/// Slider position giving the historical default of ~5 m/s.
pub const DEFAULT_CAMERA_SPEED_NORM: f32 = 0.334;

/// Map a normalized slider position to a world speed.
///
/// Exponential rather than linear: the useful range spans three orders of
/// magnitude, and a linear slider would spend most of its travel on speeds too
/// fast to control while making slow, precise movement unselectable.
#[must_use]
pub fn camera_speed_from_normalized(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    MIN_CAMERA_SPEED * (MAX_CAMERA_SPEED / MIN_CAMERA_SPEED).powf(t)
}

/// Inverse of [`camera_speed_from_normalized`], for driving the slider from
/// the scroll wheel.
#[must_use]
pub fn normalized_from_camera_speed(speed: f32) -> f32 {
    let s = speed.clamp(MIN_CAMERA_SPEED, MAX_CAMERA_SPEED);
    (s / MIN_CAMERA_SPEED).ln() / (MAX_CAMERA_SPEED / MIN_CAMERA_SPEED).ln()
}

// ─── Phase 15A1: Post-processing volume ─────────────────────────────────────

/// Scene-wide post-processing settings, exposed as a selectable entity.
///
/// Screen-space effects used to be hard-coded on in the renderer, which meant
/// an always-on vignette with no way to switch it off. This component puts them
/// in the scene (as a "Post Processing" entity in the outliner) so they can be
/// inspected and toggled. Effects default to **off** — an editor viewport
/// should show the raw image unless you ask for a look.
///
/// The engine pushes these to the renderer each frame; if no such entity
/// exists, the renderer's own defaults (everything off) apply.
/// No longer `Copy`: CONTROL-K's [`response_curve`] is a heap-backed
/// [`Curve`](somnium_ecs::curve::Curve). The two call sites that copied this
/// component now clone it, once per frame, which is a `Vec` of at most a few
/// keys — measurably nothing against a pass that reads it into a uniform.
///
/// [`response_curve`]: PostProcessComponent::response_curve
#[derive(Debug, Clone, PartialEq)]
pub struct PostProcessComponent {
    /// Manual exposure value at ISO 100 (Phase 24A).
    ///
    /// Replaces the old linear multiplier. EV100 is a photographic stop scale —
    /// +1 halves the light reaching the sensor — so it stays usable across the
    /// ~10⁹ range that physical light units introduced. Presets are in
    /// [`light_units::ev100`]. Ignored when `auto_exposure` is on.
    pub ev100: f32,
    /// Aperture in f-stops. Drives `ev100` via [`light_units::ev100_from_camera`]
    /// when `use_physical_camera` is set.
    pub aperture_f_stops: f32,
    /// Shutter time in seconds.
    pub shutter_speed_s: f32,
    /// Sensor sensitivity in ISO.
    pub sensitivity_iso: f32,
    /// Derive `ev100` from aperture/shutter/ISO instead of setting it directly.
    pub use_physical_camera: bool,
    /// Meter the scene each frame and adapt `ev100` to it (Phase 24A-3).
    pub auto_exposure: bool,
    /// Stops added on top of the metered or manual value. Negative darkens.
    pub exposure_compensation: f32,
    /// Which tone-mapping curve maps HDR luminance to display (Phase 24B).
    pub tonemapper: Tonemapper,
    /// Colour grading, applied after tone mapping (Phase 24Y).
    ///
    /// Exposure and the tone curve decide how bright the image is and how it
    /// rolls off. These decide what it feels like, and no amount of the former
    /// substitutes for the latter.
    pub temperature: f32,
    /// Green-magenta colour balance; zero is neutral.
    pub tint: f32,
    /// Contrast adjustment; one is neutral.
    pub contrast: f32,
    /// Colour saturation adjustment; one is neutral.
    pub saturation: f32,
    /// ASC CDL slope / offset / power. Neutral is (1, 0, 1).
    pub gain: f32,
    /// ASC CDL offset applied to the graded colour; zero is neutral.
    pub lift: f32,
    /// ASC CDL power applied to the graded colour; one is neutral.
    pub gamma: f32,
    /// Film grain strength (Phase 24Z). 0 = off.
    pub grain: f32,
    /// CONTROL-K: authored tone response, applied per channel after the fixed
    /// grade. An empty curve — the default — leaves grading exactly as Phase
    /// 24Y left it, so this field costs nothing until somebody uses it.
    ///
    /// The domain and the range are both `0..=1`: the input is the graded LDR
    /// channel and the output is what replaces it. An identity ramp is a
    /// no-op, which is what makes "reset" and "off" the same gesture.
    pub response_curve: somnium_ecs::curve::Curve,
    /// Bloom (Phase 24T).
    pub bloom_enabled: bool,
    /// Strength of the bloom contribution; zero disables its visible effect.
    pub bloom_intensity: f32,
    /// Screen-space occlusion (Phase 24I).
    pub gtao_enabled: bool,
    /// Depth of field (Phase 24Z). Focus distance is in metres.
    pub dof_enabled: bool,
    /// Camera-space focus distance in metres.
    pub dof_focus_distance: f32,
    /// Which anti-aliasing runs (MORROWIND-AC).
    ///
    /// One authored value replacing `fxaa_enabled` / `taa_enabled` /
    /// `fsr_enabled`, which described eight states of which five were
    /// reachable. See [`AntiAliasing`].
    pub aa: AntiAliasing,
    /// SMAA quality preset. Only read when [`Self::aa`] uses SMAA.
    pub smaa_preset: SmaaPreset,
    /// Order-independent transparency (MORROWIND-AC).
    ///
    /// Off by default. The sorted path is correct for separated panes and
    /// wrong only where two blended surfaces of the same object intersect;
    /// weighted-blended is right there and approximate everywhere else, so this
    /// is an authored trade rather than a strict upgrade. Turning it on changes
    /// what an existing scene draws, which `phase_MORROWIND.md` §3 does not
    /// allow a sub-phase to do silently.
    pub oit_enabled: bool,
    /// Ray-traced direct lighting (Phase 24K).
    ///
    /// Only has an effect where the device granted ray query; the renderer
    /// falls back to the shadow map otherwise, so the toggle is safe to leave
    /// on regardless of hardware.
    pub restir_enabled: bool,
    /// Ray-traced indirect diffuse (Phase 24L).
    ///
    /// Replaces the environment map's *diffuse* half with a traced one-bounce
    /// solution; the specular lobe still comes from the cubemap. Off means the
    /// constant ambient term the engine has always had.
    pub restir_gi_enabled: bool,
    /// Ray-traced water reflections (Phase VV — Halcyon).
    ///
    /// Off, or `SOMNIUM_RT_REFLECT=0`, restores the previous SSR + environment
    /// cube look. Hardware without ray query skips the pass regardless.
    pub rt_reflect_enabled: bool,
    /// Ray-traced water refraction (Phase VV+1).
    ///
    /// Default **off**. Traces a Snell ray through the surface and replaces the
    /// screen-space bed sample on a hit. `SOMNIUM_RT_REFRACT=0` forces it off
    /// even if this toggle is on. Hardware without ray query skips it.
    pub rt_refract_enabled: bool,
    /// Contrast adaptive sharpening (Phase 24AC).
    ///
    /// Recovers the high frequencies TAA averages away, by an amount derived
    /// per pixel from the local contrast — so it does not halo the way a
    /// fixed-strength unsharp mask does.
    pub cas_enabled: bool,
    /// 0 = least ringing, 1 = maximum. AMD's own knob.
    pub cas_sharpness: f32,
    /// How far the sharpened image is blended in.
    pub cas_strength: f32,
    /// Motion blur (Phase 24Z), on 24AD's velocity buffer.
    pub motion_blur_enabled: bool,
    /// Shutter fraction: how much of the frame interval the shutter is open.
    /// 0.5 is a 180 degree shutter, the film default.
    pub motion_blur_shutter: f32,
    /// Strength of the traced indirect diffuse (Phase 24L).
    pub restir_gi_intensity: f32,
    /// Froxel volumetrics: aerial perspective and fog (Phases 24U, 25I).
    ///
    /// Covers the whole volume. Aerial perspective is not separately optional —
    /// the air between the camera and a distant hill is there whether or not
    /// fog has been dialled in — so `fog_density` controls only the medium
    /// layered on top of the atmosphere.
    pub volumetrics_enabled: bool,
    /// Shadow-test the fog per froxel, which is what draws light shafts.
    pub light_shafts: bool,
    /// Fog extinction per metre. ~1e-3 is a visible haze.
    pub fog_density: f32,
    /// Metres over which fog density falls to 1/e, so it pools in valleys.
    pub fog_height_falloff: f32,
    /// Henyey-Greenstein asymmetry; positive scatters forward toward the sun.
    pub fog_asymmetry: f32,
    /// GTAO sampling radius in metres (Phase 24I). Small values read as
    /// contact darkening; large ones as a broad dirty smear.
    pub gtao_radius: f32,
    /// How hard GTAO is applied. 0 is the same as switching it off.
    pub gtao_intensity: f32,
    /// Replace PBR shading with the banded cel look.
    ///
    /// Lived only behind the F5 key, which is easy to hit by accident and gave
    /// no indication of what had happened — the scene simply turned flat and
    /// dark with no visible control to undo it. Owning it here means the
    /// inspector shows the current state.
    pub cel_shading: bool,
    /// Whether the radial vignette is applied.
    pub vignette_enabled: bool,
    /// Vignette strength when enabled.
    pub vignette_strength: f32,
    /// Whether chromatic aberration is applied.
    pub ca_enabled: bool,
    /// Chromatic-aberration offset (UV units at the screen edge) when enabled.
    pub ca_strength: f32,
    /// Percentage-closer soft shadows in the shading pass. Default on.
    ///
    /// When ray-traced direct lighting has a result for the pixel, PCSS is
    /// skipped and the traced visibility is used instead — that is the higher
    /// quality path, not a downgrade. Turning this off uses a single-tap
    /// shadow map compare as a cheaper fallback.
    pub pcss_enabled: bool,
    /// Screen-space contact shadows. Default on.
    pub contact_shadows_enabled: bool,
    /// Scene-wide strength of image-based (indirect) light (Phase 22C).
    ///
    /// Phase 25M-2: physically neutral by default. GTAO, material occlusion,
    /// specular occlusion, and ReSTIR GI now provide the indirect-light
    /// visibility that the old pre-AO `0.35` workaround was waiting for.
    pub ibl_intensity: f32,
    /// World-space radiance cache (Phase 24M). Default off; `SOMNIUM_WORLD_CACHE=1`.
    pub world_cache: bool,
    /// How hard the cache contributes to ambient.
    pub cache_intensity: f32,
    /// Cache voxel size in metres.
    pub cache_cell_size: f32,
    /// Scene-wide ray-traced specular (Phase 24N). Default off.
    pub specular_gi: bool,
    /// Roughness cutoff for the specular GI trace.
    pub spec_roughness: f32,
    /// Offline path tracer (Phase 24O). Replaces the image while on. Default off.
    pub path_tracer: bool,
    /// Bounces the path tracer takes. 1..=8.
    pub path_bounces: u32,
    /// Mesh-SDF cone trace (Phase 24P). Default off.
    pub mesh_sdf: bool,
    /// SH irradiance probes (Phase 24Q). Default off.
    pub probes: bool,
    /// Probe contribution; scales the SH bake.
    pub probe_intensity: f32,
    /// MORROWIND-AB portable SDF-backed dynamic diffuse GI.
    pub ddgi_enabled: bool,
    /// Scene-wide DDGI diffuse contribution.
    pub ddgi_intensity: f32,
    /// Distance between neighbouring DDGI probes, in metres.
    pub ddgi_probe_spacing_m: f32,
    /// Probe records refreshed per frame (the volume contains 64).
    pub ddgi_update_budget: u32,
    /// Fraction of the previous probe value retained during an update.
    pub ddgi_hysteresis: f32,
    /// Analytic UV gradients in vis-buffer shading (Phase 25N). Default on.
    pub analytic_grad: bool,
    /// Light-shaft shadow contrast. 0 is neutral; 1 (or greater) is full.
    pub shaft_intensity: f32,
    /// FSR RCAS sharpness, 0..=1. Default 0.8.
    pub fsr_sharpness: f32,
}

impl Default for PostProcessComponent {
    fn default() -> Self {
        // `SOMNIUM_FSR=0` used to clear one of three booleans and leave the
        // other two to a precedence chain. It now selects the next rung down
        // the ladder, which is what a user turning FSR off actually wants.
        //
        // `SOMNIUM_AA` names a rung outright, which `SOMNIUM_FSR` cannot: it is
        // the Seam 4 override of the one authored value, and it exists because
        // an A/B across six modes from a timing harness must not require six
        // clicks in Details. It wins over `SOMNIUM_FSR` when both are set,
        // because it is the more specific statement.
        let aa = match std::env::var("SOMNIUM_AA")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "off" | "none" => AntiAliasing::Off,
            "fxaa" => AntiAliasing::Fxaa,
            "smaa" | "smaa1x" => AntiAliasing::Smaa1x,
            "smaa_t2x" | "smaat2x" | "t2x" => AntiAliasing::SmaaT2x,
            "taa" => AntiAliasing::Taa,
            "fsr" => AntiAliasing::Fsr,
            _ if std::env::var("SOMNIUM_FSR").as_deref() == Ok("0") => AntiAliasing::Taa,
            _ => AntiAliasing::Fsr,
        };
        let smaa_preset = match std::env::var("SOMNIUM_SMAA_PRESET")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "low" => SmaaPreset::Low,
            "medium" => SmaaPreset::Medium,
            "high" => SmaaPreset::High,
            _ => SmaaPreset::Ultra,
        };
        let world_cache = std::env::var("SOMNIUM_WORLD_CACHE").as_deref() == Ok("1");
        Self {
            ibl_intensity: 1.0,
            ev100: light_units::ev100::SUNLIGHT,
            aperture_f_stops: 16.0,
            shutter_speed_s: 1.0 / 100.0,
            sensitivity_iso: 100.0,
            use_physical_camera: false,
            auto_exposure: true,
            exposure_compensation: 0.0,
            tonemapper: Tonemapper::AgX,
            cel_shading: false,
            temperature: 0.0,
            tint: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            gain: 1.0,
            lift: 0.0,
            gamma: 1.0,
            grain: 0.0,
            response_curve: somnium_ecs::curve::Curve::empty(),
            // Deterministic audit switch; the editor checkbox remains the
            // runtime source of truth after startup.
            bloom_enabled: std::env::var("SOMNIUM_BLOOM").as_deref() != Ok("0"),
            bloom_intensity: 0.04,
            // `SOMNIUM_GTAO=0` switches screen-space occlusion off for the
            // Phase 25A acceptance test — the same scene either side of the
            // flag has to differ *on terrain*, and before 25A-2 it could not.
            //
            // Seeded here rather than in `GtaoPass`, for the reason the ReSTIR
            // comment below gives: this component is copied into the pass every
            // frame, so a pass-side default is overwritten before it can ever
            // take effect. (It was, and the first A/B run reported a difference
            // of exactly zero everywhere — which reads identically to a feature
            // that does not work.)
            gtao_enabled: std::env::var("SOMNIUM_GTAO").as_deref() != Ok("0"),
            // Phases 24U/25I. On by default: aerial perspective is physics, not
            // a look — a kilometre of air between the camera and a hill is
            // always there. `SOMNIUM_VOLUMETRICS=0` is the A/B switch.
            restir_gi_enabled: std::env::var("SOMNIUM_RESTIR_GI").as_deref() != Ok("0"),
            rt_reflect_enabled: std::env::var("SOMNIUM_RT_REFLECT").as_deref() != Ok("0"),
            rt_refract_enabled: false,
            // FSR already owns RCAS. Keep the authored checkboxes truthful:
            // two mutually-exclusive sharpeners must not both start checked.
            cas_enabled: aa != AntiAliasing::Fsr
                && std::env::var("SOMNIUM_CAS").as_deref() != Ok("0"),
            cas_sharpness: 0.5,
            cas_strength: 1.0,
            motion_blur_enabled: std::env::var("SOMNIUM_MOTION_BLUR").as_deref() == Ok("1"),
            motion_blur_shutter: 0.5,
            restir_gi_intensity: 1.0,
            volumetrics_enabled: std::env::var("SOMNIUM_VOLUMETRICS").as_deref() != Ok("0"),
            light_shafts: std::env::var("SOMNIUM_LIGHT_SHAFTS").as_deref() != Ok("0"),
            fog_density: 0.0008,
            fog_height_falloff: 120.0,
            fog_asymmetry: 0.6,
            // Mirrors `GtaoPass::new`; the component overwrites the pass every
            // frame, so these are the values that actually take effect.
            gtao_radius: 1.0,
            gtao_intensity: 1.0,
            dof_enabled: false,
            dof_focus_distance: 10.0,
            // FSR is temporal AA/reconstruction, so this is the fallback used
            // when FSR is disabled rather than a second checked no-op.
            // Seeded from the environment so the debug switch and the Post FX
            // toggle agree. The component is the single source of truth and is
            // copied into the pass every frame, so a pass-side default would be
            // overwritten before it ever took effect.
            // On by default.
            //
            // It was switched off earlier on the reading that it "returned lit"
            // and erased shadows. That was wrong: the missing shadows were the
            // GpuMaterial layout bug, which zeroed the sun term so there was
            // nothing for any shadow to darken, traced or not. With materials
            // fixed, traced and shadow-mapped agree — measured on a cube over a
            // plane, 3.0 against 3.1 in the shadow and 110.9 lit either way.
            // SOMNIUM_RESTIR=0 forces it off.
            restir_enabled: std::env::var("SOMNIUM_RESTIR").as_deref() != Ok("0"),
            vignette_enabled: false,
            vignette_strength: 1.0,
            ca_enabled: false,
            ca_strength: 0.004,
            pcss_enabled: true,
            contact_shadows_enabled: true,
            world_cache,
            cache_intensity: 1.0,
            cache_cell_size: 2.0,
            specular_gi: std::env::var("SOMNIUM_SPECULAR_GI").as_deref() == Ok("1"),
            spec_roughness: 0.15,
            path_tracer: std::env::var("SOMNIUM_PATH_TRACER").as_deref() == Ok("1"),
            path_bounces: 3,
            // Cache RGB and SDF distance share one volume whose alpha has
            // incompatible meanings. Cache wins if both audit env vars are set.
            mesh_sdf: !world_cache && std::env::var("SOMNIUM_MESH_SDF").as_deref() == Ok("1"),
            // MORROWIND-AB keeps the old audit switch but routes it to the
            // authored DDGI field. `probes` remains deserialize-only legacy.
            probes: false,
            probe_intensity: 1.0,
            ddgi_enabled: std::env::var("SOMNIUM_PROBES").as_deref() == Ok("1"),
            ddgi_intensity: 1.0,
            ddgi_probe_spacing_m: 2.0,
            ddgi_update_budget: 8,
            ddgi_hysteresis: 0.95,
            analytic_grad: std::env::var("SOMNIUM_ANALYTIC_GRAD").as_deref() != Ok("0"),
            shaft_intensity: 1.5,
            aa,
            smaa_preset,
            // Off unless asked for. `SOMNIUM_OIT=1` is the A/B route; the
            // authored control is the Details checkbox.
            oit_enabled: std::env::var("SOMNIUM_OIT").as_deref() == Ok("1"),
            fsr_sharpness: 0.8,
        }
    }
}

/// Which anti-aliasing runs, as **one** authored value (MORROWIND-AC).
///
/// Before AC this was three independent booleans — `fxaa_enabled`,
/// `taa_enabled`, `fsr_enabled` — resolved at
/// `renderer.rs` by a precedence chain:
///
/// ```text
/// fxaa_active = fxaa_enabled && !taa.enabled() && !fsr_ok
/// ```
///
/// FSR defaults **on**, so that expression is false in the shipped
/// configuration and **FXAA has never run by default** — while presenting a
/// checked box in Details claiming otherwise. Three booleans describe eight
/// states of which only five are reachable and one is a lie. This enum is the
/// five reachable states and nothing else, which is the whole reason it exists:
/// a value the user sets is a value that takes effect.
///
/// Order is the quality/cost ladder, and [`Self::as_index`] pins it to the
/// generated Details row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AntiAliasing {
    /// No resolve. The visibility buffer has no MSAA, so this is hard-aliased
    /// and is here as the honest reference point for the others.
    Off,
    /// Timothy Lottes' FXAA 3.11, one LDR pass. Cheapest; softens text-adjacent
    /// detail because it cannot tell an edge from a glyph.
    Fxaa,
    /// SMAA 1x — morphological, spatial only (MORROWIND-AC).
    ///
    /// Three LDR passes. Better edge reconstruction than FXAA and markedly less
    /// texture softening, because blend weights come from a reconstructed edge
    /// geometry rather than from a luma gradient.
    Smaa1x,
    /// SMAA T2x — SMAA 1x plus the existing temporal resolve.
    ///
    /// Two-sample subpixel jitter reusing 24F's `jitter_ndc` and 24AD's
    /// velocity buffer, with SMAA 1x run on each resolved frame. This is the
    /// highest-quality *non-upscaling* option.
    SmaaT2x,
    /// Somnium's own TAA (Phase 24F), no morphological pass.
    Taa,
    /// AMD FSR 3 temporal reconstruction to the window (Phase VV). Default.
    ///
    /// Also the upscaler, so it is not merely an AA choice: selecting anything
    /// else means the viewport Resolution preset is blitted rather than
    /// reconstructed.
    #[default]
    Fsr,
}

impl AntiAliasing {
    /// Stable index for serialization and the generated Details row.
    #[must_use]
    pub fn as_index(self) -> u32 {
        match self {
            Self::Off => 0,
            Self::Fxaa => 1,
            Self::Smaa1x => 2,
            Self::SmaaT2x => 3,
            Self::Taa => 4,
            Self::Fsr => 5,
        }
    }

    /// Whether this mode runs an SMAA morphological pass.
    #[must_use]
    pub fn uses_smaa(self) -> bool {
        matches!(self, Self::Smaa1x | Self::SmaaT2x)
    }

    /// Whether this mode needs a temporal history and a jittered projection.
    ///
    /// `SmaaT2x` is in here and `Smaa1x` is not: that difference *is* the
    /// difference between them.
    #[must_use]
    pub fn uses_temporal(self) -> bool {
        matches!(self, Self::SmaaT2x | Self::Taa)
    }
}

/// SMAA quality preset (MORROWIND-AC).
///
/// The four presets of Jimenez et al.'s original formulation, which trade edge
/// detection sensitivity and search distance against cost. They are exposed
/// because SMAA's useful range is genuinely wide — `Low` is close to FXAA's
/// cost, `Ultra` finds edges FXAA cannot see at all.
///
/// **What is deliberately absent: SMAA S2x and SMAA 4x.** Both require
/// multisample coverage — S2x resolves two MSAA subsamples, and 4x is S2x plus
/// T2x. Somnium shades from a visibility buffer
/// (`renderer.rs`, `pass/visibility.rs`), which stores one triangle
/// per pixel and has no subsample coverage to resolve; every render target in
/// the frame is created with `sample_count: 1`. Offering either would be a
/// control that cannot work, which is the exact defect [`AntiAliasing`] exists
/// to remove. They are named here so the absence reads as a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SmaaPreset {
    /// Threshold 0.15, 4 search steps. Roughly FXAA's cost.
    Low,
    /// Threshold 0.10, 8 search steps.
    Medium,
    /// Threshold 0.10, 16 search steps.
    High,
    /// Threshold 0.05, 32 search steps. The default.
    #[default]
    Ultra,
}

impl SmaaPreset {
    /// Stable index for serialization and the generated Details row.
    #[must_use]
    pub fn as_index(self) -> u32 {
        match self {
            Self::Low => 0,
            Self::Medium => 1,
            Self::High => 2,
            Self::Ultra => 3,
        }
    }

    /// Relative luma contrast that marks a pixel as an edge.
    #[must_use]
    pub fn threshold(self) -> f32 {
        match self {
            Self::Low => 0.15,
            Self::Medium | Self::High => 0.10,
            Self::Ultra => 0.05,
        }
    }

    /// How far along an edge the search walks, in pixels.
    #[must_use]
    pub fn max_search_steps(self) -> u32 {
        match self {
            Self::Low => 4,
            Self::Medium => 8,
            Self::High => 16,
            Self::Ultra => 32,
        }
    }
}

/// Tone-mapping curve applied at the end of the frame (Phase 24B).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tonemapper {
    /// Troy Sobotka's AgX. Handles very bright saturated light without the hue
    /// shift ACES shows, which matters once a 100 000 lux sun is in frame.
    #[default]
    AgX,
    /// Narkowicz's ACES fit — what the engine used before Phase 24B.
    Aces,
    /// Plain Reinhard. Mostly useful as a reference point when comparing.
    Reinhard,
}

impl Tonemapper {
    /// Stable index for the GPU uniform. Must match `postprocess.wgsl`.
    #[must_use]
    pub fn as_index(self) -> u32 {
        match self {
            Self::AgX => 0,
            Self::Aces => 1,
            Self::Reinhard => 2,
        }
    }

    /// Cycle order for the inspector's tonemapper button.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::AgX => Self::Aces,
            Self::Aces => Self::Reinhard,
            Self::Reinhard => Self::AgX,
        }
    }

    /// Human-readable name used by the editor inspector.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::AgX => "AgX",
            Self::Aces => "ACES",
            Self::Reinhard => "Reinhard",
        }
    }
}

impl PostProcessComponent {
    // ── Derived AA state (MORROWIND-AC) ─────────────────────────────────────
    //
    // These replace three authored booleans. Nothing outside this block decides
    // which passes run together: the mutual exclusions that used to live in
    // `set_taa_enabled` / `set_fsr_enabled` are now consequences of the enum
    // having one value, so there is no combination to keep consistent and no
    // setter that can leave the component in a state Details cannot show.

    /// Whether the FXAA pass runs.
    #[must_use]
    pub fn fxaa_enabled(&self) -> bool {
        self.aa == AntiAliasing::Fxaa
    }

    /// Whether an SMAA morphological pass runs, and at which preset.
    #[must_use]
    pub fn smaa_enabled(&self) -> bool {
        self.aa.uses_smaa()
    }

    /// Whether Somnium's own temporal resolve runs.
    ///
    /// True for `SmaaT2x` as well as `Taa`: T2x *is* SMAA 1x over a temporally
    /// resolved image, and it reuses 24F's history rather than growing a
    /// second one.
    #[must_use]
    pub fn taa_enabled(&self) -> bool {
        self.aa.uses_temporal()
    }

    /// Whether FSR 3 reconstruction runs.
    #[must_use]
    pub fn fsr_enabled(&self) -> bool {
        self.aa == AntiAliasing::Fsr
    }

    /// Enable standalone CAS.
    ///
    /// FSR's RCAS already sharpens, so asking for CAS while FSR is selected
    /// drops to the next non-upscaling rung rather than stacking two
    /// sharpeners — the same intent the old boolean version had.
    pub fn set_cas_enabled(&mut self, enabled: bool) {
        self.cas_enabled = enabled;
        if enabled && self.aa == AntiAliasing::Fsr {
            self.aa = AntiAliasing::Taa;
        }
    }

    /// Enable the volumetric owner. Disabling it also makes the dependent
    /// shaft checkbox truthful instead of leaving a checked no-op behind.
    pub fn set_volumetrics_enabled(&mut self, enabled: bool) {
        self.volumetrics_enabled = enabled;
        if !enabled {
            self.light_shafts = false;
        }
    }

    /// Enable shadowed in-scatter. Shafts are part of the volumetric pass, so
    /// asking for them also enables their owner.
    pub fn set_light_shafts_enabled(&mut self, enabled: bool) {
        self.light_shafts = enabled;
        if enabled {
            self.volumetrics_enabled = true;
        }
    }

    /// The world cache owns the shared 3-D volume while active.
    pub fn set_world_cache_enabled(&mut self, enabled: bool) {
        self.world_cache = enabled;
        if enabled {
            self.mesh_sdf = false;
        }
    }

    /// Mesh SDF stores distance in the same volume alpha used by the cache.
    pub fn set_mesh_sdf_enabled(&mut self, enabled: bool) {
        self.mesh_sdf = enabled;
        if enabled {
            self.world_cache = false;
        }
    }

    /// The EV100 actually used this frame, before auto-exposure metering.
    #[must_use]
    pub fn manual_ev100(&self) -> f32 {
        let base = if self.use_physical_camera {
            light_units::ev100_from_camera(
                self.aperture_f_stops,
                self.shutter_speed_s,
                self.sensitivity_iso,
            )
        } else {
            self.ev100
        };
        base - self.exposure_compensation
    }

    /// Linear multiplier from scene luminance to display range.
    #[must_use]
    pub fn exposure_multiplier(&self) -> f32 {
        light_units::exposure_from_ev100(self.manual_ev100())
    }

    /// Vignette strength to send to the renderer (0 when disabled).
    pub fn effective_vignette(&self) -> f32 {
        if self.vignette_enabled {
            self.vignette_strength.max(0.0)
        } else {
            0.0
        }
    }

    /// Chromatic-aberration strength to send to the renderer (0 when disabled).
    pub fn effective_ca(&self) -> f32 {
        if self.ca_enabled {
            self.ca_strength.max(0.0)
        } else {
            0.0
        }
    }
}
impl somnium_ecs::Component for PostProcessComponent {}

#[cfg(test)]
mod post_process_tests {
    use super::{AntiAliasing, PostProcessComponent, SmaaPreset};

    /// MORROWIND-AC. Three tests used to live here proving that
    /// `set_taa_enabled` / `set_fsr_enabled` / `set_cas_enabled` restored a
    /// mutual exclusion between three booleans. There is one value now, so the
    /// exclusion is not something that can be violated and then repaired — it
    /// is arithmetic. What is worth asserting instead is that every mode
    /// resolves to exactly one active resolve.
    #[test]
    fn exactly_one_resolve_is_active_in_every_mode() {
        for (mode, fxaa, smaa, temporal, fsr) in [
            (AntiAliasing::Off, false, false, false, false),
            (AntiAliasing::Fxaa, true, false, false, false),
            (AntiAliasing::Smaa1x, false, true, false, false),
            (AntiAliasing::SmaaT2x, false, true, true, false),
            (AntiAliasing::Taa, false, false, true, false),
            (AntiAliasing::Fsr, false, false, false, true),
        ] {
            let pp = PostProcessComponent {
                aa: mode,
                ..Default::default()
            };
            assert_eq!(pp.fxaa_enabled(), fxaa, "{mode:?} fxaa");
            assert_eq!(pp.smaa_enabled(), smaa, "{mode:?} smaa");
            assert_eq!(pp.taa_enabled(), temporal, "{mode:?} temporal");
            assert_eq!(pp.fsr_enabled(), fsr, "{mode:?} fsr");
        }
    }

    /// The defect this sub-phase exists to remove, pinned so it cannot return.
    ///
    /// `fxaa_enabled` and `fsr_enabled` both defaulted to `true`, and
    /// `renderer.rs` resolved that with `fxaa_enabled && !taa && !fsr_ok` — so
    /// the shipped default showed a checked FXAA box that never ran a pass.
    #[test]
    fn the_default_does_not_claim_an_anti_aliasing_that_never_runs() {
        let pp = PostProcessComponent::default();
        let claimed = [
            pp.fxaa_enabled(),
            pp.smaa_enabled(),
            pp.taa_enabled(),
            pp.fsr_enabled(),
        ];
        assert_eq!(
            claimed.iter().filter(|on| **on).count(),
            1,
            "exactly one resolve may be claimed, got {claimed:?}"
        );
    }

    #[test]
    fn enabling_cas_steps_off_fsr_rather_than_stacking_two_sharpeners() {
        let mut pp = PostProcessComponent {
            aa: AntiAliasing::Fsr,
            ..Default::default()
        };
        pp.set_cas_enabled(true);
        assert!(pp.cas_enabled);
        assert!(!pp.fsr_enabled(), "FSR RCAS and CAS must not both run");
        assert!(
            pp.taa_enabled(),
            "the step-down must keep a temporal resolve"
        );
    }

    #[test]
    fn smaa_presets_are_ordered_by_cost() {
        let presets = [
            SmaaPreset::Low,
            SmaaPreset::Medium,
            SmaaPreset::High,
            SmaaPreset::Ultra,
        ];
        for pair in presets.windows(2) {
            assert!(pair[0].max_search_steps() < pair[1].max_search_steps());
            assert!(pair[0].threshold() >= pair[1].threshold());
        }
    }

    /// Every index the Details combo can produce must round-trip, or a saved
    /// scene silently reopens on a different mode.
    #[test]
    fn every_authored_index_round_trips() {
        for mode in [
            AntiAliasing::Off,
            AntiAliasing::Fxaa,
            AntiAliasing::Smaa1x,
            AntiAliasing::SmaaT2x,
            AntiAliasing::Taa,
            AntiAliasing::Fsr,
        ] {
            assert_eq!(mode.as_index() as usize, mode as usize);
        }
    }

    #[test]
    fn shafts_and_volumetrics_cannot_form_a_checked_noop() {
        let mut pp = PostProcessComponent::default();
        pp.set_volumetrics_enabled(false);
        assert!(!pp.volumetrics_enabled && !pp.light_shafts);
        pp.set_light_shafts_enabled(true);
        assert!(pp.volumetrics_enabled && pp.light_shafts);
    }

    #[test]
    fn cache_and_mesh_sdf_cannot_claim_the_shared_volume_together() {
        let mut pp = PostProcessComponent::default();
        pp.set_world_cache_enabled(true);
        pp.set_mesh_sdf_enabled(true);
        assert!(pp.mesh_sdf && !pp.world_cache);
        pp.set_world_cache_enabled(true);
        assert!(pp.world_cache && !pp.mesh_sdf);
    }
}

// ─── Phase CR: Camera settings ──────────────────────────────────────────────

/// Scene-wide camera settings, exposed as a selectable "Camera" entity.
///
/// Play possesses this entity's **world** transform (so a later player-parented
/// camera still drives the view). Physical Camera is a Post FX exposure
/// triangle. Frustum Cull is the CPU early-out (Phase CR-B), independent of
/// GPU 15B on F10.
///
/// Local `-Z` is the look direction, same as lights.
// `Eq` dropped when DOOM-F added float fields: `f32` has no total equality, and
// a derived `Eq` on a struct holding one is a lie about NaN whatever the
// compiler would let through.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraSettingsComponent {
    /// Skip terrain chunks whose AABB misses the camera frustum before they
    /// reach `draw_queue`. Default on. Off-screen casters still shadow into
    /// view. `SOMNIUM_CPU_FRUSTUM=0` forces this off.
    pub frustum_cull: bool,
    /// Phase DOOM-F. Let the renderer lower the internal 3D resolution to hold
    /// a frame budget. **Off by default and deliberately so.**
    ///
    /// This is the only control in Phase DOOM that trades image quality for
    /// speed, so it is something the user turns on with the floor in front of
    /// them, not something the engine starts doing when a frame gets expensive.
    pub dynamic_resolution: bool,
    /// Frame time the controller aims at, in milliseconds. 16.67 is 60 Hz.
    pub dynamic_target_ms: f32,
    /// Lowest scale it may choose, as a fraction of the preset resolution.
    /// 0.67 renders about 45% of the pixels.
    pub dynamic_floor: f32,
}

impl Default for CameraSettingsComponent {
    fn default() -> Self {
        Self {
            frustum_cull: true,
            dynamic_resolution: false,
            dynamic_target_ms: 1000.0 / 60.0,
            dynamic_floor: 0.67,
        }
    }
}

impl CameraSettingsComponent {
    /// Honour `SOMNIUM_CPU_FRUSTUM=0` and the Phase DOOM-F overrides at spawn.
    #[must_use]
    pub fn from_env() -> Self {
        let num = |key: &str, default: f32| -> f32 {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &f32| v.is_finite() && *v > 0.0)
                .unwrap_or(default)
        };
        let default = Self::default();
        Self {
            frustum_cull: std::env::var("SOMNIUM_CPU_FRUSTUM").as_deref() != Ok("0"),
            // `SOMNIUM_DYNRES=1` exists so a timing run can measure the
            // controller without a human clicking a checkbox first.
            dynamic_resolution: std::env::var("SOMNIUM_DYNRES").as_deref() == Ok("1"),
            dynamic_target_ms: num("SOMNIUM_DYNRES_TARGET_MS", default.dynamic_target_ms),
            dynamic_floor: num("SOMNIUM_DYNRES_FLOOR", default.dynamic_floor).clamp(0.25, 1.0),
        }
    }
}

impl somnium_ecs::Component for CameraSettingsComponent {}

/// View matrix and eye position from a camera entity's world matrix.
///
/// The entity looks along local `-Z`. Parenting the Camera to a player later
/// is just `propagate_transforms` writing a new [`WorldTransform`].
#[must_use]
pub fn camera_view_from_world(world: glam::Mat4) -> (glam::Mat4, glam::Vec3) {
    let pos = world.transform_point3(glam::Vec3::ZERO);
    (world.inverse(), pos)
}

/// Rotation that aims local `-Z` along `forward` (Y-up).
#[must_use]
pub fn look_rotation_neg_z(forward: glam::Vec3) -> glam::Quat {
    let forward = forward.normalize_or_zero();
    if forward.length_squared() < 1e-8 {
        return glam::Quat::IDENTITY;
    }
    let view = glam::Mat4::look_at_rh(glam::Vec3::ZERO, forward, glam::Vec3::Y);
    glam::Quat::from_mat4(&view.inverse())
}

#[cfg(test)]
mod camera_view_tests {
    use super::{camera_view_from_world, look_rotation_neg_z};

    #[test]
    fn identity_camera_looks_along_neg_z() {
        let (view, pos) = camera_view_from_world(glam::Mat4::IDENTITY);
        assert!(pos.length() < 1e-5);
        let ahead = view.transform_point3(glam::Vec3::NEG_Z);
        assert!(ahead.z < 0.0, "RH view: a point along look is in front");
    }

    #[test]
    fn translated_identity_matches_look_at() {
        let pos = glam::Vec3::new(0.0, 2.0, 8.0);
        let world = glam::Mat4::from_translation(pos);
        let (view, eye) = camera_view_from_world(world);
        assert!((eye - pos).length() < 1e-5);
        let expected = glam::Mat4::look_at_rh(pos, pos + glam::Vec3::NEG_Z, glam::Vec3::Y);
        let d = (view - expected).abs();
        assert!(d.to_cols_array().iter().all(|x| *x < 1e-4));
    }

    #[test]
    fn look_rotation_identity_when_facing_neg_z() {
        let q = look_rotation_neg_z(glam::Vec3::NEG_Z);
        assert!(q.dot(glam::Quat::IDENTITY).abs() > 0.999);
    }
}

// ─── Phase 11.5A: Scene Graph Components ──────────────────────────────────

/// ECS component that marks an entity as a child of another entity.
///
/// The parent's world transform is pre-multiplied with this entity's `Transform`
/// by the transform propagation system. The resulting world matrix is stored in
/// `WorldTransform` and used by the renderer instead of `Transform::to_matrix()`.
#[derive(Debug, Clone, Copy)]
pub struct Parent {
    /// The parent entity (`Entity::DANGLING` means "root / no parent").
    pub entity: somnium_ecs::Entity,
}
impl somnium_ecs::Component for Parent {}

/// ECS component that stores an entity's ordered list of child entities.
///
/// Fixed-size (16 children) so the component satisfies the ECS `Copy` constraint.
/// Hierarchies deeper than 16 siblings can be built by chaining through multiple levels.
#[derive(Debug, Clone, Copy)]
pub struct Children {
    /// Fixed-capacity child slots; only the first `count` are valid.
    pub entities: [somnium_ecs::Entity; 16],
    /// Number of valid entries in `entities`.
    pub count: u8,
}

impl Children {
    /// An empty child list.
    pub fn empty() -> Self {
        Self {
            entities: [somnium_ecs::Entity::DANGLING; 16],
            count: 0,
        }
    }

    /// The valid child entities as a slice.
    pub fn as_slice(&self) -> &[somnium_ecs::Entity] {
        &self.entities[..self.count as usize]
    }

    /// Append a child; returns `false` if the 16-slot capacity is full.
    pub fn push(&mut self, child: somnium_ecs::Entity) -> bool {
        if self.count as usize >= 16 {
            return false;
        }
        self.entities[self.count as usize] = child;
        self.count += 1;
        true
    }

    /// Remove a child if present, preserving order.
    pub fn remove(&mut self, child: somnium_ecs::Entity) {
        if let Some(pos) = self.as_slice().iter().position(|&e| e == child) {
            self.entities[pos..self.count as usize].rotate_left(1);
            self.count -= 1;
        }
    }
}
impl somnium_ecs::Component for Children {}

/// ECS component that stores the final world-space transform matrix for an entity.
///
/// Computed each frame by the transform propagation system:
/// - Root entities: `WorldTransform = Transform::to_matrix()`
/// - Child entities: `WorldTransform = parent_world * Transform::to_matrix()`
///
/// The renderer reads `WorldTransform` instead of calling `Transform::to_matrix()` directly.
#[derive(Debug, Clone, Copy)]
pub struct WorldTransform(pub glam::Mat4);

impl WorldTransform {
    /// A `WorldTransform` holding the identity matrix.
    pub fn identity() -> Self {
        Self(glam::Mat4::IDENTITY)
    }
}
impl somnium_ecs::Component for WorldTransform {}

// ─── Phase 11.5A-2: Transform Propagation System ──────────────────────────

/// Propagates parent-child transform hierarchies, writing `WorldTransform` for
/// every entity that has a `Transform` component.
///
/// Run this in `on_update` after physics sync and before rendering. Entities
/// without a `Parent` component are treated as roots (parent = identity).
///
/// **Requires:** All spawned entities must include a `WorldTransform::identity()`
/// component so the propagation system can write to them.
///
/// Algorithm: BFS starting from root entities (Transform + no Parent). For each
/// root, `world_mat = Transform::to_matrix()`. For each child, `child_world =
/// parent_world * local_transform.to_matrix()`.
pub fn propagate_transforms(world: &mut World) {
    use somnium_ecs::ComponentId;

    let t_id = ComponentId::of::<Transform>();

    // Phase 1 — collect all entities with Transform, regardless of Parent
    // component, then determine roots by checking Parent contents at runtime.
    // This correctly handles `Parent { entity: DANGLING }` as a root.
    let t_req = ComponentSet::from_ids(vec![t_id]);
    let mut all_entities: Vec<(Entity, glam::Mat4)> = Vec::new();
    for arch in world.query_archetypes(&t_req, &ComponentSet::empty()) {
        let t_col = arch.column_index(t_id).unwrap();
        for row in 0..arch.len() {
            let entity = arch.entities()[row];
            let t = unsafe { arch.column(t_col).get::<Transform>(row) };
            all_entities.push((entity, t.to_matrix()));
        }
    }

    // Phase 1b — seed the BFS stack with roots:
    // root = no Parent component, OR Parent.entity is DANGLING, OR parent is dead.
    let mut stack: Vec<(Entity, glam::Mat4)> = Vec::new();
    for &(entity, local_mat) in &all_entities {
        let is_root = match world.get::<Parent>(entity) {
            None => true,
            Some(p) => p.entity == Entity::DANGLING || !world.is_alive(p.entity),
        };
        if is_root {
            stack.push((entity, local_mat));
        }
    }

    // Phase 1c — BFS: accumulate world matrices for all children.
    let mut i = 0;
    while i < stack.len() {
        let (entity, world_mat) = stack[i];
        i += 1;
        if let Some(children) = world.get::<Children>(entity) {
            let children_copy = *children;
            for &child in children_copy.as_slice() {
                if world.is_alive(child) {
                    if let Some(local_t) = world.get::<Transform>(child) {
                        stack.push((child, world_mat * local_t.to_matrix()));
                    }
                }
            }
        }
    }

    // Phase 2 — write WorldTransform. All immutable borrows from phase 1 are
    // released here; &mut self borrows are safe.
    for (entity, world_mat) in stack {
        let _ = world
            .get_mut::<WorldTransform>(entity)
            .map(|wt| wt.0 = world_mat);
    }
}

// ─── Phase 11.5J: GPU Particle System ────────────────────────────────────────

/// Per-particle runtime state (CPU-side).
#[derive(Debug, Clone, Copy)]
pub struct ParticleState {
    /// World-space position.
    pub position: glam::Vec3,
    /// World-space velocity (m/s).
    pub velocity: glam::Vec3,
    /// Current age in seconds (0 = just born).
    pub age: f32,
    /// Total lifetime in seconds.
    pub lifetime: f32,
}

/// ECS component that drives a GPU particle emitter.
///
/// Add this component to an entity together with `Transform` and `WorldTransform`.
/// The engine simulates particles each frame and uploads the results to the
/// `ParticlePass` for instanced billboard rendering.
#[derive(Debug, Clone)]
pub struct ParticleEmitter {
    // ── Emitter parameters ────────────────────────────────────────────────────
    /// Maximum number of live particles at once.
    pub max_particles: u32,
    /// New particles spawned per second.
    pub spawn_rate: f32,
    /// Each particle's lifetime in seconds.
    pub lifetime: f32,
    /// Initial speed in m/s (direction is randomized within `spread_angle`).
    pub initial_speed: f32,
    /// Cone half-angle (radians) for direction randomization (0 = straight up).
    pub spread_angle: f32,
    /// Particle size at birth (metres, billboard half-width).
    pub size_start: f32,
    /// Particle size at end of life.
    pub size_end: f32,
    /// CONTROL-K: linear RGBA over the particle's life, `0` at birth and `1`
    /// at death.
    ///
    /// Replaces the `color_start`/`color_end` pair, which could express a
    /// straight line between two colours and nothing else — no flash, no
    /// fade-in-then-out, no hold. A two-stop ramp reproduces the old pair
    /// exactly, so the default is that pair.
    pub color_over_life: somnium_ecs::curve::Gradient,
    /// Downward gravity acceleration (m/s²).
    pub gravity: f32,
    /// Phase CONTROL-N: a constant velocity added to every particle at birth.
    ///
    /// The cone spawn is right for a fountain and useless for rain, which
    /// falls in one direction and is *sheared* by wind. One vector turns the
    /// same emitter into both, which is the plan's "precipitation through the
    /// existing particle emitter" rather than a second particle system.
    pub velocity_bias: [f32; 3],
    /// Phase CONTROL-N: half-extents of a box particles spawn in, around the
    /// emitter's origin.
    ///
    /// Zero is the point emitter every existing scene has. Non-zero makes the
    /// emitter a volume, which is what rain needs: a camera-anchored box
    /// overhead, so precipitation exists where the player is and nowhere else.
    pub spawn_extents: [f32; 3],

    // ── Runtime state (not user-facing) ──────────────────────────────────────
    /// Live particles owned by this emitter.
    pub particles: Vec<ParticleState>,
    /// Fractional carry-over for sub-frame spawning.
    pub spawn_accum: f32,
}

impl Default for ParticleEmitter {
    fn default() -> Self {
        Self {
            max_particles: 1000,
            spawn_rate: 100.0,
            lifetime: 3.0,
            initial_speed: 5.0,
            spread_angle: 0.8,
            size_start: 1.0,
            size_end: 0.2,
            color_over_life: somnium_ecs::curve::Gradient::ramp(
                [1.0, 0.4, 0.1, 1.0],
                [0.2, 0.0, 0.0, 0.0],
            ),
            velocity_bias: [0.0; 3],
            spawn_extents: [0.0; 3],
            gravity: 1.0,
            particles: Vec::new(),
            spawn_accum: 0.0,
        }
    }
}
impl somnium_ecs::Component for ParticleEmitter {}

/// Simulate all particle emitters and return a flat list of GPU instances.
///
/// Call each frame in `about_to_wait` after physics and before `render()`.
/// `seed` increments each frame (used for deterministic pseudo-random spawn direction).
pub fn simulate_particles(
    world: &mut somnium_ecs::World,
    dt: f32,
    frame: u64,
) -> Vec<somnium_renderer::pass::particle::GpuParticle> {
    use somnium_renderer::pass::particle::GpuParticle;

    let mut gpu_particles = Vec::new();

    let emitter_entities: Vec<somnium_ecs::Entity> = world
        .entities()
        .filter(|e| world.get::<ParticleEmitter>(*e).is_some())
        .collect();

    for entity in emitter_entities {
        // Borrow world piecemeal to satisfy the borrow checker.
        let origin = world
            .get::<WorldTransform>(entity)
            .map(|wt| glam::Vec3::new(wt.0.w_axis.x, wt.0.w_axis.y, wt.0.w_axis.z))
            .or_else(|| world.get::<Transform>(entity).map(|t| t.translation))
            .unwrap_or(glam::Vec3::ZERO);

        let Some(emitter) = world.get_mut::<ParticleEmitter>(entity) else {
            continue;
        };

        // ── 1. Advance existing particles ─────────────────────────────────────
        let gravity = emitter.gravity;
        emitter.particles.retain_mut(|p| {
            p.age += dt;
            p.velocity.y -= gravity * dt;
            p.position += p.velocity * dt;
            p.age < p.lifetime
        });

        // ── 2. Spawn new particles ────────────────────────────────────────────
        emitter.spawn_accum += emitter.spawn_rate * dt;
        let to_spawn = emitter.spawn_accum.floor() as u32;
        emitter.spawn_accum -= to_spawn as f32;
        let available = emitter
            .max_particles
            .saturating_sub(emitter.particles.len() as u32);
        let count = to_spawn.min(available);

        let speed = emitter.initial_speed;
        let spread = emitter.spread_angle;
        let lifetime = emitter.lifetime;

        for i in 0..count {
            // Deterministic LCG pseudo-random — good enough for particles.
            let seed = frame
                .wrapping_mul(1_000_003)
                .wrapping_add((i as u64).wrapping_mul(6_364_136_223_846_793_005));
            let r1 = ((seed >> 33) & 0xFFFF) as f32 / 65535.0; // 0..1
            let r2 = ((seed >> 17) & 0xFFFF) as f32 / 65535.0 * 2.0 * std::f32::consts::PI;
            let theta = r1 * spread;
            let dir = glam::Vec3::new(theta.sin() * r2.cos(), theta.cos(), theta.sin() * r2.sin());
            // CONTROL-N: a spawn volume and a constant velocity, both zero for
            // every emitter authored before this existed.
            let extents = glam::Vec3::from(emitter.spawn_extents);
            let jitter = if extents == glam::Vec3::ZERO {
                glam::Vec3::ZERO
            } else {
                let r3 = ((seed >> 5) & 0xFFFF) as f32 / 65535.0;
                let r4 = ((seed >> 41) & 0xFFFF) as f32 / 65535.0;
                let r5 = ((seed >> 23) & 0xFFFF) as f32 / 65535.0;
                (glam::Vec3::new(r3, r4, r5) * 2.0 - glam::Vec3::ONE) * extents
            };
            emitter.particles.push(ParticleState {
                position: origin + jitter,
                velocity: dir * speed + glam::Vec3::from(emitter.velocity_bias),
                age: 0.0,
                lifetime,
            });
        }

        // ── 3. Emit GPU instances ─────────────────────────────────────────────
        let size_start = emitter.size_start;
        let size_end = emitter.size_end;
        // CONTROL-K: the ramp is sampled per particle rather than baked into a
        // table, because an emitter's particle count is the small number here
        // and a table would need invalidating whenever the ramp was edited —
        // which is every frame of a drag.
        let ramp = &emitter.color_over_life;

        for p in &emitter.particles {
            let frac = (p.age / p.lifetime).clamp(0.0, 1.0);
            let size = size_start + (size_end - size_start) * frac;
            let color = ramp.evaluate(frac);
            gpu_particles.push(GpuParticle {
                position: p.position.to_array(),
                size,
                color,
            });
        }
    }

    gpu_particles
}

// ── Water Component ─────────────────────────────────────────────────────────

/// Configuration for the procedural water shader (Phase 13).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterComponent {
    /// Stable handle into renderer-owned `WaterBodyData`.
    pub water_id: u32,
    /// Renderer terrain whose local space and bathymetry this body follows.
    pub terrain_id: u32,
    /// 0 = legacy/unset, 1 = baked Great Lakes lake, 2 = full-coverage ocean.
    pub preset: u32,
    /// 0 = lake. Reserved for ocean and river body types.
    pub body_kind: u32,
    /// Surface datum in the parent terrain's local metres.
    pub surface_level: f32,
    /// Deepest baked point below the datum.
    pub max_depth: f32,
    /// Terrain-local `[min_x, min_z, max_x, max_z]` coverage bounds.
    pub bounds: [f32; 4],
    /// Rendering/gameplay enable flag stored as a scalar for scene stability.
    pub enabled: bool,
    /// Deep water color.
    pub deep_color: [f32; 4],
    /// Shallow water color (near edges/shore).
    pub shallow_color: [f32; 4],
    /// Foam/edge highlight color.
    pub edge_color: [f32; 4],
    /// Clarity factor (higher = clearer).
    pub clarity: f32,
    /// Distance factor for the edge intersection.
    pub edge_scale: f32,
    /// Wave amplitude multiplier.
    pub amplitude: f32,
    /// UV/Coordinate scale.
    pub coord_scale: [f32; 2],
    /// UV/Coordinate offset.
    pub coord_offset: [f32; 2],
    /// Direction of primary waves.
    pub wave_dir_a: [f32; 2],
    /// Direction of secondary waves.
    pub wave_dir_b: [f32; 2],
    /// Wave blending factor.
    pub wave_blend: f32,
    /// Dominant Gerstner wavelength in metres.
    pub wave_length_a: f32,
    /// Secondary Gerstner wavelength in metres.
    pub wave_length_b: f32,
    /// Multiplier on deep-water dispersion speed.
    pub wave_speed: f32,
    /// Horizontal Gerstner displacement, clamped below breaking.
    pub wave_steepness: f32,
    /// RGB Beer-Lambert absorption coefficients in inverse metres.
    pub absorption: [f32; 3],
    /// RGB single-scattering coefficients in inverse metres.
    pub scattering: [f32; 3],
    /// Water microfacet roughness.
    pub roughness: f32,
    /// Henyey-Greenstein forward-scattering asymmetry.
    pub anisotropy: f32,
    /// Screen-space reflection contribution before environment fallback.
    pub ssr_strength: f32,
    /// Ray-traced reflection mix when Halcyon is running (Phase VV).
    pub rt_reflect_strength: f32,
    /// 0 off, 1 SSR hit/miss, 2 reflection-source colouring (Phase VV-A).
    pub reflect_debug: f32,
    /// Blend from deterministic Gerstner (0) to the two-cascade spectral tier (1).
    pub spectrum_blend: f32,
    /// Authored wind speed in metres per second. Scales the spectral cascade
    /// roster (Wind = 10 leaves the design speeds untouched).
    pub wind_speed: f32,
    /// Crest foam persistence control forwarded to the cascade foam amount
    /// (0–10). The inspector labels this Foam.
    pub foam_decay: f32,
    /// Jacobian whitecap threshold forwarded to every foam-bearing cascade.
    pub foam_threshold: f32,
    /// Multiplier for underwater projected caustics.
    pub caustic_strength: f32,
    /// Whether this body participates in the underwater medium pass.
    pub underwater_enabled: bool,
}

impl Default for WaterComponent {
    fn default() -> Self {
        Self {
            water_id: u32::MAX,
            terrain_id: u32::MAX,
            preset: 0,
            body_kind: 0,
            surface_level: 0.0,
            max_depth: 0.0,
            bounds: [-10.0, -10.0, 10.0, 10.0],
            enabled: true,
            deep_color: [0.008, 0.035, 0.075, 0.9],
            shallow_color: [0.06, 0.28, 0.42, 0.5],
            edge_color: [0.8, 0.9, 1.0, 1.0],
            clarity: 0.1,
            edge_scale: 1.0,
            amplitude: 1.0,
            coord_scale: [1.0, 1.0],
            coord_offset: [0.0, 0.0],
            wave_dir_a: [1.0, 0.0],
            wave_dir_b: [0.0, 1.0],
            wave_blend: 0.5,
            wave_length_a: 18.0,
            wave_length_b: 11.0,
            wave_speed: 1.0,
            wave_steepness: 0.35,
            absorption: [0.18, 0.055, 0.025],
            scattering: [0.012, 0.035, 0.055],
            roughness: 0.65,
            anisotropy: 0.35,
            ssr_strength: 0.85,
            rt_reflect_strength: 1.0,
            reflect_debug: 0.0,
            spectrum_blend: 0.75,
            wind_speed: 8.0,
            foam_decay: 0.9,
            foam_threshold: 0.08,
            caustic_strength: 1.0,
            underwater_enabled: true,
        }
    }
}

impl WaterComponent {
    /// The authored Phase IV Great Lakes body paired with a 1024 m terrain.
    pub fn great_lakes(water_id: u32, terrain_id: u32, bounds: [f32; 4]) -> Self {
        Self {
            water_id,
            terrain_id,
            preset: 1,
            body_kind: 0,
            surface_level: somnium_renderer::terrain::DEFAULT_WATER_LEVEL_METRES,
            // Deliberately deeper than the baked bed
            // ([`somnium_renderer::terrain::DEFAULT_WATER_DEPTH_METRES`]). This
            // is the optical path the extinction integral walks where nothing
            // opaque lies behind the surface, so the extra six metres is what
            // carries open water into full absorption instead of leaving it
            // thin and grey out towards the horizon.
            max_depth: 18.6,
            bounds,
            // Under a metre of swell for an inland body. This scales rendered
            // displacement but not the cascade Jacobian, so foam still forms
            // for the full-strength sea; the mismatch is deliberate and reads
            // as crests that break slightly early. Pushing it much past one
            // buries the surface in white.
            amplitude: 0.57,
            clarity: 1.0,
            coord_scale: [1.0, 1.0],
            wave_dir_a: [0.944, 0.330],
            wave_dir_b: [-0.243, 0.970],
            wave_length_a: 18.0,
            wave_length_b: 11.0,
            // Authored Speed in the inspector was dialled to 0.2 because the FFT
            // still carried the surface and the Gerstner layer barely showed.
            // Buoyancy samples only that Gerstner layer — the spectral cascade
            // is visual-only on the CPU — so 0.2 froze the boat while the water
            // kept moving. 0.85 is the value the vessel was tuned against.
            wave_speed: 0.85,
            wave_steepness: 0.42,
            edge_color: [0.88, 0.96, 1.0, 1.0],
            edge_scale: 1.35,
            absorption: [0.22, 0.070, 0.032],
            scattering: [0.016, 0.045, 0.065],
            // Phase IV-K, authored against the shipped scene rather than taken
            // from the reference. A near-mirror microfacet distribution is what
            // gives the sun a tight, glittering track across the swell instead
            // of a broad sheen; the sky reflection stays soft regardless,
            // because the shader blurs that with its own separate roughness.
            roughness: 0.02,
            anisotropy: 0.45,
            ssr_strength: 1.0,
            rt_reflect_strength: 1.0,
            spectrum_blend: 0.64,
            // Wind = 10 leaves the cascade roster at its design speeds; 6.5 is
            // a calmer inland lake. Foam and Whitecap drive the spectral foam
            // grow/decay and Jacobian threshold (see WaterSpectrumPass::record).
            wind_speed: 6.5,
            foam_decay: 4.5,
            foam_threshold: 0.54,
            caustic_strength: 0.85,
            underwater_enabled: true,
            ..Self::default()
        }
    }

    /// Open ocean filling `bounds`. Same frozen look as [`Self::great_lakes`]
    /// (bake datum / optical 18.6 / Gerstner 0.85); coverage is a wet rectangle
    /// so the island can sit in surrounding sea instead of a lake mask.
    pub fn ocean(water_id: u32, terrain_id: u32, bounds: [f32; 4]) -> Self {
        Self {
            preset: 2,
            body_kind: 1,
            ..Self::great_lakes(water_id, terrain_id, bounds)
        }
    }

    /// Renderer-facing descriptor; large textures and query arrays stay out of ECS.
    pub fn descriptor(self) -> somnium_renderer::water_body::WaterBodyDescriptor {
        somnium_renderer::water_body::WaterBodyDescriptor {
            water_id: self.water_id,
            terrain_id: self.terrain_id,
            preset: self.preset,
            surface_level: self.surface_level,
            max_depth: self.max_depth,
            bounds: self.bounds,
            amplitude: self.amplitude,
            wave_dir_a: self.wave_dir_a,
            wave_dir_b: self.wave_dir_b,
            wave_length_a: self.wave_length_a,
            wave_length_b: self.wave_length_b,
            wave_speed: self.wave_speed,
            wave_steepness: self.wave_steepness,
        }
    }
}

impl somnium_ecs::Component for WaterComponent {}

/// Distributed buoyancy parameters for a floating vessel (Phase IV-I/J).
///
/// The demo boat and any future player craft share this component so the
/// inspector can edit thrust, drag, and draft without reaching into the
/// example binary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BuoyantVessel {
    /// Water body whose surface queries drive the samples.
    pub water_id: u32,
    /// World-space origin of that body's parent terrain.
    pub water_origin: glam::Vec3,
    /// Newtons of buoyancy applied at a fully submerged sample.
    pub buoyancy_per_sample: f32,
    /// Linear drag coefficient against relative water velocity.
    pub linear_drag: f32,
    /// Extra drag on the rotational part of sample velocity.
    pub angular_drag: f32,
    /// Constant bow thrust while afloat, in newtons.
    pub propulsion_force: f32,
    /// Metres of immersion that map a sample from dry to fully wet.
    pub draft: f32,
    /// Righting force that pulls a rolled hull back upright.
    pub righting: f32,
}

impl Default for BuoyantVessel {
    fn default() -> Self {
        Self {
            water_id: u32::MAX,
            water_origin: glam::Vec3::ZERO,
            buoyancy_per_sample: 16_000.0,
            linear_drag: 1_200.0,
            angular_drag: 2_400.0,
            propulsion_force: 7_500.0,
            draft: 0.65,
            righting: 9_000.0,
        }
    }
}

impl somnium_ecs::Component for BuoyantVessel {}

#[cfg(test)]
mod camera_speed_tests {
    use super::*;

    #[test]
    fn directional_light_uses_the_accepted_moonlight_default() {
        assert_eq!(LightComponent::directional(100_000.0).moon_intensity, 0.010);
    }

    #[test]
    fn disc_and_tube_convert_lumens_like_a_point() {
        let point = LightComponent::point(800.0, 10.0).photometric_color();
        let disc = LightComponent::disc(800.0, 10.0, 0.4).photometric_color();
        let tube = LightComponent::tube(800.0, 10.0, 0.75, 0.04).photometric_color();
        assert!((point - disc).length() < 1e-5);
        assert!((point - tube).length() < 1e-5);
        assert_eq!(
            LightComponent::disc(800.0, 10.0, 0.4).light_type,
            LightType::Disc
        );
        assert_eq!(
            LightComponent::tube(800.0, 10.0, 0.75, 0.04).light_type,
            LightType::Tube
        );
    }

    #[test]
    fn slider_ends_map_to_the_configured_range() {
        assert!((camera_speed_from_normalized(0.0) - MIN_CAMERA_SPEED).abs() < 1e-4);
        assert!((camera_speed_from_normalized(1.0) - MAX_CAMERA_SPEED).abs() < 1e-2);
    }

    #[test]
    fn the_default_position_is_about_five_metres_per_second() {
        let s = camera_speed_from_normalized(DEFAULT_CAMERA_SPEED_NORM);
        assert!((s - 5.0).abs() < 0.5, "default speed was {s}");
    }

    #[test]
    fn mapping_round_trips() {
        for t in [0.0_f32, 0.15, 0.5, 0.75, 1.0] {
            let back = normalized_from_camera_speed(camera_speed_from_normalized(t));
            assert!((back - t).abs() < 1e-4, "{t} -> {back}");
        }
    }

    #[test]
    fn speed_increases_monotonically() {
        let mut prev = 0.0;
        for i in 0..=20 {
            let s = camera_speed_from_normalized(i as f32 / 20.0);
            assert!(s > prev, "not monotonic at {i}");
            prev = s;
        }
    }
}
