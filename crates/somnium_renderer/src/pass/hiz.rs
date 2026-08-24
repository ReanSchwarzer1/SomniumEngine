//! Phase 15E: builds the Hi-Z depth pyramid each frame.
//!
//! Owns an R32Float mip chain the size of the viewport. Level 0 is a copy of the
//! visibility pass's depth buffer; every level above holds the furthest depth of
//! the four texels below it. See `shaders/hiz.wgsl` for why `max` is the correct
//! reduction and how odd mip sizes are handled.
//!
//! The pyramid is rebuilt whenever the viewport resizes, which is also when the
//! per-mip views and bind groups have to be recreated — they are cached rather
//! than rebuilt per frame, since neither the texture nor the depth view moves
//! between resizes.

/// Storage format for the pyramid. `r32float` is guaranteed to support
/// write-only storage binding, which `depth32float` is not — hence the copy.
const HIZ_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

pub struct HiZPass {
    copy_pipeline: wgpu::ComputePipeline,
    down_pipeline: wgpu::ComputePipeline,
    copy_layout: wgpu::BindGroupLayout,
    down_layout: wgpu::BindGroupLayout,

    texture: wgpu::Texture,
    /// A view of the whole chain, for consumers that sample a chosen level.
    pub view: wgpu::TextureView,
    /// One single-mip view per level, used as the compute source/destination.
    mip_views: Vec<wgpu::TextureView>,
    /// `bind_groups[i]` produces mip `i`. Index 0 reads the depth buffer.
    bind_groups: Vec<wgpu::BindGroup>,
    /// Phase 24AC: builds levels 1.. in a couple of dispatches instead of one
    /// each. `SOMNIUM_SPD=0` falls back to the per-mip chain, which is the A/B.
    /// `None` when the device grants fewer storage textures per stage than SPD
    /// needs — the per-mip chain is the fallback, not an error.
    spd: Option<crate::pass::spd::SpdPass>,
    use_spd: bool,

    width: u32,
    height: u32,
}

impl HiZPass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        depth_view: &wgpu::TextureView,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Hi-Z Shader"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("hiz.wgsl").into()),
        });

        let copy_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Hi-Z Copy BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: HIZ_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });

        let down_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Hi-Z Downsample BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        // Sampled with textureLoad only, so it never needs to filter.
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: HIZ_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });

        let make_pipeline = |label: &str, layout: &wgpu::BindGroupLayout, entry: &str| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let copy_pipeline = make_pipeline("Hi-Z Copy", &copy_layout, "copy_depth");
        let down_pipeline = make_pipeline("Hi-Z Downsample", &down_layout, "downsample");

        let (texture, view, mip_views, bind_groups) = Self::make_resources(
            device,
            width,
            height,
            depth_view,
            &copy_layout,
            &down_layout,
        );

        // Phase 24AC. The bind-group layout would fail outright on a device
        // with fewer storage textures than the shader declares, so this is
        // checked before anything is created rather than caught after.
        let spd_supported = device.limits().max_storage_textures_per_shader_stage
            >= crate::pass::spd::MIPS_PER_DISPATCH;
        let spd = if spd_supported {
            let mut p = crate::pass::spd::SpdPass::new(device, shaders);
            p.build(
                device,
                queue,
                &mip_views,
                width,
                height,
                mip_count(width, height),
            );
            Some(p)
        } else {
            tracing::info!(
                "Hi-Z: single-pass downsample unavailable (needs {} storage textures per stage)                  — using the per-mip chain",
                crate::pass::spd::MIPS_PER_DISPATCH,
            );
            None
        };
        let use_spd = spd.is_some() && std::env::var("SOMNIUM_SPD").as_deref() != Ok("0");

        Self {
            copy_pipeline,
            down_pipeline,
            copy_layout,
            down_layout,
            texture,
            view,
            mip_views,
            bind_groups,
            width,
            height,
            spd,
            use_spd,
        }
    }

    /// Number of mip levels in the pyramid.
    pub fn mip_count(&self) -> u32 {
        self.texture.mip_level_count()
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Rebuild for a new viewport size. Also required after the depth view is
    /// recreated, since the level-0 bind group holds a reference to it.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        depth_view: &wgpu::TextureView,
    ) {
        let (texture, view, mip_views, bind_groups) = Self::make_resources(
            device,
            width,
            height,
            depth_view,
            &self.copy_layout,
            &self.down_layout,
        );
        self.texture = texture;
        self.view = view;
        self.mip_views = mip_views;
        self.bind_groups = bind_groups;
        self.width = width;
        self.height = height;
        if let Some(spd) = self.spd.as_mut() {
            spd.build(
                device,
                queue,
                &self.mip_views,
                width,
                height,
                mip_count(width, height),
            );
        }
    }

    /// Record the full pyramid build.
    ///
    /// Level 0 is always its own dispatch: a depth texture cannot be bound as a
    /// storage image, so it has to be copied rather than reduced. Above that,
    /// SPD takes six levels per dispatch (Phase 24AC); the fallback path walks
    /// them one at a time, each a pipeline barrier behind the last.
    pub fn record(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Hi-Z Build"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.copy_pipeline);
        pass.set_bind_group(0, &self.bind_groups[0], &[]);
        let (w0, h0) = mip_size(self.width, self.height, 0);
        pass.dispatch_workgroups(w0.div_ceil(8), h0.div_ceil(8), 1);

        if let (true, Some(spd)) = (self.use_spd, self.spd.as_ref()) {
            spd.record(&mut pass);
            return;
        }

        pass.set_pipeline(&self.down_pipeline);
        for (level, bg) in self.bind_groups.iter().enumerate().skip(1) {
            let (w, h) = mip_size(self.width, self.height, level as u32);
            pass.set_bind_group(0, bg, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
    }

    fn make_resources(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        depth_view: &wgpu::TextureView,
        copy_layout: &wgpu::BindGroupLayout,
        down_layout: &wgpu::BindGroupLayout,
    ) -> (
        wgpu::Texture,
        wgpu::TextureView,
        Vec<wgpu::TextureView>,
        Vec<wgpu::BindGroup>,
    ) {
        let width = width.max(1);
        let height = height.max(1);
        let levels = mip_count(width, height);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Hi-Z Pyramid"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HIZ_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mip_views: Vec<wgpu::TextureView> = (0..levels)
            .map(|level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("Hi-Z Mip"),
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();

        let bind_groups = (0..levels)
            .map(|level| {
                let (src, layout) = if level == 0 {
                    (depth_view, copy_layout)
                } else {
                    (&mip_views[level as usize - 1], down_layout)
                };
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Hi-Z BG"),
                    layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(src),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(
                                &mip_views[level as usize],
                            ),
                        },
                    ],
                })
            })
            .collect();

        (texture, view, mip_views, bind_groups)
    }
}

