//! Nested material clipmaps (Phase DF — Daggerfall).
//!
//! Look-at-centred stacks of RGBA caches. Generate runs strongest-four + hex +
//! height-blend **once per dirty texel** in world XZ; shading taps the cache
//! instead of repeating that work per vis-buffer pixel. Architecture studied
//! from O3DE `TerrainClipmapManager` (Apache-2.0 OR MIT) — original WGSL/Rust,
//! no source copied. See ATTRIBUTION.md §1.8.

use super::GpuTerrainMaterial;

/// Detail clipmap edge in texels. 1024 matches O3DE's default and keeps both
/// stacks under the 128 MiB GPU budget.
pub const DETAIL_SIZE: u32 = 1024;
/// Nested rings, scale base 2. Eight rings at 512 texels/m reach ~128 m radius.
pub const DETAIL_RINGS: u32 = 8;
/// Finest detail density. Walking must still resolve hex; raising this is how
/// we recover fidelity, not by skipping hex at the feet.
pub const DETAIL_FINEST_TEXELS_PER_M: f32 = 512.0;
pub const CLIPMAP_SCALE_BASE: f32 = 2.0;

/// Macro stack: unique-colour already owns the coarsest hue; these rings cover
/// the 1 km tile between the detail stack and that map.
pub const MACRO_SIZE: u32 = 1024;
pub const MACRO_RINGS: u32 = 4;
pub const MACRO_FINEST_TEXELS_PER_M: f32 = 4.0;

/// Do not retile until the camera has moved this many texels (O3DE `updateMultiple`).
/// 16 texels at 512 t/m is ~3 cm — walking no longer retile-every-frame on the
/// finest ring while still catching real motion.
pub const UPDATE_MULTIPLE: i32 = 16;
/// Extra texels generated past a dirty edge so bilinear wrap does not sample
/// a stale neighbour (O3DE extended margin).
pub const EXTENDED_MARGIN: i32 = 2;

/// Hex in generate when the ring is dense enough to resolve a tile.
/// 8 t/m still covers a ~60 m look-at (ring 6); below that unique-colour owns hue.
pub const HEX_MIN_TEXELS_PER_M: f32 = 8.0;

/// Generate at most one 1024² ring per frame. A full-stack refresh is 12
/// rings; filling it in one shot re-runs hex on 12M texels and hitchs the
/// editor. Walking L-strips are tens of thousands of texels.
pub const MAX_GEN_TEXELS: u32 = 1024 * 1024;
const MAX_TAKE_JOBS: usize = 32;

const CLIPMAP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// One axis-aligned dirty rectangle in clipmap texel space, already unwrapped
/// into `0..size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl ClipRect {
    pub fn is_empty(self) -> bool {
        self.w == 0 || self.h == 0
    }
}

/// CPU state + GPU images for one terrain's clipmap stacks.
pub struct TerrainClipmap {
    pub enabled: bool,
    detail_albedo: wgpu::Texture,
    detail_surface: wgpu::Texture,
    macro_albedo: wgpu::Texture,
    macro_normal: wgpu::Texture,
    detail_albedo_sampled: wgpu::TextureView,
    detail_surface_sampled: wgpu::TextureView,
    macro_albedo_sampled: wgpu::TextureView,
    macro_normal_sampled: wgpu::TextureView,
    detail_albedo_layers: [wgpu::TextureView; DETAIL_RINGS as usize],
    detail_surface_layers: [wgpu::TextureView; DETAIL_RINGS as usize],
    macro_albedo_layers: [wgpu::TextureView; MACRO_RINGS as usize],
    macro_normal_layers: [wgpu::TextureView; MACRO_RINGS as usize],
    pub detail_albedo_ids: [i32; DETAIL_RINGS as usize],
    pub detail_surface_ids: [i32; DETAIL_RINGS as usize],
    pub macro_albedo_ids: [i32; MACRO_RINGS as usize],
    pub macro_normal_ids: [i32; MACRO_RINGS as usize],
    rings: [ClipmapRing; DETAIL_RINGS as usize],
    macro_rings: [ClipmapRing; MACRO_RINGS as usize],
    /// Last terrain `edit_revision` we generated against.
    edit_revision: u64,
    initialized: bool,
}

