//! Single-pass downsample (Phase 24AC, SPD half).
//!
//! Builds a mip chain in a handful of dispatches instead of one per level. See
//! `shaders/spd.wgsl` for the technique, the FidelityFX reference, and why the
//! last-workgroup trick is not portable to WGSL.
//!
//! Drives the Hi-Z pyramid. The pyramid's level 0 is still a separate copy pass
//! because a depth texture cannot be bound as a storage image.

/// Mips one dispatch can produce. Fixed by the shader's shared-memory tile.
pub const MIPS_PER_DISPATCH: u32 = 6;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpdParams {
    src_size: [u32; 2],
    mip_count: u32,
    _pad: u32,
}

/// One dispatch: the level it reads, the levels it writes, and its bind group.
struct SpdStage {
    bind_group: wgpu::BindGroup,
    params: wgpu::Buffer,
    groups: (u32, u32),
}

pub struct SpdPass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    stages: Vec<SpdStage>,
}

impl SpdPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let storage = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::R32Float,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let mut entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        for i in 0..MIPS_PER_DISPATCH {
            entries.push(storage(2 + i));
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SPD BGL"),
            entries: &entries,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spd.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/spd.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("SPD PL"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("SPD"),
            layout: Some(&pl),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self { pipeline, layout, stages: Vec::new() }
    }

    /// Plan the dispatches for a pyramid of `levels` levels over `mip_views`.
    ///
    /// Level 0 is assumed already written; this fills 1..levels.
    pub fn build(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mip_views: &[wgpu::TextureView],
        width: u32,
        height: u32,
        levels: u32,
    ) {
        self.stages.clear();
        let mip_size = |level: u32| ((width >> level).max(1), (height >> level).max(1));

        let mut src_level = 0u32;
        while src_level + 1 < levels {
            let remaining = levels - 1 - src_level;
            let count = remaining.min(MIPS_PER_DISPATCH);
            let (sw, sh) = mip_size(src_level);

            let params = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("SPD Params"),
                size: std::mem::size_of::<SpdParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(
                &params,
                0,
                bytemuck::bytes_of(&SpdParams {
                    src_size: [sw, sh],
                    mip_count: count,
                    _pad: 0,
                }),
            );

            let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&mip_views[src_level as usize]),
                },
                wgpu::BindGroupEntry { binding: 1, resource: params.as_entire_binding() },
            ];
            for i in 0..MIPS_PER_DISPATCH {
                // Slots past `count` are never written — the shader's loop stops
                // — but every binding in the layout still has to point at a real
                // view, so the last valid one is repeated.
                let level = (src_level + 1 + i).min(levels - 1);
                entries.push(wgpu::BindGroupEntry {
                    binding: 2 + i,
                    resource: wgpu::BindingResource::TextureView(&mip_views[level as usize]),
                });
            }
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("SPD BG"),
                layout: &self.layout,
                entries: &entries,
            });

            // A workgroup owns a 32x32 tile of the *first output* level, which
            // is a 64x64 tile of the source.
            let (dw, dh) = mip_size(src_level + 1);
            self.stages.push(SpdStage {
                bind_group,
                params,
                groups: (dw.div_ceil(32), dh.div_ceil(32)),
            });

            src_level += count;
        }
    }

    /// Number of dispatches this build takes — the number the phase exists to
    /// bring down, so it is worth being able to assert on.
    pub fn dispatch_count(&self) -> usize {
        self.stages.len()
    }

    pub fn record(&self, pass: &mut wgpu::ComputePass<'_>) {
        pass.set_pipeline(&self.pipeline);
        for stage in &self.stages {
            pass.set_bind_group(0, &stage.bind_group, &[]);
            pass.dispatch_workgroups(stage.groups.0, stage.groups.1, 1);
        }
    }
}

/// Dispatches SPD needs for a pyramid of `levels` levels, level 0 excluded.
///
/// Pure, so the thing the phase is measured by can be tested without a GPU.
#[must_use]
pub fn dispatches_for(levels: u32) -> u32 {
    if levels <= 1 {
        return 0;
    }
    (levels - 1).div_ceil(MIPS_PER_DISPATCH)
}

#[cfg(test)]
mod tests {
    use super::{dispatches_for, MIPS_PER_DISPATCH};

    #[test]
    fn a_720p_pyramid_takes_two_dispatches_instead_of_ten() {
        // 1280x720 needs 11 levels. The old builder issued one dispatch per
        // level after the copy — ten of them, each a pipeline barrier behind
        // the last. This is the number the phase exists to reduce.
        let levels = 11;
        assert_eq!(dispatches_for(levels), 2);
        assert!(dispatches_for(levels) < levels - 1);
    }

    #[test]
    fn six_mips_fit_in_one_dispatch() {
        assert_eq!(dispatches_for(1 + MIPS_PER_DISPATCH), 1);
        assert_eq!(dispatches_for(2 + MIPS_PER_DISPATCH), 2);
    }

    #[test]
    fn a_single_level_pyramid_needs_no_work() {
        assert_eq!(dispatches_for(1), 0);
        assert_eq!(dispatches_for(0), 0);
    }

    #[test]
    fn every_level_above_zero_is_covered() {
        // The property that matters: no level may be skipped, or occlusion
        // culling reads an uninitialised mip and rejects visible geometry.
        for levels in 1..20u32 {
            let covered = dispatches_for(levels) * MIPS_PER_DISPATCH;
            assert!(
                covered >= levels - 1,
                "{levels} levels left {} uncovered",
                levels - 1 - covered.min(levels - 1)
            );
        }
    }
}