/// Mip levels needed to reach a 1x1 top level.
pub fn mip_count(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

/// Dimensions of `level`, halving and clamping at 1 like the graphics API does.
pub fn mip_size(width: u32, height: u32, level: u32) -> (u32, u32) {
    ((width >> level).max(1), (height >> level).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_power_of_two_viewport_reaches_one_by_one() {
        assert_eq!(mip_count(1024, 1024), 11); // 1024 down to 1
        assert_eq!(mip_size(1024, 1024, 10), (1, 1));
    }

    #[test]
    fn mip_count_follows_the_longer_axis() {
        // The chain has to keep going until BOTH axes are 1, so it is driven by
        // the larger dimension.
        assert_eq!(mip_count(1920, 1080), 11);
        assert_eq!(mip_size(1920, 1080, 10), (1, 1));
    }

    #[test]
    fn non_power_of_two_sizes_still_terminate_at_one() {
        for (w, h) in [(1791u32, 1113u32), (800, 600), (17, 5), (3, 3)] {
            let levels = mip_count(w, h);
            let (lw, lh) = mip_size(w, h, levels - 1);
            assert_eq!((lw, lh), (1, 1), "{w}x{h} ended at {lw}x{lh}");
        }
    }

    #[test]
    fn a_one_by_one_viewport_has_a_single_level() {
        assert_eq!(mip_count(1, 1), 1);
        assert_eq!(mip_size(1, 1, 0), (1, 1));
    }

    #[test]
    fn a_zero_size_viewport_does_not_produce_an_empty_chain() {
        // Guards against a minimised window asking for a texture with zero mips.
        assert_eq!(mip_count(0, 0), 1);
    }

    #[test]
    fn every_level_is_at_least_one_texel() {
        let (w, h) = (1791u32, 1113u32);
        for level in 0..mip_count(w, h) {
            let (lw, lh) = mip_size(w, h, level);
            assert!(lw >= 1 && lh >= 1, "level {level} was {lw}x{lh}");
        }
    }
}