#[derive(Clone, Copy, Debug)]
struct ClipmapRing {
    center: [f32; 2],
    /// Toroidal texel origin. Sampling UV is `fract(logical + origin / size)`.
    origin: [i32; 2],
    texels_per_m: f32,
    size: u32,
    dirty: [ClipRect; 4],
    dirty_count: u8,
}

impl ClipmapRing {
    fn new(texels_per_m: f32, size: u32) -> Self {
        Self {
            center: [0.0; 2],
            origin: [0; 2],
            texels_per_m,
            size,
            dirty: [ClipRect {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            }; 4],
            dirty_count: 0,
        }
    }

    fn mark_full(&mut self) {
        self.dirty[0] = ClipRect {
            x: 0,
            y: 0,
            w: self.size,
            h: self.size,
        };
        self.dirty_count = 1;
    }

    fn clear_dirty(&mut self) {
        self.dirty_count = 0;
    }

    fn push_dirty(&mut self, rect: ClipRect) {
        if rect.is_empty() {
            return;
        }
        if (self.dirty_count as usize) < self.dirty.len() {
            self.dirty[self.dirty_count as usize] = rect;
            self.dirty_count += 1;
        } else {
            self.mark_full();
        }
    }

    #[allow(dead_code)]
    fn world_extent(&self) -> f32 {
        self.size as f32 / self.texels_per_m
    }

    #[allow(dead_code)]
    fn contains(&self, world_xz: [f32; 2], margin_m: f32) -> bool {
        let half = self.world_extent() * 0.5 - margin_m;
        (world_xz[0] - self.center[0]).abs() < half && (world_xz[1] - self.center[1]).abs() < half
    }
}

impl TerrainClipmap {
    pub fn env_forced_off() -> bool {
        matches!(
            std::env::var("SOMNIUM_TERRAIN_CLIPMAP").as_deref(),
            Ok("0") | Ok("off")
        )
    }

    /// Default off until DF-E gates pass. `SOMNIUM_TERRAIN_CLIPMAP=1` forces on.
    pub fn env_default_enabled() -> bool {
        if Self::env_forced_off() {
            return false;
        }
        matches!(
            std::env::var("SOMNIUM_TERRAIN_CLIPMAP").as_deref(),
            Ok("1") | Ok("on")
        )
    }

    /// Next `update` regenerates every ring (inspector enable, hex toggle).
    pub fn invalidate(&mut self) {
        self.initialized = false;
    }

    pub fn new(device: &wgpu::Device) -> Self {
        let make = |label: &str, size: u32, layers: u32| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: CLIPMAP_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let sampled = |tex: &wgpu::Texture, label: &str| {
            tex.create_view(&wgpu::TextureViewDescriptor {
                label: Some(label),
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                usage: Some(wgpu::TextureUsages::TEXTURE_BINDING),
                ..Default::default()
            })
        };
        let layer = |tex: &wgpu::Texture, i: u32| {
            tex.create_view(&wgpu::TextureViewDescriptor {
                label: None,
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: i,
                array_layer_count: Some(1),
                usage: Some(wgpu::TextureUsages::RENDER_ATTACHMENT),
                ..Default::default()
            })
        };

        let detail_albedo = make("Terrain clipmap detail albedo", DETAIL_SIZE, DETAIL_RINGS);
        let detail_surface = make("Terrain clipmap detail surface", DETAIL_SIZE, DETAIL_RINGS);
        let macro_albedo = make("Terrain clipmap macro albedo", MACRO_SIZE, MACRO_RINGS);
        let macro_normal = make("Terrain clipmap macro normal", MACRO_SIZE, MACRO_RINGS);

        let rings = std::array::from_fn(|i| {
            let tpm = DETAIL_FINEST_TEXELS_PER_M / CLIPMAP_SCALE_BASE.powi(i as i32);
            ClipmapRing::new(tpm, DETAIL_SIZE)
        });
        let macro_rings = std::array::from_fn(|i| {
            let tpm = MACRO_FINEST_TEXELS_PER_M / CLIPMAP_SCALE_BASE.powi(i as i32);
            ClipmapRing::new(tpm, MACRO_SIZE)
        });

        Self {
            enabled: Self::env_default_enabled(),
            detail_albedo_sampled: sampled(&detail_albedo, "Terrain clipmap detail albedo sampled"),
            detail_surface_sampled: sampled(
                &detail_surface,
                "Terrain clipmap detail surface sampled",
            ),
            macro_albedo_sampled: sampled(&macro_albedo, "Terrain clipmap macro albedo sampled"),
            macro_normal_sampled: sampled(&macro_normal, "Terrain clipmap macro normal sampled"),
            detail_albedo_layers: std::array::from_fn(|i| layer(&detail_albedo, i as u32)),
            detail_surface_layers: std::array::from_fn(|i| layer(&detail_surface, i as u32)),
            macro_albedo_layers: std::array::from_fn(|i| layer(&macro_albedo, i as u32)),
            macro_normal_layers: std::array::from_fn(|i| layer(&macro_normal, i as u32)),
            detail_albedo,
            detail_surface,
            macro_albedo,
            macro_normal,
            detail_albedo_ids: [-1; DETAIL_RINGS as usize],
            detail_surface_ids: [-1; DETAIL_RINGS as usize],
            macro_albedo_ids: [-1; MACRO_RINGS as usize],
            macro_normal_ids: [-1; MACRO_RINGS as usize],
            rings,
            macro_rings,
            edit_revision: 0,
            initialized: false,
        }
    }

