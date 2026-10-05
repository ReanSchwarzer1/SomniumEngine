//! Vertex painting: a per-placement RGBA8 mask on a mesh's vertices. Each
//! channel paints a *layer*: any material asset (albedo, normal, roughness,
//! metal, occlusion, height), projected in world space and height-blended
//! into the surface under it. A channel with no material paints the built-in
//! weathering instead: R dirt and grime, G rust, B wetness, A blood. The
//! blend lives in `shading.wgsl` (`apply_vertex_layers`,
//! `apply_vertex_weathering`); this module owns the authored data, the brush
//! and the generators, and hands changed masks to the renderer.
//!
//! The masks are stored on the entity, not the mesh asset, so two placements
//! of one crate weather differently. Order is the mesh's vertex order as
//! uploaded (`SomniumRenderer::read_mesh` returns it), which for an imported
//! glTF primitive is the accessor order.

use std::collections::{HashMap, HashSet};

use base64::Engine as _;
use glam::Vec3;
use somnium_asset::database::AssetId;
use somnium_ecs::{Component, ComponentId, Entity, World};

pub use somnium_renderer::vertex_paint::{NO_LAYER, PaintLayers, pack, unpack};

/// Channel names in mask order, as the brush UI and MCP spell them: what the
/// channel paints when it has no layer material.
pub const CHANNELS: [&str; 4] = ["dirt", "rust", "wet", "blood"];

/// The channel `name` refers to: a weathering name, `r`/`g`/`b`/`a` or `1`..`4`.
#[must_use]
pub fn channel_index(name: &str) -> Option<usize> {
    CHANNELS
        .iter()
        .position(|c| *c == name)
        .or_else(|| ["r", "g", "b", "a"].iter().position(|c| *c == name))
        .or_else(|| ["1", "2", "3", "4"].iter().position(|c| *c == name))
}

/// An entity's painted masks and the material each channel paints.
#[derive(Debug, Clone, PartialEq)]
pub struct VertexPaintComponent {
    /// Off keeps the paint but renders the bare material.
    pub enabled: bool,
    /// The masks, encoded by [`encode`]. Empty means unpainted.
    pub data: String,
    /// The material each channel blends in. Unset: the built-in weathering.
    pub layer_r: AssetId,
    pub layer_g: AssetId,
    pub layer_b: AssetId,
    pub layer_a: AssetId,
    /// Per layer (R, G, B, A): texture repeats per world metre.
    pub tiling: [f32; 4],
    /// Per layer: 0 fades by the painted amount, 1 lets the height maps
    /// shape the edge (the layer fills crevices before it covers).
    pub height_contrast: [f32; 4],
    /// Per layer: the lowest and highest surface normal Y it shows on.
    /// -1..1 is everywhere; 0.7..1 only what faces up; -0.3..0.3 only walls.
    pub slope_min: [f32; 4],
    pub slope_max: [f32; 4],
    /// Per layer: how far noise breaks up part-painted areas into patches.
    pub breakup: [f32; 4],
    /// Per layer: size of those patches in metres.
    pub breakup_scale: [f32; 4],
}

impl Component for VertexPaintComponent {}

impl Default for VertexPaintComponent {
    fn default() -> Self {
        let layers = PaintLayers::default();
        Self {
            enabled: true,
            data: String::new(),
            layer_r: AssetId::NONE,
            layer_g: AssetId::NONE,
            layer_b: AssetId::NONE,
            layer_a: AssetId::NONE,
            tiling: layers.tiling,
            height_contrast: layers.height_contrast,
            slope_min: layers.slope_min,
            slope_max: layers.slope_max,
            breakup: layers.breakup,
            breakup_scale: layers.breakup_scale,
        }
    }
}

impl VertexPaintComponent {
    /// The layer material of each channel, R to A.
    #[must_use]
    pub fn layer_assets(&self) -> [AssetId; 4] {
        [self.layer_r, self.layer_g, self.layer_b, self.layer_a]
    }

