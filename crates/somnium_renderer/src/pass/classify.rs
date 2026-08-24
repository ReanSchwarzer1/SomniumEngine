//! Tile classification for binned shading (Phase DOOM-C).
//!
//! Produces, for each bin, a list of tiles and an indirect draw whose instance
//! count is that list's length. The shading pass then draws one instanced quad
//! per tile with a pipeline compiled for exactly that bin's code.
//!
//! # This path is DEFAULT OFF because it was measured slower
//!
//! `SOMNIUM_SHADE_BINS=1` turns it on. Leave it off unless you are reproducing
//! the measurement below.
//!
//! The phase plan specified compute dispatches, following Wicked Engine's
//! `visibility_shadeCS` and UE5's Nanite shade binning. Both are compute
//! because they want wave-level control inside the shading itself; what Somnium
//! wanted from them was only the *binning*, and porting `fs_main` to compute
//! would have meant removing every derivative-dependent intrinsic from a
//! 1600-line shader — `textureSample` on four mesh maps, `textureSampleCompare`
//! in the PCF filter, `dpdx`/`dpdy` on terrain world position, `fwidth` in the
//! star field and the moon limb. So this drew one instanced quad per tile
//! instead, keeping the fragment shader byte-for-byte and making the parity
//! gate a real gate rather than a comparison of two ports.
//!
//! It is correct — the binned image matched the fullscreen one to **2 pixels
//! out of 2 615 044** — and it is slower at every tile size tried:
//!
//! | tile | Shading, Coastal ground, maximized Native |
//! |---|---:|
//! | fullscreen triangle | **24.851 ms** |
//! | 8 px | 32.533 ms |
//! | 16 px | 27.820 ms |
//! | 32 px | 26.967 ms |
//! | 64 px | 26.131 ms |
//!
//! The overhead is per-primitive setup and it falls monotonically as tiles grow
//! — but it approaches the fullscreen cost from above and never crosses it,
//! while larger tiles simultaneously make the classification worse by sending
//! more tiles to `MIXED`. There is no tile size at which this wins.
//!
//! **That is the answer to why the references use compute**, and it was not
//! obvious from reading them: a dispatch has no vertex shader, no primitive
//! setup and no rasterizer, so the binning is free there and costs 1.3–7.7 ms
//! here. DOOM-B had already measured the whole prize at ~0.4 ms, so this could
//! never have paid for more than a fraction of its own overhead.
//!
//! Kept, off, because it is a working implementation of the mechanism and the
//! measurement is worth more than the deletion. **Phase DOOM-E does not use
//! it** — a depth-split pair of fullscreen draws gets per-distance pipelines
//! with one large triangle each and no primitive overhead at all
//! (`renderer.rs`, the Shading pass).

use crate::pass::census::BIN_COUNT as CENSUS_BINS;

/// Shading bins. Order matches `classify.wgsl`'s `TILE_*` constants.
pub const BIN_COUNT: usize = 6;

pub const BIN_SKY: usize = 0;
pub const BIN_MESH: usize = 1;
pub const BIN_FOLIAGE: usize = 2;
pub const BIN_TERRAIN_NEAR: usize = 3;
pub const BIN_TERRAIN_AERIAL: usize = 4;
pub const BIN_MIXED: usize = 5;

pub const BIN_NAMES: [&str; BIN_COUNT] = [
    "sky",
    "mesh",
    "foliage",
    "terrain-near",
    "terrain-aerial",
    "mixed",
];

/// Tile edge in pixels.
///
/// Eight matches Wicked's `VISIBILITY_BLOCKSIZE`. UE5 makes the equivalent
/// constant a tuned parameter (`SHADING_BIN_TILE_SIZE_BITS` is 3 or 5 depending
/// on the binning technique), which is the better lesson: this is a starting
/// point to be measured against 16 and 32, not a number arrived at by
/// revelation. It must stay even, or a 2×2 derivative quad could straddle two
/// tiles drawn by different pipelines.
pub const TILE_SIZE: u32 = 8;

/// `DrawIndirectArgs`, one per bin.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DrawArgs {
    vertex_count: u32,
    instance_count: u32,
    first_vertex: u32,
    first_instance: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ClassifyParams {
    width: u32,
    height: u32,
    tiles_x: u32,
    tile_capacity: u32,
    aerial_split: f32,
    tile_size: u32,
    _pad: [u32; 2],
}

pub struct ClassifyPass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    draw_args: wgpu::Buffer,
    tiles: wgpu::Buffer,
    params: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
    width: u32,
    height: u32,
    tiles_x: u32,
    tiles_y: u32,
    /// Camera distance past which terrain takes the aerial pipeline (DOOM-E).
    pub aerial_split: f32,
    /// Master switch. Off falls back to the single fullscreen draw, which is
    /// the A/B and the escape hatch for the life of the phase.
    pub enabled: bool,
}