    /// Publish each array layer as a bindless `texture_2d` so shading can tap
    /// rings without a texture-array slot in the global pool.
    pub fn register_bindless(&mut self, add: &mut dyn FnMut(wgpu::TextureView) -> u32) {
        let layer_view = |tex: &wgpu::Texture, layer: u32, label: &'static str| {
            tex.create_view(&wgpu::TextureViewDescriptor {
                label: Some(label),
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            })
        };
        for i in 0..DETAIL_RINGS {
            self.detail_albedo_ids[i as usize] =
                add(layer_view(&self.detail_albedo, i, "Clipmap detail albedo")) as i32;
            self.detail_surface_ids[i as usize] = add(layer_view(
                &self.detail_surface,
                i,
                "Clipmap detail surface",
            )) as i32;
        }
        for i in 0..MACRO_RINGS {
            self.macro_albedo_ids[i as usize] =
                add(layer_view(&self.macro_albedo, i, "Clipmap macro albedo")) as i32;
            self.macro_normal_ids[i as usize] =
                add(layer_view(&self.macro_normal, i, "Clipmap macro normal")) as i32;
        }
    }

    pub fn detail_sampled(&self) -> (&wgpu::TextureView, &wgpu::TextureView) {
        (&self.detail_albedo_sampled, &self.detail_surface_sampled)
    }

    pub fn macro_sampled(&self) -> (&wgpu::TextureView, &wgpu::TextureView) {
        (&self.macro_albedo_sampled, &self.macro_normal_sampled)
    }

    pub fn detail_layer(&self, ring: u32) -> (&wgpu::TextureView, &wgpu::TextureView) {
        let i = (ring as usize).min(self.detail_albedo_layers.len() - 1);
        (&self.detail_albedo_layers[i], &self.detail_surface_layers[i])
    }

    pub fn macro_layer(&self, ring: u32) -> (&wgpu::TextureView, &wgpu::TextureView) {
        let i = (ring as usize).min(self.macro_albedo_layers.len() - 1);
        (&self.macro_albedo_layers[i], &self.macro_normal_layers[i])
    }

    /// Recentre rings on the look-at (see [`focus_xz`]) and collect dirty rectangles.
    ///
    /// A sculpt/paint that bumps `edit_revision` forces a full refresh.
    pub fn update(&mut self, camera_xz: [f32; 2], edit_revision: u64) {
        let force_full = !self.initialized || edit_revision != self.edit_revision;
        self.edit_revision = edit_revision;
        self.initialized = true;
        for ring in &mut self.rings {
            update_ring(ring, camera_xz, force_full);
        }
        for ring in &mut self.macro_rings {
            update_ring(ring, camera_xz, force_full);
        }
    }

    pub fn detail_jobs(&self) -> Vec<ClipmapGenJob> {
        jobs_for(&self.rings, true)
    }

    pub fn macro_jobs(&self) -> Vec<ClipmapGenJob> {
        jobs_for(&self.macro_rings, false)
    }

    /// Consume dirty rectangles up to `budget` texels. Leftover stays queued
    /// so a later frame can finish; do not call `clear_dirty` after this.
    pub fn take_jobs(&mut self, is_detail: bool, budget: &mut u32) -> Vec<ClipmapGenJob> {
        let rings = if is_detail {
            &mut self.rings[..]
        } else {
            &mut self.macro_rings[..]
        };
        let mut out = Vec::new();
        for (ring_i, ring) in rings.iter_mut().enumerate() {
            if *budget == 0 || out.len() >= MAX_TAKE_JOBS {
                break;
            }
            let pending: Vec<ClipRect> = ring.dirty[..ring.dirty_count as usize].to_vec();
            ring.dirty_count = 0;
            for rect in pending {
                if rect.is_empty() {
                    continue;
                }
                if *budget == 0 || out.len() >= MAX_TAKE_JOBS {
                    ring.push_dirty(rect);
                    continue;
                }
                let (now, rest) = split_rect(rect, *budget);
                let used = now.w.saturating_mul(now.h);
                *budget = budget.saturating_sub(used);
                if !now.is_empty() {
                    out.push(job_from_ring(ring, ring_i as u32, now, is_detail));
                }
                if let Some(left) = rest {
                    ring.push_dirty(left);
                }
            }
        }
        out
    }

    pub fn clear_dirty(&mut self) {
        for ring in &mut self.rings {
            ring.clear_dirty();
        }
        for ring in &mut self.macro_rings {
            ring.clear_dirty();
        }
    }

    /// Write clipmap addressing into the terrain material uploaded this frame.
    pub fn fill_gpu(&self, material: &mut GpuTerrainMaterial) {
        let on = self.enabled && !Self::env_forced_off();
        material.clipmap_enabled = u32::from(on);
        material.clipmap_rings = DETAIL_RINGS;
        material.clipmap_size = DETAIL_SIZE as f32;
        material.clipmap_debug = 0;
        material.clipmap_albedo = self.detail_albedo_ids;
        material.clipmap_surface = self.detail_surface_ids;
        for (i, ring) in self.rings.iter().enumerate() {
            material.clipmap_center[i * 2] = ring.center[0];
            material.clipmap_center[i * 2 + 1] = ring.center[1];
            material.clipmap_origin[i * 2] = ring.origin[0] as f32 / ring.size as f32;
            material.clipmap_origin[i * 2 + 1] = ring.origin[1] as f32 / ring.size as f32;
            material.clipmap_tpm[i] = ring.texels_per_m;
        }
        material.clipmap_macro_albedo = self.macro_albedo_ids;
        material.clipmap_macro_normal = self.macro_normal_ids;
        for (i, ring) in self.macro_rings.iter().enumerate() {
            material.clipmap_macro_center[i * 2] = ring.center[0];
            material.clipmap_macro_center[i * 2 + 1] = ring.center[1];
            material.clipmap_macro_origin[i * 2] = ring.origin[0] as f32 / ring.size as f32;
            material.clipmap_macro_origin[i * 2 + 1] = ring.origin[1] as f32 / ring.size as f32;
            material.clipmap_macro_tpm[i] = ring.texels_per_m;
        }
        material.clipmap_macro_rings = MACRO_RINGS;
        material.clipmap_macro_size = MACRO_SIZE as f32;
        material._clipmap_pad = [0.0; 2];
    }

    /// GPU memory for both stacks (albedo + packed surface, no mips).
    pub fn gpu_bytes() -> u64 {
        let detail = 2u64 * (DETAIL_SIZE as u64) * (DETAIL_SIZE as u64) * 4 * DETAIL_RINGS as u64;
        let macro_b = 2u64 * (MACRO_SIZE as u64) * (MACRO_SIZE as u64) * 4 * MACRO_RINGS as u64;
        detail + macro_b
    }

    /// Bindless indices that alias the storage images. Generate must not
    /// sample these in the same dispatch that writes them.
    pub fn bindless_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.detail_albedo_ids
            .iter()
            .chain(self.detail_surface_ids.iter())
            .chain(self.macro_albedo_ids.iter())
            .chain(self.macro_normal_ids.iter())
            .copied()
            .filter_map(|id| u32::try_from(id).ok())
    }
}