    /// Set channel `index`'s layer material.
    pub fn set_layer_asset(&mut self, index: usize, asset: AssetId) {
        *[&mut self.layer_r, &mut self.layer_g, &mut self.layer_b, &mut self.layer_a][index] = asset;
    }

    /// What the shader reads, with each layer's material resolved to its
    /// renderer slot by `runtime`. False when a set layer has no slot yet
    /// (its asset is still loading or is missing): that channel weathers.
    #[must_use]
    pub fn gpu_layers(&self, runtime: &HashMap<AssetId, u32>) -> (PaintLayers, bool) {
        let mut resolved = true;
        let material = self.layer_assets().map(|asset| {
            if asset == AssetId::NONE {
                return NO_LAYER;
            }
            runtime.get(&asset).copied().unwrap_or_else(|| {
                resolved = false;
                NO_LAYER
            })
        });
        (
            PaintLayers {
                material,
                tiling: self.tiling,
                height_contrast: self.height_contrast,
                slope_min: self.slope_min,
                slope_max: self.slope_max,
                breakup: self.breakup,
                breakup_scale: self.breakup_scale,
            },
            resolved,
        )
    }
}

/// Every layer material some enabled, painted entity uses.
#[must_use]
pub fn layer_assets(world: &World) -> HashSet<AssetId> {
    world
        .iter_with::<VertexPaintComponent>()
        .filter(|(_, paint)| paint.enabled && !paint.data.is_empty())
        .flat_map(|(_, paint)| paint.layer_assets())
        .filter(|asset| *asset != AssetId::NONE)
        .collect()
}

/// Renderer key for an entity's paint slot.
#[must_use]
pub fn key(entity: Entity) -> u64 {
    (u64::from(entity.generation()) << 32) | u64::from(entity.index())
}

// ── Encoding ────────────────────────────────────────────────────────────────

/// `z1:` + base64 of the zlib-compressed little-endian words. Masks are
/// smooth and mostly repeat, so this is 5-15x smaller than the raw words in
/// the scene JSON. Decoding also accepts `w1:` (uncompressed words).
#[must_use]
pub fn encode(colors: &[u32]) -> String {
    if colors.iter().all(|&c| c == 0) {
        return String::new();
    }
    let raw: Vec<u8> = colors.iter().flat_map(|w| w.to_le_bytes()).collect();
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
    format!("z1:{}", base64::engine::general_purpose::STANDARD.encode(packed))
}

/// Inverse of [`encode`]. `None` for a malformed string.
#[must_use]
pub fn decode(data: &str) -> Option<Vec<u32>> {
    if data.is_empty() {
        return Some(Vec::new());
    }
    let (tag, body) = data.split_at_checked(3)?;
    let mut bytes = base64::engine::general_purpose::STANDARD.decode(body).ok()?;
    match tag {
        "w1:" => {}
        "z1:" => bytes = miniz_oxide::inflate::decompress_to_vec_zlib(&bytes).ok()?,
        _ => return None,
    }
    (bytes.len() % 4 == 0).then(|| {
        bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    })
}

/// The entity's masks resized to `vertex_count` (missing vertices unpainted).
#[must_use]
pub fn colors_of(world: &World, entity: Entity, vertex_count: usize) -> Vec<u32> {
    let mut colors = world
        .get::<VertexPaintComponent>(entity)
        .and_then(|c| decode(&c.data))
        .unwrap_or_default();
    colors.resize(vertex_count, 0);
    colors
}

/// Write `colors` to the entity, adding the component if needed. Returns the
/// previous component for undo.
pub fn store(world: &mut World, entity: Entity, colors: &[u32]) -> Option<VertexPaintComponent> {
    let before = world.get::<VertexPaintComponent>(entity).cloned();
    let data = encode(colors);
    match world.get_mut::<VertexPaintComponent>(entity) {
        Some(c) => c.data = data,
        None => {
            let _ = world.insert_component(
                entity,
                VertexPaintComponent {
                    data,
                    ..VertexPaintComponent::default()
                },
            );
        }
    }
    before
}

