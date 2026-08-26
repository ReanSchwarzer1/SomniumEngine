//! Software-sparse virtual-shadow page management.
//!
//! This module is deliberately renderer-API code rather than editor state.  It
//! implements the part of a virtual shadow map that is easy to get subtly
//! wrong: stable virtual addresses, directional clipmaps, screen-derived page
//! demand, a bounded physical cache, LRU eviction, invalidation, and coarse
//! parent fallback.  The page render pass consumes [`RenderPage`] records; the
//! shader-facing page table is produced by [`VirtualShadowMap::page_table_words`].
//!
//! The design follows the public sparse-VSM literature (J. Stephano's Stratus
//! write-up), not proprietary engine source.  A page is valid only when its
//! recorded light and caster revisions match the current frame.  Merely having
//! a physical slot is not enough.

use std::collections::{BTreeMap, BTreeSet};

/// Shader sentinel for a virtual page that has no resident physical tile.
pub const INVALID_PHYSICAL_PAGE: u32 = u32::MAX;
/// Dynamic-uniform capacity of the physical-page raster pass.
pub const MAX_RENDER_PAGES_PER_FRAME: u32 = 256;

/// Per-light authored shadow choice.  CSM remains the portable fallback.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShadowTechnique {
    #[default]
    Cascaded,
    Virtual,
}

/// Runtime policy for one shadow-casting light.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShadowLightPolicy {
    pub light_id: u32,
    pub technique: ShadowTechnique,
    /// If the virtual cache misses its frame budget, sample the coarser CSM
    /// rather than treating the receiver as unshadowed.
    pub csm_fallback: bool,
}

/// Which parts of the production VSM path are live on this adapter/build.
///
/// Selection is intentionally all-or-nothing.  Sampling an allocated page
/// table before the page raster pass exists would turn a feature toggle into a
/// black-shadow toggle; silently pretending that state is Virtual would be
/// worse than the explicit CSM fallback.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VirtualShadowReadiness {
    pub gpu_resources: bool,
    pub page_raster: bool,
    pub shading_sample: bool,
}

impl VirtualShadowReadiness {
    #[must_use]
    pub const fn complete(self) -> bool {
        self.gpu_resources && self.page_raster && self.shading_sample
    }
}

impl ShadowLightPolicy {
    /// Effective renderer branch.  The authored choice is preserved even when
    /// this adapter must take the CSM fallback.
    #[must_use]
    pub const fn effective(self, readiness: VirtualShadowReadiness) -> ShadowTechnique {
        match self.technique {
            ShadowTechnique::Virtual if readiness.complete() => ShadowTechnique::Virtual,
            _ => ShadowTechnique::Cascaded,
        }
    }
}

impl Default for ShadowLightPolicy {
    fn default() -> Self {
        Self {
            light_id: 0,
            technique: ShadowTechnique::Cascaded,
            csm_fallback: true,
        }
    }
}

/// Immutable allocation limits.  They are validated once at construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualShadowConfig {
    /// Resolution of one directional clip level before it is split into pages.
    pub virtual_resolution: u32,
    /// Width and height of one physical tile in texels.
    pub page_size: u32,
    /// Number of slots in the square physical pool.
    pub physical_pages: u32,
    /// Dirty/new pages that may be rendered in one frame.
    pub render_budget: u32,
    pub clip_levels: u8,
}

impl Default for VirtualShadowConfig {
    fn default() -> Self {
        Self {
            virtual_resolution: 16_384,
            page_size: 128,
            physical_pages: 1_024,
            render_budget: 64,
            clip_levels: 4,
        }
    }
}