/// One compute dispatch: one ring, one unwrapped dirty rectangle.
#[derive(Clone, Copy, Debug)]
pub struct ClipmapGenJob {
    pub ring: u32,
    pub rect: ClipRect,
    pub center: [f32; 2],
    pub origin_uv: [f32; 2],
    pub texels_per_m: f32,
    pub clipmap_size: f32,
    pub hex: u32,
    pub is_detail: bool,
}

fn job_from_ring(
    ring: &ClipmapRing,
    ring_i: u32,
    rect: ClipRect,
    is_detail: bool,
) -> ClipmapGenJob {
    ClipmapGenJob {
        ring: ring_i,
        rect,
        center: ring.center,
        origin_uv: [
            ring.origin[0] as f32 / ring.size as f32,
            ring.origin[1] as f32 / ring.size as f32,
        ],
        texels_per_m: ring.texels_per_m,
        clipmap_size: ring.size as f32,
        hex: u32::from(ring.texels_per_m >= HEX_MIN_TEXELS_PER_M),
        is_detail,
    }
}

fn split_rect(rect: ClipRect, budget: u32) -> (ClipRect, Option<ClipRect>) {
    let texels = rect.w.saturating_mul(rect.h);
    if texels == 0 || texels <= budget {
        return (rect, None);
    }
    let rows = (budget / rect.w.max(1)).max(1);
    if rows >= rect.h {
        return (rect, None);
    }
    (
        ClipRect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: rows,
        },
        Some(ClipRect {
            x: rect.x,
            y: rect.y + rows,
            w: rect.w,
            h: rect.h - rows,
        }),
    )
}

