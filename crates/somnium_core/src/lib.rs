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
pub mod app;
pub mod config;
pub mod context;
pub mod editor_commands;
pub mod error;
pub mod event;
pub mod landscape;
pub mod light_units;
pub mod log_capture;
pub mod scene_serial;
pub mod sun;
pub mod time;

// ── Re-exports for ergonomic top-level access ──────────────────────────────

pub use app::{Engine, GameApp};
pub use config::EngineConfig;
pub use context::{EngineContext, SimulationClock, SimulationState};
pub use editor_commands::{
    CreateEntityCmd, CreateLandscapeCmd, DeleteEntityCmd, EditorCommand, EntitySnapshot,
    ReparentCmd, SetLightCmd, SetNameCmd, SetTransformCmd, UndoStack,
};
pub use error::EngineError;
pub use event::{EngineEvent, InputState};
pub use landscape::{
    BuiltLandscape, DEFAULT_LANDSCAPE_VERSION, DefaultLandscapePreset, create_default_landscape,
};
pub use scene_serial::{parse_scene, save_scene};
pub use time::TimeState;

// Re-export input types so game code does not need a direct `winit` dependency.
pub use winit::event::MouseButton;
pub use winit::keyboard::KeyCode;

// Re-export core ECS types so game code can use them from `somnium_core`.
pub use somnium_ecs::{Component, ComponentBundle, Entity, World};
pub use somnium_ecs::{ComponentId, ComponentSet};

/// ECS Component for a mesh instance.
#[derive(Debug, Clone, Copy)]
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
#[derive(Debug, Clone, Copy)]
pub struct MaterialComponent {
    /// Index into the renderer's material pool.
    pub id: u32,
}
impl somnium_ecs::Component for MaterialComponent {}

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
}
impl somnium_ecs::Component for TerrainComponent {}

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
#[derive(Debug, Clone, Copy, PartialEq)]
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
#[derive(Debug, Clone, Copy, PartialEq)]
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
    /// Temporal anti-aliasing (Phase 24F).
    pub taa_enabled: bool,
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
    /// Whether FXAA anti-aliasing is applied (Phase 15A2).
    ///
    /// Unlike the stylistic effects above this defaults **on** — it is an image
    /// quality feature, and the visibility-buffer pipeline has no MSAA, so
    /// edges are otherwise hard-aliased.
    pub fxaa_enabled: bool,
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
    /// Analytic UV gradients in vis-buffer shading (Phase 25N). Default on.
    pub analytic_grad: bool,
    /// Light-shaft boost on the sun in-scatter (Phase 24U). 1 is unscaled air.
    pub shaft_intensity: f32,
    /// AMD FSR 3 temporal reconstruct to the window. Default on; `SOMNIUM_FSR=0`.
    ///
    /// Replaces Somnium TAA and the bilinear present blit while enabled. RCAS
    /// sharpness lives in `fsr_sharpness`; Somnium CAS stays off on this path.
    pub fsr_enabled: bool,
    /// FSR RCAS sharpness, 0..=1. Default 0.8.
    pub fsr_sharpness: f32,
}

impl Default for PostProcessComponent {
    fn default() -> Self {
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
            bloom_enabled: true,
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
            cas_enabled: std::env::var("SOMNIUM_CAS").as_deref() != Ok("0"),
            cas_sharpness: 0.5,
            cas_strength: 1.0,
            motion_blur_enabled: std::env::var("SOMNIUM_MOTION_BLUR").as_deref() == Ok("1"),
            motion_blur_shutter: 0.5,
            restir_gi_intensity: 1.0,
            volumetrics_enabled: std::env::var("SOMNIUM_VOLUMETRICS").as_deref() != Ok("0"),
            light_shafts: true,
            fog_density: 0.0008,
            fog_height_falloff: 120.0,
            fog_asymmetry: 0.6,
            // Mirrors `GtaoPass::new`; the component overwrites the pass every
            // frame, so these are the values that actually take effect.
            gtao_radius: 1.0,
            gtao_intensity: 1.0,
            dof_enabled: false,
            dof_focus_distance: 10.0,
            taa_enabled: true,
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
            fxaa_enabled: true,
            pcss_enabled: true,
            contact_shadows_enabled: true,
            world_cache: std::env::var("SOMNIUM_WORLD_CACHE").as_deref() == Ok("1"),
            cache_intensity: 1.0,
            cache_cell_size: 2.0,
            specular_gi: std::env::var("SOMNIUM_SPECULAR_GI").as_deref() == Ok("1"),
            spec_roughness: 0.15,
            path_tracer: std::env::var("SOMNIUM_PATH_TRACER").as_deref() == Ok("1"),
            path_bounces: 3,
            mesh_sdf: std::env::var("SOMNIUM_MESH_SDF").as_deref() == Ok("1"),
            probes: std::env::var("SOMNIUM_PROBES").as_deref() == Ok("1"),
            probe_intensity: 1.0,
            analytic_grad: std::env::var("SOMNIUM_ANALYTIC_GRAD").as_deref() != Ok("0"),
            shaft_intensity: 1.5,
            fsr_enabled: std::env::var("SOMNIUM_FSR").as_deref() != Ok("0"),
            fsr_sharpness: 0.8,
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

// ─── Phase CR: Camera settings ──────────────────────────────────────────────

/// Scene-wide camera settings, exposed as a selectable "Camera" entity.
///
/// Fly-cam lives in `hello_engine`; Physical Camera is a Post FX exposure
/// triangle. This component is the inspector home for CPU frustum early-out
/// (Phase CR-B), independent of GPU 15B on F10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraSettingsComponent {
    /// Skip terrain chunks whose AABB misses the camera frustum before they
    /// reach `draw_queue`. Default on. Off-screen casters still shadow into
    /// view. `SOMNIUM_CPU_FRUSTUM=0` forces this off.
    pub frustum_cull: bool,
}

impl Default for CameraSettingsComponent {
    fn default() -> Self {
        Self { frustum_cull: true }
    }
}

impl CameraSettingsComponent {
    /// Honour `SOMNIUM_CPU_FRUSTUM=0` at spawn.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            frustum_cull: std::env::var("SOMNIUM_CPU_FRUSTUM").as_deref() != Ok("0"),
        }
    }
}

impl somnium_ecs::Component for CameraSettingsComponent {}

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
    /// Linear RGBA color at birth.
    pub color_start: [f32; 4],
    /// Linear RGBA color at end of life.
    pub color_end: [f32; 4],
    /// Downward gravity acceleration (m/s²).
    pub gravity: f32,

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
            color_start: [1.0, 0.4, 0.1, 1.0],
            color_end: [0.2, 0.0, 0.0, 0.0],
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
            emitter.particles.push(ParticleState {
                position: origin,
                velocity: dir * speed,
                age: 0.0,
                lifetime,
            });
        }

        // ── 3. Emit GPU instances ─────────────────────────────────────────────
        let size_start = emitter.size_start;
        let size_end = emitter.size_end;
        let color_start = emitter.color_start;
        let color_end = emitter.color_end;

        for p in &emitter.particles {
            let frac = (p.age / p.lifetime).clamp(0.0, 1.0);
            let size = size_start + (size_end - size_start) * frac;
            let color = [
                color_start[0] + (color_end[0] - color_start[0]) * frac,
                color_start[1] + (color_end[1] - color_start[1]) * frac,
                color_start[2] + (color_end[2] - color_start[2]) * frac,
                color_start[3] + (color_end[3] - color_start[3]) * frac,
            ];
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
    /// 0 = legacy/unset, 1 = baked Great Lakes lake preset.
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