impl ClassifyPass {
    /// Bytes the indirect argument block occupies.
    const ARGS_BYTES: u64 = (BIN_COUNT * std::mem::size_of::<DrawArgs>()) as u64;

    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        global_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        // MORROWIND-C: composition is declared in `classify.wgsl` and
        // resolved by `somnium_shader`; this site no longer knows the order.
        let source = shaders.source_or_panic("classify.wgsl");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("classify.wgsl"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });

        let storage = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Classify BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                storage(2),
                storage(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Classify PL"),
            bind_group_layouts: &[Some(global_layout), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Classify"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let draw_args = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Classify draw args"),
            size: Self::ARGS_BYTES,
            usage: wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Sized on first use. Zero-length buffers are illegal, so it starts at
        // one bin's worth of a small screen and grows.
        let tiles = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Classify tiles"),
            size: (BIN_COUNT * 1024 * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Classify params"),
            size: std::mem::size_of::<ClassifyParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            layout,
            draw_args,
            tiles,
            params,
            bind_group: None,
            width: 0,
            height: 0,
            tiles_x: 0,
            tiles_y: 0,
            // 150 m. DOOM-B's buckets put 63.9% of Coastal ground inside 100 m
            // and 54.9% of the overview between 100 and 400 m, so a split in
            // that gap is where the two viewpoints actually differ. Tunable in
            // Terrain details because the right value is a look decision.
            //
            // `SOMNIUM_AERIAL_SPLIT` exists for one specific job: pushing it
            // past the far plane makes every terrain tile take the near (full)
            // pipeline, which turns the binned path into an exact
            // reimplementation of the fullscreen one — and that is the only way
            // to prove DOOM-C is parity-clean separately from DOOM-E, whose
            // whole purpose is to change those pixels.
            aerial_split: std::env::var("SOMNIUM_AERIAL_SPLIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &f32| v.is_finite() && *v > 0.0)
                .unwrap_or(150.0),
            enabled: std::env::var("SOMNIUM_SHADE_BINS").as_deref() == Ok("1"),
        }
    }

    pub fn tile_count(&self) -> u32 {
        self.tiles_x * self.tiles_y
    }

    pub fn tiles_x(&self) -> u32 {
        self.tiles_x
    }

    pub fn draw_args_buffer(&self) -> &wgpu::Buffer {
        &self.draw_args
    }

    pub fn tiles_buffer(&self) -> &wgpu::Buffer {
        &self.tiles
    }

    /// Byte offset of bin `bin`'s indirect argument.
    pub fn args_offset(bin: usize) -> u64 {
        (bin * std::mem::size_of::<DrawArgs>()) as u64
    }

    pub fn invalidate(&mut self) {
        self.bind_group = None;
    }

    /// Allocate for a viewport size and rebuild the bind group.
    ///
    /// Returns true when the tile buffer was reallocated, which the shading
    /// pass needs to know because its own bind group holds a view of it.
    pub fn ensure(
        &mut self,
        device: &wgpu::Device,
        vis_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> bool {
        if self.bind_group.is_some() && self.width == width && self.height == height {
            return false;
        }
        self.width = width;
        self.height = height;
        self.tiles_x = width.div_ceil(TILE_SIZE);
        self.tiles_y = height.div_ceil(TILE_SIZE);

        let needed = (BIN_COUNT as u64) * u64::from(self.tile_count().max(1)) * 4;
        let grew = needed > self.tiles.size();
        if grew {
            self.tiles = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Classify tiles"),
                size: needed,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
        }

        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Classify BG"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(vis_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.draw_args.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.tiles.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.params.as_entire_binding(),
                },
            ],
        }));
        grew
    }

    /// Reset the counters and classify this frame's tiles.
    pub fn record(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        global_bind_group: &wgpu::BindGroup,
    ) {
        if !self.enabled || self.width == 0 {
            return;
        }
        let Some(bind_group) = &self.bind_group else {
            return;
        };

        // Rewritten rather than cleared: `vertex_count` has to be six every
        // frame (two triangles), and a `clear_buffer` would zero it as well and
        // silently draw nothing at all.
        let reset = [DrawArgs {
            vertex_count: 6,
            instance_count: 0,
            first_vertex: 0,
            first_instance: 0,
        }; BIN_COUNT];
        queue.write_buffer(&self.draw_args, 0, bytemuck::cast_slice(&reset));
        queue.write_buffer(
            &self.params,
            0,
            bytemuck::bytes_of(&ClassifyParams {
                width: self.width,
                height: self.height,
                tiles_x: self.tiles_x,
                tile_capacity: self.tile_count(),
                aerial_split: self.aerial_split,
                tile_size: TILE_SIZE,
                _pad: [0; 2],
            }),
        );

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Classify"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, global_bind_group, &[]);
        pass.set_bind_group(1, bind_group, &[]);
        // One workgroup per tile: `@workgroup_size(8, 8)` and `TILE_SIZE` are
        // the same eight, which is what makes `workgroup_id` the tile index.
        pass.dispatch_workgroups(self.tiles_x, self.tiles_y, 1);
    }
}

/// The census and the classifier must agree about what a pixel is.
///
/// They apply different thresholds — three distance buckets against one aerial
/// split — but both call `pc_classify`, and this is the compile-time reminder
/// that the census's extra bucket is a threshold difference and not a second
/// taxonomy.
const _: () = assert!(CENSUS_BINS == 7 && BIN_COUNT == 6);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tile_is_an_even_number_of_pixels() {
        // Odd tiles would let a 2×2 derivative quad straddle two tiles drawn by
        // different pipelines, which is exactly the class of bug 25N exists to
        // fix and would be far harder to see.
        assert_eq!(TILE_SIZE % 2, 0);
    }

    #[test]
    fn every_bin_has_a_name_and_the_indices_are_distinct() {
        assert_eq!(BIN_NAMES.len(), BIN_COUNT);
        let mut seen = [
            BIN_SKY,
            BIN_MESH,
            BIN_FOLIAGE,
            BIN_TERRAIN_NEAR,
            BIN_TERRAIN_AERIAL,
            BIN_MIXED,
        ];
        seen.sort_unstable();
        assert_eq!(seen, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn argument_offsets_tile_the_block_without_gaps() {
        for bin in 0..BIN_COUNT {
            assert_eq!(ClassifyPass::args_offset(bin), (bin * 16) as u64);
        }
        assert_eq!(ClassifyPass::ARGS_BYTES, (BIN_COUNT * 16) as u64);
    }
}