fn jobs_for(rings: &[ClipmapRing], is_detail: bool) -> Vec<ClipmapGenJob> {
    let mut out = Vec::new();
    for (ring_i, ring) in rings.iter().enumerate() {
        let hex = u32::from(ring.texels_per_m >= HEX_MIN_TEXELS_PER_M);
        for i in 0..ring.dirty_count {
            let rect = ring.dirty[i as usize];
            if rect.is_empty() {
                continue;
            }
            out.push(ClipmapGenJob {
                ring: ring_i as u32,
                rect,
                center: ring.center,
                origin_uv: [
                    ring.origin[0] as f32 / ring.size as f32,
                    ring.origin[1] as f32 / ring.size as f32,
                ],
                texels_per_m: ring.texels_per_m,
                clipmap_size: ring.size as f32,
                hex,
                is_detail,
            });
        }
    }
    out
}

fn update_ring(ring: &mut ClipmapRing, camera_xz: [f32; 2], force_full: bool) {
    let texel_m = 1.0 / ring.texels_per_m;
    let snap = |v: f32| (v / texel_m).round() * texel_m;
    let new_center = [snap(camera_xz[0]), snap(camera_xz[1])];
    if force_full {
        ring.center = new_center;
        ring.origin = [0; 2];
        ring.mark_full();
        return;
    }
    let dx_tex = ((new_center[0] - ring.center[0]) * ring.texels_per_m).round() as i32;
    let dy_tex = ((new_center[1] - ring.center[1]) * ring.texels_per_m).round() as i32;
    if dx_tex.abs() < UPDATE_MULTIPLE && dy_tex.abs() < UPDATE_MULTIPLE {
        return;
    }
    // Leftover dirty rectangles are in the current origin's texel space.
    // Sliding first would make them write the wrong world XZ.
    if ring.dirty_count > 0 {
        return;
    }
    let size = ring.size as i32;
    if dx_tex.abs() >= size || dy_tex.abs() >= size {
        ring.center = new_center;
        ring.origin = [
            wrap_i(ring.origin[0] + dx_tex, size),
            wrap_i(ring.origin[1] + dy_tex, size),
        ];
        ring.mark_full();
        return;
    }
    let old_origin = ring.origin;
    ring.center = new_center;
    ring.origin = [
        wrap_i(old_origin[0] + dx_tex, size),
        wrap_i(old_origin[1] + dy_tex, size),
    ];
    for rect in toroidal_dirty_rects(old_origin, ring.origin, size) {
        for expanded in expand_and_wrap(rect, size, EXTENDED_MARGIN) {
            ring.push_dirty(expanded);
        }
    }
}

fn wrap_i(v: i32, size: i32) -> i32 {
    ((v % size) + size) % size
}