impl VirtualShadowConfig {
    pub fn validate(self) -> Result<Self, &'static str> {
        if self.virtual_resolution == 0 || self.page_size == 0 {
            return Err("virtual shadow resolutions must be non-zero");
        }
        if !self.virtual_resolution.is_power_of_two() || !self.page_size.is_power_of_two() {
            return Err("virtual shadow resolutions must be powers of two");
        }
        if self.virtual_resolution % self.page_size != 0 {
            return Err("virtual resolution must be divisible by page size");
        }
        if self.physical_pages == 0 || self.render_budget == 0 || self.clip_levels == 0 {
            return Err("virtual shadow budgets and clip levels must be non-zero");
        }
        if self.render_budget > MAX_RENDER_PAGES_PER_FRAME {
            return Err("virtual shadow render budget exceeds raster pass capacity");
        }
        let side = (self.physical_pages as f64).sqrt() as u32;
        if side.saturating_mul(side) != self.physical_pages {
            return Err("physical page count must form a square atlas");
        }
        if self.pages_per_axis() > u16::MAX as u32 {
            return Err("virtual page coordinate exceeds u16");
        }
        Ok(self)
    }

    #[must_use]
    pub const fn pages_per_axis(self) -> u32 {
        self.virtual_resolution / self.page_size
    }

    #[must_use]
    pub fn physical_atlas_size(self) -> u32 {
        (self.physical_pages as f64).sqrt() as u32 * self.page_size
    }
}

/// Stable address in the software page table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VirtualPageKey {
    pub light_id: u32,
    pub clip_level: u8,
    pub x: u16,
    pub y: u16,
}

/// One directional clip level.  `view_proj` maps the complete virtual level to
/// NDC; [`page_view_proj`](Self::page_view_proj) crops it to a single page.
#[derive(Clone, Copy, Debug)]
pub struct DirectionalClipmap {
    pub view_proj: glam::Mat4,
    pub split_depth: f32,
    pub pages_per_axis: u32,
}

impl DirectionalClipmap {
    /// Map a visible world-space sample to its virtual page.
    #[must_use]
    pub fn page_at(self, light_id: u32, level: u8, world: glam::Vec3) -> Option<VirtualPageKey> {
        let clip = self.view_proj * world.extend(1.0);
        if clip.w.abs() <= f32::EPSILON {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        if ndc.z < 0.0 || ndc.z > 1.0 || ndc.x.abs() > 1.0 || ndc.y.abs() > 1.0 {
            return None;
        }
        let uv = glam::Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        let side = self.pages_per_axis as f32;
        let x = (uv.x * side).floor().clamp(0.0, side - 1.0) as u16;
        let y = (uv.y * side).floor().clamp(0.0, side - 1.0) as u16;
        Some(VirtualPageKey {
            light_id,
            clip_level: level,
            x,
            y,
        })
    }

    /// Matrix used while rasterising just `key` into one physical tile.
    #[must_use]
    pub fn page_view_proj(self, key: VirtualPageKey) -> glam::Mat4 {
        let scale = self.pages_per_axis as f32;
        // X uses normal NDC orientation. Y is expressed in texture orientation
        // because shadow sampling flips NDC Y before looking up the page table.
        let offset_x = scale - 2.0 * key.x as f32 - 1.0;
        let offset_y = 1.0 - scale + 2.0 * key.y as f32;
        let crop = glam::Mat4::from_cols(
            glam::Vec4::new(scale, 0.0, 0.0, 0.0),
            glam::Vec4::new(0.0, scale, 0.0, 0.0),
            glam::Vec4::new(0.0, 0.0, 1.0, 0.0),
            glam::Vec4::new(offset_x, offset_y, 0.0, 1.0),
        );
        crop * self.view_proj
    }
}

#[derive(Clone, Copy, Debug)]
struct PhysicalSlot {
    key: Option<VirtualPageKey>,
    last_used_frame: u64,
    rendered_light_revision: u64,
    rendered_caster_revision: u64,
}

impl Default for PhysicalSlot {
    fn default() -> Self {
        Self {
            key: None,
            last_used_frame: 0,
            rendered_light_revision: u64::MAX,
            rendered_caster_revision: u64::MAX,
        }
    }
}

/// Work emitted for the physical-page render pass.
#[derive(Clone, Copy, Debug)]
pub struct RenderPage {
    pub key: VirtualPageKey,
    pub physical_page: u32,
    pub view_proj: glam::Mat4,
}

/// Allocation and cache result for the current frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VirtualShadowStats {
    pub demanded: u32,
    pub resident: u32,
    pub cache_hits: u32,
    pub allocations: u32,
    pub evictions: u32,
    pub invalidated: u32,
    pub scheduled: u32,
    pub budget_misses: u32,
}