// ── Brush ───────────────────────────────────────────────────────────────────

/// The Vertex Paint tool's brush.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VertexPaintBrush {
    /// World-space radius in metres.
    pub radius: f32,
    /// How far one dab moves a vertex toward the target, `0..1`.
    pub strength: f32,
    /// 0 = hard edge, 1 = linear falloff to the rim.
    pub falloff: f32,
    /// Dirt, rust, wet, blood.
    pub channels: [bool; 4],
    /// Remove paint instead of adding it.
    pub erase: bool,
}

impl Default for VertexPaintBrush {
    fn default() -> Self {
        Self {
            radius: 0.5,
            strength: 0.35,
            falloff: 0.75,
            channels: [true, false, false, false],
            erase: false,
        }
    }
}

/// One dab at `center` (Flax's weighting: `strength * lerp(1, 1 - d/r,
/// falloff)`). `positions` are world space, in vertex order. Returns how many
/// vertices changed.
pub fn dab(colors: &mut [u32], positions: &[Vec3], center: Vec3, brush: &VertexPaintBrush) -> usize {
    let radius = brush.radius.max(1e-4);
    let target = if brush.erase { 0.0 } else { 1.0 };
    let mut changed = 0;
    for (color, p) in colors.iter_mut().zip(positions) {
        let d = p.distance(center) / radius;
        if d > 1.0 {
            continue;
        }
        let w = (brush.strength * (1.0 + (-d) * brush.falloff.clamp(0.0, 1.0))).clamp(0.0, 1.0);
        let mut m = unpack(*color);
        for (ch, on) in brush.channels.iter().enumerate() {
            if *on {
                // At least one 8-bit step, so a light brush still finishes.
                let step = (target - m[ch]) * w;
                m[ch] = if step.abs() < 1.0 / 255.0 && w > 0.0 && m[ch] != target {
                    m[ch] + (target - m[ch]).signum() / 255.0
                } else {
                    m[ch] + step
                };
            }
        }
        let packed = pack(m);
        if packed != *color {
            *color = packed;
            changed += 1;
        }
    }
    changed
}

/// Set the chosen channels of every vertex to `value`.
pub fn fill(colors: &mut [u32], channels: [bool; 4], value: f32) {
    for color in colors {
        let mut m = unpack(*color);
        for (ch, on) in channels.iter().enumerate() {
            if *on {
                m[ch] = value;
            }
        }
        *color = pack(m);
    }
}

// ── Generators ──────────────────────────────────────────────────────────────

/// A rule that scores every vertex `0..1`, for painting whole sets of props at
/// once: grime that rises from the floor, dust on top faces, rust under lips
/// and in crevices.
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(missing_docs)] // the variant docs say what each parameter is
pub enum Rule {
    /// 1 at `ground` (default: the mesh's lowest point) fading to 0 at
    /// `ground + height`.
    Ground { height: f32, ground: Option<f32> },
    /// Upward-facing, `n.y^power`.
    Up { power: f32 },
    /// Downward-facing, `(-n.y)^power`.
    Down { power: f32 },
    /// Faces whose normal Y lies in `min..=max`, soft over 0.1 at each end:
    /// -0.3..0.3 is walls, 0.7..1 is what faces up.
    Slope { min: f32, max: f32 },
    /// Concave vertices (creases, inside corners), from neighbour geometry.
    Cavity { gain: f32 },
    /// Convex vertices (edges, corners).
    Edges { gain: f32 },
    /// World-space value-noise patches.
    Noise { scale: f32, seed: f32 },
    /// Vertical runs (water damage, rain below a ledge): noise stretched
    /// eight times along world Y.
    Streaks { scale: f32, seed: f32 },
    /// Every vertex.
    All,
}

/// How a generated layer combines with what is already painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Blend {
    Max,
    Add,
    Set,
    Multiply,
}