/// Dirty strips when a toroidal origin slides from `old` to `new`.
///
/// The region that just entered the window is an L-shape; wrapping can split
/// each arm into two, so the caller may see up to four rectangles.
pub fn toroidal_dirty_rects(
    old_origin: [i32; 2],
    new_origin: [i32; 2],
    size: i32,
) -> Vec<ClipRect> {
    let dx = wrap_delta(new_origin[0] - old_origin[0], size);
    let dy = wrap_delta(new_origin[1] - old_origin[1], size);
    if dx == 0 && dy == 0 {
        return Vec::new();
    }
    if dx.abs() >= size || dy.abs() >= size {
        return vec![ClipRect {
            x: 0,
            y: 0,
            w: size as u32,
            h: size as u32,
        }];
    }
    let mut rects = Vec::new();
    // Columns that entered along X. Origin increase means the west edge left
    // and the east edge entered — those new columns live at the previous origin
    // (physical texels that now mean a new world position).
    if dx != 0 {
        let w = dx.abs() as u32;
        let x = if dx > 0 {
            wrap_i(old_origin[0], size) as u32
        } else {
            wrap_i(new_origin[0], size) as u32
        };
        rects.extend(wrap_span(x, 0, w, size as u32, size as u32));
    }
    if dy != 0 {
        let h = dy.abs() as u32;
        let y = if dy > 0 {
            wrap_i(old_origin[1], size) as u32
        } else {
            wrap_i(new_origin[1], size) as u32
        };
        rects.extend(wrap_span(0, y, size as u32, h, size as u32));
    }
    rects
}

fn wrap_delta(d: i32, size: i32) -> i32 {
    let mut d = ((d % size) + size) % size;
    if d > size / 2 {
        d -= size;
    }
    d
}

fn wrap_span(x: u32, y: u32, w: u32, h: u32, size: u32) -> Vec<ClipRect> {
    let mut out = Vec::new();
    let x1 = x + w;
    let y1 = y + h;
    let xs = if x1 <= size {
        vec![(x, w)]
    } else {
        vec![(x, size - x), (0, x1 - size)]
    };
    let ys = if y1 <= size {
        vec![(y, h)]
    } else {
        vec![(y, size - y), (0, y1 - size)]
    };
    for &(xx, ww) in &xs {
        for &(yy, hh) in &ys {
            if ww > 0 && hh > 0 {
                out.push(ClipRect {
                    x: xx,
                    y: yy,
                    w: ww,
                    h: hh,
                });
            }
        }
    }
    out
}

fn expand_and_wrap(rect: ClipRect, size: i32, margin: i32) -> Vec<ClipRect> {
    let x = rect.x as i32 - margin;
    let y = rect.y as i32 - margin;
    let w = rect.w as i32 + margin * 2;
    let h = rect.h as i32 + margin * 2;
    if w >= size || h >= size {
        return vec![ClipRect {
            x: 0,
            y: 0,
            w: size as u32,
            h: size as u32,
        }];
    }
    wrap_span(
        wrap_i(x, size) as u32,
        wrap_i(y, size) as u32,
        w as u32,
        h as u32,
        size as u32,
    )
}

/// Ground XZ the clipmap should sit on.
///
/// Ring 0 is only [`finest_radius_metres`] across (1 m at 512 t/m). Centering
/// on the camera leaves the visible near slope on a coarser ring. Intersect the
/// view ray with a plane 1.7 m below the camera and clamp to 8 m so the player
/// still sits inside ring 3 (64 t/m) while the look-at gets the dense rings.
/// Looking straight down keeps the centre under the camera.
pub fn focus_xz(camera_pos: [f32; 3], camera_forward: [f32; 3]) -> [f32; 2] {
    const MAX_LOOK_AHEAD_M: f32 = 8.0;
    const EYE_HEIGHT_M: f32 = 1.7;
    let horiz =
        (camera_forward[0] * camera_forward[0] + camera_forward[2] * camera_forward[2]).sqrt();
    if horiz < 1e-4 {
        return [camera_pos[0], camera_pos[2]];
    }
    let down = (-camera_forward[1]).max(0.0);
    let ray_t = if down > 0.08 {
        EYE_HEIGHT_M / down
    } else {
        MAX_LOOK_AHEAD_M / horiz
    };
    let xz_dist = (ray_t * horiz).min(MAX_LOOK_AHEAD_M);
    [
        camera_pos[0] + camera_forward[0] / horiz * xz_dist,
        camera_pos[2] + camera_forward[2] / horiz * xz_dist,
    ]
}

/// Finest ring radius in metres (half the world coverage of ring 0).
pub fn finest_radius_metres() -> f32 {
    (DETAIL_SIZE as f32 / DETAIL_FINEST_TEXELS_PER_M) * 0.5
}