/// CPU mirror of the software-sparse virtual shadow cache.
///
/// The mirror is deterministic and is also the portable fallback for devices
/// without the atomics needed for a fully GPU-managed allocator.  A future GPU
/// allocator can publish the same dense page-table words and consume the same
/// [`RenderPage`] contract.
pub struct VirtualShadowMap {
    config: VirtualShadowConfig,
    slots: Vec<PhysicalSlot>,
    resident: BTreeMap<VirtualPageKey, u32>,
    demand: BTreeSet<VirtualPageKey>,
    frame: u64,
    light_revision: u64,
    caster_revision: u64,
    stats: VirtualShadowStats,
}

/// Renderer-owned GPU side of the virtual cache.
///
/// This owns real physical depth storage and the shader-visible page table.
/// [`VirtualShadowReadiness`] keeps selection on CSM if any production stage is
/// unavailable while preserving the authored choice for a later frame/device.
pub struct VirtualShadowGpu {
    pub physical_atlas: wgpu::Texture,
    pub physical_atlas_view: wgpu::TextureView,
    pub comparison_sampler: wgpu::Sampler,
    pub page_table: wgpu::Buffer,
    pub params: wgpu::Buffer,
    config: VirtualShadowConfig,
    atlas_initialized: bool,
}

impl VirtualShadowGpu {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: VirtualShadowConfig,
    ) -> Result<Self, &'static str> {
        let config = config.validate()?;
        let atlas_size = config.physical_atlas_size();
        let physical_atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Virtual Shadow Physical Atlas"),
            size: wgpu::Extent3d {
                width: atlas_size,
                height: atlas_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let physical_atlas_view = physical_atlas.create_view(&wgpu::TextureViewDescriptor {
            label: Some("Virtual Shadow Physical Atlas View"),
            aspect: wgpu::TextureAspect::DepthOnly,
            ..Default::default()
        });
        let comparison_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Virtual Shadow Comparison Sampler"),
            compare: Some(wgpu::CompareFunction::LessEqual),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let word_count = config.pages_per_axis() as u64
            * config.pages_per_axis() as u64
            * u64::from(config.clip_levels);
        let page_table = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Virtual Shadow Page Table"),
            size: word_count * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // [pages/axis, page size, physical atlas size, clip levels,
        //  physical slots, render budget, enabled, CSM fallback]
        let params_words = [
            config.pages_per_axis(),
            config.page_size,
            atlas_size,
            u32::from(config.clip_levels),
            config.physical_pages,
            config.render_budget,
            0,
            1,
        ];
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Virtual Shadow Params"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&params, 0, bytemuck::cast_slice(&params_words));

        Ok(Self {
            physical_atlas,
            physical_atlas_view,
            comparison_sampler,
            page_table,
            params,
            config,
            atlas_initialized: false,
        })
    }

    /// Upload only entries whose depth is valid for the current revisions.
    pub fn upload_page_table(&self, queue: &wgpu::Queue, cache: &VirtualShadowMap, light_id: u32) {
        debug_assert_eq!(self.config, cache.config());
        let words = cache.page_table_words(light_id);
        queue.write_buffer(&self.page_table, 0, bytemuck::cast_slice(&words));
    }

    /// Publish whether opaque shading may consume this cache this frame.
    pub fn set_enabled(&self, queue: &wgpu::Queue, enabled: bool, csm_fallback: bool) {
        queue.write_buffer(
            &self.params,
            24,
            bytemuck::cast_slice(&[u32::from(enabled), u32::from(csm_fallback)]),
        );
    }

    /// The first render defines every texel before later passes use `Load`.
    pub fn take_full_clear(&mut self) -> bool {
        let clear = !self.atlas_initialized;
        self.atlas_initialized = true;
        clear
    }

    /// Physical tile rectangle for `RenderPass::set_viewport`.
    #[must_use]
    pub fn page_viewport(&self, physical_page: u32) -> (f32, f32, f32, f32) {
        let side = (self.config.physical_pages as f64).sqrt() as u32;
        let x = physical_page % side;
        let y = physical_page / side;
        (
            (x * self.config.page_size) as f32,
            (y * self.config.page_size) as f32,
            self.config.page_size as f32,
            self.config.page_size as f32,
        )
    }
}