/// Score every vertex. `positions` and `normals` are world space.
#[must_use]
pub fn generate(rule: Rule, positions: &[Vec3], normals: &[Vec3], indices: &[u32]) -> Vec<f32> {
    match rule {
        Rule::All => vec![1.0; positions.len()],
        Rule::Ground { height, ground } => {
            let floor = ground.unwrap_or_else(|| positions.iter().map(|p| p.y).fold(f32::MAX, f32::min));
            let h = height.max(1e-3);
            positions
                .iter()
                .map(|p| 1.0 - smoothstep(0.0, h, p.y - floor))
                .collect()
        }
        Rule::Up { power } => normals.iter().map(|n| n.y.max(0.0).powf(power.max(0.01))).collect(),
        Rule::Down { power } => normals.iter().map(|n| (-n.y).max(0.0).powf(power.max(0.01))).collect(),
        Rule::Slope { min, max } => normals
            .iter()
            .map(|n| smoothstep(min - 0.1, min, n.y) * (1.0 - smoothstep(max, max + 0.1, n.y)))
            .collect(),
        Rule::Cavity { gain } => curvature(positions, normals, indices)
            .into_iter()
            .map(|c| (c * gain).clamp(0.0, 1.0))
            .collect(),
        Rule::Edges { gain } => curvature(positions, normals, indices)
            .into_iter()
            .map(|c| (-c * gain).clamp(0.0, 1.0))
            .collect(),
        Rule::Noise { scale, seed } => positions
            .iter()
            .map(|p| smoothstep(0.42, 0.72, fbm(*p * scale.max(1e-3) + Vec3::splat(seed * 17.0))))
            .collect(),
        Rule::Streaks { scale, seed } => positions
            .iter()
            .map(|p| {
                let q = *p * Vec3::new(1.0, 0.125, 1.0) * scale.max(1e-3) + Vec3::splat(seed * 17.0);
                smoothstep(0.5, 0.72, fbm(q))
            })
            .collect(),
    }
}

/// Combine `values * amount` into one channel.
pub fn apply(colors: &mut [u32], channel: usize, values: &[f32], amount: f32, blend: Blend) {
    for (color, v) in colors.iter_mut().zip(values) {
        let mut m = unpack(*color);
        let x = (v * amount).clamp(0.0, 1.0);
        m[channel] = match blend {
            Blend::Max => m[channel].max(x),
            Blend::Add => m[channel] + x,
            Blend::Set => x,
            Blend::Multiply => m[channel] * x,
        };
        *color = pack(m);
    }
}