/// Coarsest detail-ring radius in metres.
pub fn coarsest_detail_radius_metres() -> f32 {
    let tpm = DETAIL_FINEST_TEXELS_PER_M / CLIPMAP_SCALE_BASE.powi((DETAIL_RINGS - 1) as i32);
    (DETAIL_SIZE as f32 / tpm) * 0.5
}

/// Uniforms for one generate dispatch. Mirrors `ClipmapGenParams` in WGSL.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuClipmapGen {
    pub terrain_index: u32,
    pub ring: u32,
    pub rect_min: [u32; 2],
    pub rect_max: [u32; 2],
    pub center: [f32; 2],
    pub origin_uv: [f32; 2],
    pub texels_per_m: f32,
    pub clipmap_size: f32,
    pub hex: u32,
    pub _pad: u32,
    pub _pad2: [u32; 2],
}

impl GpuClipmapGen {
    pub fn from_job(terrain_index: u32, job: &ClipmapGenJob) -> Self {
        Self {
            terrain_index,
            ring: job.ring,
            rect_min: [job.rect.x, job.rect.y],
            rect_max: [job.rect.x + job.rect.w, job.rect.y + job.rect.h],
            center: job.center,
            origin_uv: job.origin_uv,
            texels_per_m: job.texels_per_m,
            clipmap_size: job.clipmap_size,
            hex: job.hex,
            _pad: 0,
            _pad2: [0; 2],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_wrap_stays_inside_the_texture() {
        assert_eq!(wrap_i(-1, 1024), 1023);
        assert_eq!(wrap_i(1024, 1024), 0);
        assert_eq!(wrap_i(1025, 1024), 1);
    }

    #[test]
    fn a_one_texel_slide_dirties_a_column() {
        let rects = toroidal_dirty_rects([0, 0], [4, 0], 1024);
        assert_eq!(rects.len(), 1);
        assert_eq!(
            rects[0],
            ClipRect {
                x: 0,
                y: 0,
                w: 4,
                h: 1024
            }
        );
    }

    #[test]
    fn wrap_across_the_seam_splits_into_quadrants() {
        let rects = wrap_span(1020, 1020, 8, 8, 1024);
        assert_eq!(rects.len(), 4);
        assert!(rects.iter().any(|r| r.x == 1020 && r.y == 1020));
        assert!(
            rects
                .iter()
                .any(|r| r.x == 0 && r.y == 0 && r.w == 4 && r.h == 4)
        );
    }

    #[test]
    fn density_math_covers_the_planned_radii() {
        assert!((finest_radius_metres() - 1.0).abs() < 1e-4);
        assert!((coarsest_detail_radius_metres() - 128.0).abs() < 1e-3);
        assert!(TerrainClipmap::gpu_bytes() <= 128 * 1024 * 1024);
    }

    #[test]
    fn focus_stays_under_the_camera_when_looking_down() {
        let p = [10.0, 5.0, 20.0];
        assert_eq!(focus_xz(p, [0.0, -1.0, 0.0]), [10.0, 20.0]);
    }

    #[test]
    fn focus_moves_forward_when_looking_along_the_ground() {
        let p = [0.0, 1.7, 0.0];
        let f = focus_xz(p, [0.0, 0.0, -1.0]);
        assert!((f[0] - 0.0).abs() < 1e-4);
        assert!((f[1] + 8.0).abs() < 1e-3);
    }

    #[test]
    fn focus_at_forty_five_degrees_hits_eye_height_ahead() {
        let inv_s2 = 0.5_f32.sqrt();
        let f = focus_xz([0.0, 1.7, 0.0], [0.0, -inv_s2, -inv_s2]);
        assert!((f[0] - 0.0).abs() < 1e-3);
        assert!((f[1] + 1.7).abs() < 0.05);
    }

    #[test]
    fn gen_params_are_64_byte_aligned() {
        assert_eq!(std::mem::size_of::<GpuClipmapGen>(), 64);
        assert_eq!(std::mem::size_of::<GpuClipmapGen>() % 16, 0);
    }

    #[test]
    fn split_rect_keeps_the_remainder() {
        let (now, rest) = split_rect(
            ClipRect {
                x: 0,
                y: 0,
                w: 1024,
                h: 1024,
            },
            1024 * 256,
        );
        assert_eq!(now.h, 256);
        assert_eq!(rest.unwrap().h, 768);
        assert_eq!(rest.unwrap().y, 256);
    }
}