impl VirtualShadowMap {
    pub fn new(config: VirtualShadowConfig) -> Result<Self, &'static str> {
        let config = config.validate()?;
        Ok(Self {
            config,
            slots: vec![PhysicalSlot::default(); config.physical_pages as usize],
            resident: BTreeMap::new(),
            demand: BTreeSet::new(),
            frame: 0,
            light_revision: 0,
            caster_revision: 0,
            stats: VirtualShadowStats::default(),
        })
    }

    #[must_use]
    pub const fn config(&self) -> VirtualShadowConfig {
        self.config
    }

    #[must_use]
    pub fn stats(&self) -> &VirtualShadowStats {
        &self.stats
    }

    /// Start demand collection.  Revision changes invalidate cached depth but
    /// preserve mappings, avoiding allocator churn after a moving caster.
    pub fn begin_frame(&mut self, light_revision: u64, caster_revision: u64) {
        self.frame = self.frame.wrapping_add(1).max(1);
        self.demand.clear();
        self.stats = VirtualShadowStats::default();
        if light_revision != self.light_revision || caster_revision != self.caster_revision {
            self.stats.invalidated = self.resident.len() as u32;
        }
        self.light_revision = light_revision;
        self.caster_revision = caster_revision;
    }

    /// Request the page under one screen-derived world sample.  Callers
    /// normally feed a reduced depth buffer; tests and headless tools can feed
    /// reconstructed samples directly.
    pub fn request_screen_sample(
        &mut self,
        light_id: u32,
        world: glam::Vec3,
        view_depth: f32,
        clipmaps: &[DirectionalClipmap],
    ) -> Option<VirtualPageKey> {
        let level = clipmaps
            .iter()
            .position(|clip| view_depth <= clip.split_depth)
            .unwrap_or_else(|| clipmaps.len().saturating_sub(1));
        let key = clipmaps.get(level)?.page_at(light_id, level as u8, world)?;
        self.demand.insert(key);
        Some(key)
    }

    /// Demand a small neighbourhood to hide bilinear/PCF seams at page edges.
    pub fn request_neighbourhood(&mut self, centre: VirtualPageKey, radius: u16) {
        let side = self.config.pages_per_axis() as i32;
        for y in centre.y as i32 - radius as i32..=centre.y as i32 + radius as i32 {
            for x in centre.x as i32 - radius as i32..=centre.x as i32 + radius as i32 {
                if x >= 0 && y >= 0 && x < side && y < side {
                    self.demand.insert(VirtualPageKey {
                        x: x as u16,
                        y: y as u16,
                        ..centre
                    });
                }
            }
        }
    }

    /// Resolve demand into bounded physical-page raster work.
    pub fn resolve(&mut self, clipmaps: &[DirectionalClipmap]) -> Vec<RenderPage> {
        self.stats.demanded = self.demand.len() as u32;
        let demanded = self.demand.iter().copied().collect::<Vec<_>>();
        let mut work = Vec::new();

        for key in demanded {
            let (slot_index, newly_allocated) = if let Some(&slot) = self.resident.get(&key) {
                self.stats.cache_hits += 1;
                (slot, false)
            } else if let Some(slot) = self.allocate_slot() {
                self.resident.insert(key, slot);
                self.slots[slot as usize].key = Some(key);
                self.stats.allocations += 1;
                (slot, true)
            } else {
                self.stats.budget_misses += 1;
                continue;
            };

            let slot = &mut self.slots[slot_index as usize];
            slot.last_used_frame = self.frame;
            let dirty = newly_allocated
                || slot.rendered_light_revision != self.light_revision
                || slot.rendered_caster_revision != self.caster_revision;

            if dirty {
                if work.len() >= self.config.render_budget as usize {
                    self.stats.budget_misses += 1;
                    continue;
                }
                let Some(clip) = clipmaps.get(key.clip_level as usize).copied() else {
                    self.stats.budget_misses += 1;
                    continue;
                };
                work.push(RenderPage {
                    key,
                    physical_page: slot_index,
                    view_proj: clip.page_view_proj(key),
                });
                slot.rendered_light_revision = self.light_revision;
                slot.rendered_caster_revision = self.caster_revision;
            }
        }

        self.stats.scheduled = work.len() as u32;
        self.stats.resident = self.resident.len() as u32;
        work
    }

    fn allocate_slot(&mut self) -> Option<u32> {
        if let Some(index) = self.slots.iter().position(|slot| slot.key.is_none()) {
            return Some(index as u32);
        }

        // Never evict a page demanded by the current frame.  LRU among the
        // rest is deterministic because physical index breaks equal-frame ties.
        let victim = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.key.is_some_and(|key| !self.demand.contains(&key)))
            .min_by_key(|(index, slot)| (slot.last_used_frame, *index))
            .map(|(index, _)| index)?;
        if let Some(old) = self.slots[victim].key.take() {
            self.resident.remove(&old);
            self.stats.evictions += 1;
        }
        self.slots[victim].rendered_light_revision = u64::MAX;
        self.slots[victim].rendered_caster_revision = u64::MAX;
        Some(victim as u32)
    }

    /// Physical page for `key`, or the first resident coarser parent.
    #[must_use]
    pub fn resident_or_parent(&self, mut key: VirtualPageKey) -> Option<u32> {
        loop {
            if let Some(&physical) = self.resident.get(&key) {
                let slot = &self.slots[physical as usize];
                if slot.rendered_light_revision == self.light_revision
                    && slot.rendered_caster_revision == self.caster_revision
                {
                    return Some(physical);
                }
            }
            key = self.parent_key(key)?;
        }
    }

    #[must_use]
    pub fn parent_key(&self, key: VirtualPageKey) -> Option<VirtualPageKey> {
        if key.clip_level + 1 >= self.config.clip_levels {
            return None;
        }
        let quarter = self.config.pages_per_axis() / 4;
        Some(VirtualPageKey {
            clip_level: key.clip_level + 1,
            x: (u32::from(key.x) / 2 + quarter) as u16,
            y: (u32::from(key.y) / 2 + quarter) as u16,
            ..key
        })
    }

    /// Dense shader table ordered `[clip][y][x]` for one light.
    #[must_use]
    pub fn page_table_words(&self, light_id: u32) -> Vec<u32> {
        let side = self.config.pages_per_axis() as usize;
        let mut words = vec![INVALID_PHYSICAL_PAGE; side * side * self.config.clip_levels as usize];
        for (&key, &physical) in &self.resident {
            if key.light_id != light_id {
                continue;
            }
            let slot = &self.slots[physical as usize];
            if slot.rendered_light_revision != self.light_revision
                || slot.rendered_caster_revision != self.caster_revision
            {
                continue;
            }
            let index =
                key.clip_level as usize * side * side + key.y as usize * side + key.x as usize;
            words[index] = physical;
        }
        words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(physical_pages: u32, budget: u32) -> VirtualShadowConfig {
        VirtualShadowConfig {
            virtual_resolution: 512,
            page_size: 128,
            physical_pages,
            render_budget: budget,
            clip_levels: 2,
        }
    }

    fn clips() -> [DirectionalClipmap; 2] {
        [
            DirectionalClipmap {
                view_proj: glam::Mat4::IDENTITY,
                split_depth: 10.0,
                pages_per_axis: 4,
            },
            DirectionalClipmap {
                view_proj: glam::Mat4::IDENTITY,
                split_depth: 100.0,
                pages_per_axis: 4,
            },
        ]
    }

    #[test]
    fn rejects_non_square_physical_pool() {
        assert!(VirtualShadowMap::new(config(3, 1)).is_err());
    }

    #[test]
    fn render_budget_cannot_outgrow_the_dynamic_raster_buffer() {
        assert!(VirtualShadowMap::new(config(4, MAX_RENDER_PAGES_PER_FRAME + 1)).is_err());
    }

    #[test]
    fn authored_virtual_choice_falls_back_until_every_gpu_stage_is_ready() {
        let policy = ShadowLightPolicy {
            technique: ShadowTechnique::Virtual,
            ..ShadowLightPolicy::default()
        };
        assert_eq!(
            policy.effective(VirtualShadowReadiness {
                gpu_resources: true,
                page_raster: false,
                shading_sample: false,
            }),
            ShadowTechnique::Cascaded
        );
        assert_eq!(
            policy.effective(VirtualShadowReadiness {
                gpu_resources: true,
                page_raster: true,
                shading_sample: true,
            }),
            ShadowTechnique::Virtual
        );
    }

    #[test]
    fn screen_sample_selects_level_and_page() {
        let mut map = VirtualShadowMap::new(config(4, 4)).unwrap();
        map.begin_frame(1, 1);
        let key = map
            .request_screen_sample(7, glam::Vec3::new(0.25, -0.25, 0.5), 4.0, &clips())
            .unwrap();
        assert_eq!(
            key,
            VirtualPageKey {
                light_id: 7,
                clip_level: 0,
                x: 2,
                y: 2
            }
        );
    }

    #[test]
    fn page_crop_maps_page_centre_to_ndc_origin() {
        let clip = clips()[0];
        let key = VirtualPageKey {
            light_id: 0,
            clip_level: 0,
            x: 2,
            y: 1,
        };
        // Page (2,1) centre in full NDC is (+0.25,+0.25): texture Y is flipped.
        let cropped = clip.page_view_proj(key) * glam::Vec4::new(0.25, 0.25, 0.5, 1.0);
        assert!(cropped.x.abs() < 1e-6, "x={}", cropped.x);
        assert!(cropped.y.abs() < 1e-6, "y={}", cropped.y);
    }

    #[test]
    fn unchanged_pages_are_cache_hits_without_render_work() {
        let mut map = VirtualShadowMap::new(config(4, 4)).unwrap();
        map.begin_frame(1, 1);
        let key = map
            .request_screen_sample(0, glam::Vec3::ZERO, 1.0, &clips())
            .unwrap();
        assert_eq!(map.resolve(&clips()).len(), 1);
        assert_eq!(map.resident_or_parent(key), Some(0));

        map.begin_frame(1, 1);
        map.request_screen_sample(0, glam::Vec3::ZERO, 1.0, &clips());
        assert!(map.resolve(&clips()).is_empty());
        assert_eq!(map.stats().cache_hits, 1);
    }

    #[test]
    fn caster_revision_invalidates_depth_without_reallocating() {
        let mut map = VirtualShadowMap::new(config(4, 4)).unwrap();
        map.begin_frame(1, 1);
        map.request_screen_sample(0, glam::Vec3::ZERO, 1.0, &clips());
        let first = map.resolve(&clips());

        map.begin_frame(1, 2);
        map.request_screen_sample(0, glam::Vec3::ZERO, 1.0, &clips());
        let second = map.resolve(&clips());
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].physical_page, second[0].physical_page);
        assert_eq!(map.stats().allocations, 0);
        assert_eq!(map.stats().invalidated, 1);
    }

    #[test]
    fn render_budget_keeps_unrendered_pages_invalid() {
        let mut map = VirtualShadowMap::new(config(4, 1)).unwrap();
        map.begin_frame(1, 1);
        let a = VirtualPageKey {
            light_id: 0,
            clip_level: 0,
            x: 0,
            y: 0,
        };
        let b = VirtualPageKey { x: 1, ..a };
        map.request_neighbourhood(a, 0);
        map.request_neighbourhood(b, 0);
        assert_eq!(map.resolve(&clips()).len(), 1);
        assert_eq!(map.stats().budget_misses, 1);
        let words = map.page_table_words(0);
        assert_eq!(
            words
                .iter()
                .filter(|&&word| word != INVALID_PHYSICAL_PAGE)
                .count(),
            1
        );
    }

    #[test]
    fn lru_evicts_only_a_page_not_demanded_this_frame() {
        let mut map = VirtualShadowMap::new(config(1, 1)).unwrap();
        let a = VirtualPageKey {
            light_id: 0,
            clip_level: 0,
            x: 0,
            y: 0,
        };
        let b = VirtualPageKey { x: 1, ..a };
        map.begin_frame(1, 1);
        map.request_neighbourhood(a, 0);
        map.resolve(&clips());
        map.begin_frame(1, 1);
        map.request_neighbourhood(b, 0);
        let work = map.resolve(&clips());
        assert_eq!(work[0].key, b);
        assert_eq!(map.stats().evictions, 1);
        assert!(map.resident_or_parent(a).is_none());
    }

    #[test]
    fn coarse_parent_is_used_when_fine_page_misses() {
        let mut map = VirtualShadowMap::new(config(4, 4)).unwrap();
        let fine = VirtualPageKey {
            light_id: 0,
            clip_level: 0,
            x: 2,
            y: 2,
        };
        let parent = map.parent_key(fine).unwrap();
        map.begin_frame(1, 1);
        map.request_neighbourhood(parent, 0);
        map.resolve(&clips());
        assert_eq!(map.resident_or_parent(fine), map.resident_or_parent(parent));
    }
}