/// Mean of `dot(n_a, normalize(p_b - p_a))` over each vertex's neighbours:
/// positive where the surface around a vertex rises above its tangent plane
/// (a crease), negative on a ridge.
fn curvature(positions: &[Vec3], normals: &[Vec3], indices: &[u32]) -> Vec<f32> {
    let mut sum = vec![0.0f32; positions.len()];
    let mut count = vec![0u32; positions.len()];
    let mut edge = |a: usize, b: usize| {
        if let (Some(pa), Some(pb), Some(na)) = (positions.get(a), positions.get(b), normals.get(a)) {
            let d = (*pb - *pa).normalize_or_zero();
            sum[a] += na.dot(d);
            count[a] += 1;
        }
    };
    for tri in indices.chunks_exact(3) {
        let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        for (x, y) in [(a, b), (b, c), (c, a)] {
            edge(x, y);
            edge(y, x);
        }
    }
    sum.iter()
        .zip(&count)
        .map(|(s, &n)| if n == 0 { 0.0 } else { s / n as f32 })
        .collect()
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn hash3(p: Vec3) -> f32 {
    let q = (p * 0.318_309_9 + Vec3::new(0.71, 0.113, 0.419)).fract() * 17.0;
    (q.x * q.y * q.z * (q.x + q.y + q.z)).fract()
}

fn value_noise(p: Vec3) -> f32 {
    let i = p.floor();
    let f = p - i;
    let u = f * f * (Vec3::splat(3.0) - 2.0 * f);
    let h = |x: f32, y: f32, z: f32| hash3(i + Vec3::new(x, y, z));
    let x00 = h(0.0, 0.0, 0.0) + (h(1.0, 0.0, 0.0) - h(0.0, 0.0, 0.0)) * u.x;
    let x10 = h(0.0, 1.0, 0.0) + (h(1.0, 1.0, 0.0) - h(0.0, 1.0, 0.0)) * u.x;
    let x01 = h(0.0, 0.0, 1.0) + (h(1.0, 0.0, 1.0) - h(0.0, 0.0, 1.0)) * u.x;
    let x11 = h(0.0, 1.0, 1.0) + (h(1.0, 1.0, 1.0) - h(0.0, 1.0, 1.0)) * u.x;
    let y0 = x00 + (x10 - x00) * u.y;
    let y1 = x01 + (x11 - x01) * u.y;
    y0 + (y1 - y0) * u.z
}

fn fbm(p: Vec3) -> f32 {
    0.5 * value_noise(p) + 0.3 * value_noise(p * 2.03 + 11.0) + 0.2 * value_noise(p * 4.07 + 23.0)
}

// ── Renderer sync ───────────────────────────────────────────────────────────

/// Hands changed masks to the renderer. Costs one comparison on a frame where
/// no `VertexPaint` was written and nothing was spawned or despawned.
#[derive(Default)]
pub struct VertexPaintSync {
    tick: u64,
    structure: u64,
    /// Per slot: fingerprint of the masks, and the layer header uploaded.
    fingerprints: HashMap<u64, (u64, PaintLayers)>,
    /// Slots with a layer material that had no renderer slot yet.
    unresolved: HashSet<u64>,
    /// Frames until the unresolved layers are tried again.
    retry_in: u32,
}

impl VertexPaintSync {
    /// Whether [`VertexPaintSync::sync`] has anything to do this frame: paint
    /// was written, entities came or went, or a layer material that was not
    /// loaded yet is due another look (twice a second at 60 fps).
    pub fn stale(&mut self, world: &World) -> bool {
        if world.change_tick(ComponentId::of::<VertexPaintComponent>()) != self.tick
            || world.structure_tick() != self.structure
        {
            return true;
        }
        if self.unresolved.is_empty() {
            return false;
        }
        self.retry_in = self.retry_in.saturating_sub(1);
        self.retry_in == 0
    }

    /// Upload changed masks and layer settings, and drop the slots of
    /// entities that lost theirs. `materials` maps each layer material that
    /// is ready to its renderer slot.
    pub fn sync(
        &mut self,
        world: &World,
        renderer: &mut somnium_renderer::SomniumRenderer,
        materials: &HashMap<AssetId, u32>,
    ) {
        let tick = world.change_tick(ComponentId::of::<VertexPaintComponent>());
        let written = tick != self.tick;
        self.tick = tick;
        self.structure = world.structure_tick();
        self.retry_in = 30;

        let mut live = HashSet::new();
        for (entity, paint) in world.iter_with::<VertexPaintComponent>() {
            if !paint.enabled || paint.data.is_empty() {
                continue;
            }
            let key = key(entity);
            live.insert(key);
            let known = self.fingerprints.get(&key).copied();
            if !written && known.is_some() && !self.unresolved.contains(&key) {
                continue;
            }
            let (layers, resolved) = paint.gpu_layers(materials);
            if resolved {
                self.unresolved.remove(&key);
            } else {
                self.unresolved.insert(key);
            }
            // The masks only need hashing again when a component was written.
            let fp = match known {
                Some((fp, _)) if !written => fp,
                _ => fingerprint(paint.data.as_bytes()),
            };
            if known == Some((fp, layers)) {
                continue;
            }
            match decode(&paint.data) {
                Some(colors) => {
                    renderer.vertex_paint.set(key, &layers, &colors);
                    self.fingerprints.insert(key, (fp, layers));
                }
                None => tracing::warn!("VertexPaint on entity {} is malformed; ignored", entity.index()),
            }
        }
        renderer.vertex_paint.retain(|k| live.contains(&k));
        self.fingerprints.retain(|k, _| live.contains(k));
        self.unresolved.retain(|k| live.contains(k));
    }
}

fn fingerprint(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ bytes.len() as u64;
    for chunk in bytes.chunks(8) {
        let mut w = [0u8; 8];
        w[..chunk.len()].copy_from_slice(chunk);
        h = (h ^ u64::from_le_bytes(w)).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(23);
    }
    h
}

/// The renderer handle for `entity`'s paint, or 0. For draw submission.
#[must_use]
pub fn handle(renderer: &somnium_renderer::SomniumRenderer, entity: Entity) -> u32 {
    renderer.vertex_paint.handle(key(entity))
}

// ── Editing ─────────────────────────────────────────────────────────────────

/// One mesh read back from the GPU pool, in its own (local) space.
#[allow(missing_docs)]
pub struct PaintMesh {
    pub entity: Entity,
    pub vertex_offset: u32,
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub indices: Vec<u32>,
}

impl PaintMesh {
    /// Read `entity`'s mesh back from the renderer. Blocking.
    #[must_use]
    pub fn read(
        world: &World,
        renderer: &somnium_renderer::SomniumRenderer,
        ctx: &somnium_renderer::RenderContext,
        entity: Entity,
    ) -> Option<Self> {
        let mesh = world.get::<crate::MeshComponent>(entity)?;
        let (vertices, indices) =
            renderer.read_mesh(ctx, mesh.vertex_offset, mesh.index_offset, mesh.index_count)?;
        Some(Self {
            entity,
            vertex_offset: mesh.vertex_offset,
            positions: vertices.iter().map(|v| Vec3::from_array(v.position)).collect(),
            normals: vertices.iter().map(|v| Vec3::from_array(v.normal)).collect(),
            indices,
        })
    }

    /// World-space positions and normals under `model`.
    #[must_use]
    pub fn world(&self, model: glam::Mat4) -> (Vec<Vec3>, Vec<Vec3>) {
        let normal_matrix = glam::Mat3::from_mat4(model).inverse().transpose();
        (
            self.positions.iter().map(|p| model.transform_point3(*p)).collect(),
            self.normals
                .iter()
                .map(|n| (normal_matrix * *n).normalize_or_zero())
                .collect(),
        )
    }

    /// Nearest world-space hit of a world ray on this mesh (Möller–Trumbore in
    /// local space, so non-uniform scale is exact).
    #[must_use]
    pub fn raycast(&self, model: glam::Mat4, origin: Vec3, dir: Vec3) -> Option<Vec3> {
        let inv = model.inverse();
        let o = inv.transform_point3(origin);
        let d = inv.transform_vector3(dir);
        let mut best = f32::MAX;
        for tri in self.indices.chunks_exact(3) {
            let (Some(a), Some(b), Some(c)) = (
                self.positions.get(tri[0] as usize),
                self.positions.get(tri[1] as usize),
                self.positions.get(tri[2] as usize),
            ) else {
                continue;
            };
            let (e1, e2) = (*b - *a, *c - *a);
            let p = d.cross(e2);
            let det = e1.dot(p);
            if det.abs() < 1e-12 {
                continue;
            }
            let inv_det = 1.0 / det;
            let s = o - *a;
            let u = s.dot(p) * inv_det;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let q = s.cross(e1);
            let v = d.dot(q) * inv_det;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let t = e2.dot(q) * inv_det;
            if t > 0.0 && t < best {
                best = t;
            }
        }
        (best < f32::MAX).then(|| model.transform_point3(o + d * best))
    }
}

/// The default "this has been here a while" stack: grime rising up walls
/// and settling in creases, dust patches on tops, damp at the foot of walls
/// and in patches on floors, rust on the edges and undersides of metal.
/// Floors are not grimed wholesale: the rising terms fade out on up-facing
/// faces, which would otherwise all sit "at the ground". Returns new masks.
#[must_use]
pub fn auto_weather(colors: &[u32], positions: &[Vec3], normals: &[Vec3], indices: &[u32], metal: bool) -> Vec<u32> {
    let mut out = colors.to_vec();
    let score = |rule| generate(rule, positions, normals, indices);
    let times = |a: Vec<f32>, b: &[f32]| -> Vec<f32> { a.iter().zip(b).map(|(x, y)| x * y).collect() };
    let not_floor: Vec<f32> = normals.iter().map(|n| 1.0 - n.y.max(0.0).powi(2)).collect();
    apply(&mut out, 0, &times(score(Rule::Ground { height: 0.6, ground: None }), &not_floor), 0.85, Blend::Max);
    apply(&mut out, 0, &score(Rule::Cavity { gain: 3.0 }), 0.7, Blend::Max);
    apply(&mut out, 0, &score(Rule::Noise { scale: 1.3, seed: 0.0 }), 0.25, Blend::Max);
    let dust = times(score(Rule::Up { power: 4.0 }), &score(Rule::Noise { scale: 0.8, seed: 7.0 }));
    apply(&mut out, 0, &dust, 0.45, Blend::Max);
    apply(&mut out, 2, &times(score(Rule::Ground { height: 0.25, ground: None }), &not_floor), 0.6, Blend::Max);
    let puddles = times(score(Rule::Up { power: 4.0 }), &score(Rule::Noise { scale: 0.6, seed: 9.0 }));
    apply(&mut out, 2, &times(puddles, &score(Rule::Ground { height: 0.05, ground: None })), 0.7, Blend::Max);
    if metal {
        let patches = score(Rule::Noise { scale: 2.2, seed: 3.0 });
        for rule in [Rule::Edges { gain: 2.5 }, Rule::Down { power: 2.0 }] {
            let values: Vec<f32> = score(rule).iter().zip(&patches).map(|(v, n)| v * (0.4 + 0.6 * n)).collect();
            apply(&mut out, 1, &values, 0.75, Blend::Max);
        }
    }
    out
}

/// One undo step: a stroke, a fill, a paste or a generated pass.
#[allow(missing_docs)]
pub struct VertexPaintCmd {
    pub entity: Entity,
    pub before: Option<VertexPaintComponent>,
    pub after: VertexPaintComponent,
}

impl VertexPaintCmd {
    fn write(world: &mut World, entity: Entity, value: Option<&VertexPaintComponent>) {
        match value {
            Some(v) => match world.get_mut::<VertexPaintComponent>(entity) {
                Some(c) => *c = v.clone(),
                None => {
                    let _ = world.insert_component(entity, v.clone());
                }
            },
            None => {
                let _ = world.remove_component::<VertexPaintComponent>(entity);
            }
        }
    }
}

impl crate::editor_commands::EditorCommand for VertexPaintCmd {
    fn execute(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        Self::write(world, self.entity, Some(&self.after));
    }

    fn undo(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        Self::write(world, self.entity, self.before.as_ref());
    }

    fn description(&self) -> &str {
        "Vertex paint"
    }

    fn is_no_op(&self) -> bool {
        self.before.as_ref() == Some(&self.after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_round_trips_and_compresses_sparse_paint() {
        let mut colors = vec![0u32; 1000];
        colors[10] = pack([1.0, 0.0, 0.5, 0.0]);
        let s = encode(&colors);
        assert!(s.starts_with("z1:") && s.len() < 100, "{}", s.len());
        assert_eq!(decode(&s).unwrap(), colors);
        let noisy: Vec<u32> = (0..300u32).map(|i| i.wrapping_mul(2_654_435_761)).collect();
        assert_eq!(decode(&encode(&noisy)).unwrap(), noisy);
        // What tools/vertex_paint_bake.py writes: zlib.compress of the LE words.
        assert_eq!(decode("z1:eJxjYGBgAAAABAAB").unwrap(), vec![0]);
        assert_eq!(decode("w1:AQAAAA==").unwrap(), vec![1]);
        assert_eq!(encode(&[0, 0]), "");
        assert_eq!(decode("").unwrap(), Vec::<u32>::new());
        assert!(decode("x1:AAAA").is_none());
    }

    #[test]
    fn layers_resolve_to_renderer_slots_and_report_what_is_still_loading() {
        let rock = AssetId::from_relative_path("environment/rock.sommat");
        let moss = AssetId::from_relative_path("environment/moss.sommat");
        let mut paint = VertexPaintComponent::default();
        paint.set_layer_asset(0, rock);
        paint.set_layer_asset(3, moss);
        paint.slope_min[3] = 0.7;
        assert_eq!(paint.layer_assets(), [rock, AssetId::NONE, AssetId::NONE, moss]);
        let mut ready = HashMap::from([(rock, 5u32)]);
        let (layers, resolved) = paint.gpu_layers(&ready);
        assert_eq!(layers.material, [5, NO_LAYER, NO_LAYER, NO_LAYER]);
        assert!(!resolved, "moss has no slot yet");
        ready.insert(moss, 9);
        let (layers, resolved) = paint.gpu_layers(&ready);
        assert_eq!((layers.material[3], layers.slope_min[3], resolved), (9, 0.7, true));
        assert_eq!(channel_index("wet"), Some(2));
        assert_eq!(channel_index("a"), Some(3));
        assert_eq!(channel_index("2"), Some(1));
        assert_eq!(channel_index("mud"), None);
    }

    #[test]
    fn a_dab_paints_inside_the_radius_only_and_erase_undoes_it() {
        let positions = [Vec3::ZERO, Vec3::new(0.4, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)];
        let mut colors = vec![0u32; 3];
        let brush = VertexPaintBrush {
            radius: 0.5,
            strength: 1.0,
            falloff: 0.0,
            channels: [false, true, false, false],
            erase: false,
        };
        assert_eq!(dab(&mut colors, &positions, Vec3::ZERO, &brush), 2);
        assert_eq!(unpack(colors[0])[1], 1.0);
        assert_eq!(unpack(colors[0])[0], 0.0, "only the chosen channel");
        assert_eq!(colors[2], 0);
        let erase = VertexPaintBrush { erase: true, ..brush };
        dab(&mut colors, &positions, Vec3::ZERO, &erase);
        assert_eq!(colors, vec![0, 0, 0]);
    }

    #[test]
    fn a_weak_brush_still_reaches_full_paint() {
        let mut colors = vec![0u32];
        let brush = VertexPaintBrush {
            strength: 0.02,
            ..VertexPaintBrush::default()
        };
        for _ in 0..2000 {
            dab(&mut colors, &[Vec3::ZERO], Vec3::ZERO, &brush);
        }
        assert_eq!(unpack(colors[0])[0], 1.0);
    }

    #[test]
    fn generators_find_the_floor_the_tops_and_the_crease() {
        // A V-shaped crease: the bottom vertex is concave, the rims are edges.
        let positions = [
            Vec3::new(-1.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        let normals = [Vec3::Y; 4];
        let indices = [0, 1, 3, 1, 2, 3];
        let ground = generate(Rule::Ground { height: 0.5, ground: None }, &positions, &normals, &indices);
        assert_eq!(ground[1], 1.0);
        assert_eq!(ground[0], 0.0);
        let cavity = generate(Rule::Cavity { gain: 2.0 }, &positions, &normals, &indices);
        assert!(cavity[1] > 0.5, "{cavity:?}");
        assert!(cavity[0] < 0.01);
        let up = generate(Rule::Up { power: 1.0 }, &positions, &normals, &indices);
        assert_eq!(up, vec![1.0; 4]);
        let walls = generate(Rule::Slope { min: -0.3, max: 0.3 }, &positions, &[Vec3::Y, Vec3::X, Vec3::NEG_Y, Vec3::Z], &indices);
        assert_eq!(walls, vec![0.0, 1.0, 0.0, 1.0]);
        let mut colors = vec![0u32; 4];
        apply(&mut colors, 0, &ground, 1.0, Blend::Max);
        assert_eq!(unpack(colors[1])[0], 1.0);
    }
}
