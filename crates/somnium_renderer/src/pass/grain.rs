//! DREAMS-B's shared spatiotemporal sampling masks.
//!
//! A deterministic progressive rank tile is expanded to 64 temporal slices.
//! Vulkan uses the authored Slang compute shader; adapters without passthrough
//! shaders receive the same logical resource from a CPU cook. The fallback is
//! what keeps Slang an optional capability rather than a new required feature.

const EDGE: u32 = 64;
const SLICES: u32 = 64;
const SEED: u32 = 0x5eed_b10e;

/// DREAMS features ship on; an explicit `0` remains the deterministic A/B rail.
#[must_use]
pub(crate) fn enabled_by_default(name: &str) -> bool {
    std::env::var(name).as_deref() != Ok("0")
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GrainParams {
    edge: u32,
    slices: u32,
    seed: u32,
    _pad: u32,
}

/// One shared mask atlas for every pass that needs stochastic samples.
pub struct GrainMasks {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    packed: wgpu::Buffer,
    cpu_bytes: Vec<u8>,
    packed_frame: u32,
    stf_enabled: bool,
    shared_enabled: bool,
    sampler: wgpu::Sampler,
    slang_generated: bool,
}

impl GrainMasks {
    /// Build the atlas once. The GPU texture stays immutable; the CPU mirror advances per frame.
    #[must_use]
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shaders: &crate::shaders::Shaders,
    ) -> Self {
        use wgpu::util::DeviceExt as _;

        let seed = std::env::var("SOMNIUM_DREAMS_SEED")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(SEED);
        let ranks = progressive_ranks(EDGE, seed);
        let cpu_bytes = atlas_bytes(&ranks, seed);
        let packed = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("DREAMS-B packed grain masks"),
            // One 64x64 slice is 16 KiB, well below the portable uniform
            // binding ceiling. Shading is already at both sampled-texture and
            // storage-buffer limits; the uniform mirror costs neither.
            contents: &cpu_bytes[..(EDGE * EDGE * 4) as usize],
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("DREAMS-B grain masks"),
            size: wgpu::Extent3d {
                width: EDGE,
                height: EDGE,
                depth_or_array_layers: SLICES,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("DREAMS-B grain mask array"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("DREAMS-B grain sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let slang_generated = if device
            .features()
            .contains(wgpu::Features::PASSTHROUGH_SHADERS)
        {
            match shaders.slang_module(device, "dreams_grain.slang") {
                Ok(module) => {
                    build_on_gpu(device, queue, &module, &ranks, &view, seed);
                    true
                }
                Err(error) => {
                    tracing::warn!("Slang grain cook rejected; using CPU atlas: {error}");
                    upload_cpu(queue, &texture, &cpu_bytes);
                    false
                }
            }
        } else {
            upload_cpu(queue, &texture, &cpu_bytes);
            false
        };

        Self {
            _texture: texture,
            view,
            packed,
            cpu_bytes,
            packed_frame: 0,
            stf_enabled: enabled_by_default("SOMNIUM_DREAMS_STF"),
            shared_enabled: enabled_by_default("SOMNIUM_DREAMS_GRAIN"),
            sampler,
            slang_generated,
        }
    }

    /// Array view: 64x64 texels by 64 temporal slices.
    #[must_use]
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Nearest, toroidally repeating sampler for the rank masks.
    #[must_use]
    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// Packed RGBA8 spatial mirror for stages already at sampled/storage limits.
    #[must_use]
    pub fn packed(&self) -> &wgpu::Buffer {
        &self.packed
    }

    /// Advance terrain's portable uniform mirror by one temporal slice.
    pub fn advance_packed(&mut self, queue: &wgpu::Queue) {
        if !self.stf_enabled {
            return;
        }
        let slice_bytes = (EDGE * EDGE * 4) as usize;
        let slice = (self.packed_frame & (SLICES - 1)) as usize;
        let start = slice * slice_bytes;
        queue.write_buffer(&self.packed, 0, &self.cpu_bytes[start..start + slice_bytes]);
        self.packed_frame = self.packed_frame.wrapping_add(1);
    }

    /// Whether terrain stochastic filtering was armed at startup.
    #[must_use]
    pub fn stf_enabled(&self) -> bool {
        self.stf_enabled
    }

    /// Whether the shared consumer substitution was armed at startup.
    #[must_use]
    pub fn shared_enabled(&self) -> bool {
        self.shared_enabled
    }

    /// Change the shared-pass substitution without rebuilding the atlas.
    pub fn set_shared_enabled(&mut self, enabled: bool) {
        self.shared_enabled = enabled;
    }

    /// Change terrain stochastic filtering without rebuilding its pipeline.
    pub fn set_stf_enabled(&mut self, enabled: bool) {
        self.stf_enabled = enabled;
    }

    /// Whether this adapter executed the Slang cook rather than the CPU fallback.
    #[must_use]
    pub fn slang_generated(&self) -> bool {
        self.slang_generated
    }
}

/// CPU mirror of the atlas' first two channels for projection jitter.
///
/// TAA needs the offset before command recording, so sampling the GPU texture
/// there would add a readback dependency. Keeping this tiny mirror beside the
/// atlas generator makes the sequence one resource with two representations,
/// rather than a second noise system hidden in TAA.
#[must_use]
pub fn jitter(frame: u32) -> glam::Vec2 {
    let seed = std::env::var("SOMNIUM_DREAMS_SEED")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(SEED);
    let slice = frame & (SLICES - 1);
    let x = (mix_bits(seed.wrapping_add(slice.wrapping_mul(67))) & 0xffff) as f32 / 65_536.0;
    let y = (mix_bits(seed.wrapping_add(slice.wrapping_mul(67)).wrapping_add(131)) & 0xffff) as f32
        / 65_536.0;
    glam::Vec2::new(x - 0.5, y - 0.5)
}

fn build_on_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    module: &wgpu::ShaderModule,
    ranks: &[u32],
    view: &wgpu::TextureView,
    seed: u32,
) {
    use wgpu::util::DeviceExt as _;

    let ranks = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("DREAMS-B progressive ranks"),
        contents: bytemuck::cast_slice(ranks),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("DREAMS-B grain parameters"),
        contents: bytemuck::bytes_of(&GrainParams {
            edge: EDGE,
            slices: SLICES,
            seed,
            _pad: 0,
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("DREAMS-B grain BGL"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
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
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                },
                count: None,
            },
        ],
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("DREAMS-B grain BG"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: ranks.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(view),
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("DREAMS-B grain PL"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("DREAMS-B grain mask cook (Slang)"),
        layout: Some(&pipeline_layout),
        module,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("DREAMS-B grain cook"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("DREAMS-B grain cook"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, Some(&bind), &[]);
        pass.dispatch_workgroups(EDGE.div_ceil(8), EDGE.div_ceil(8), SLICES);
    }
    queue.submit([encoder.finish()]);
}

fn atlas_bytes(ranks: &[u32], seed: u32) -> Vec<u8> {
    let mut bytes = vec![0_u8; (EDGE * EDGE * SLICES * 4) as usize];
    for slice in 0..SLICES {
        for y in 0..EDGE {
            for x in 0..EDGE {
                let pixel = ((slice * EDGE * EDGE + y * EDGE + x) * 4) as usize;
                for channel in 0..4_u32 {
                    let offset = toroidal_offset(slice, channel, seed);
                    let sx = (x + offset.0) & (EDGE - 1);
                    let sy = (y + offset.1) & (EDGE - 1);
                    let rank = ranks[(sy * EDGE + sx) as usize];
                    let rotation = mix_bits(
                        seed.wrapping_add(slice.wrapping_mul(67))
                            .wrapping_add(channel.wrapping_mul(131)),
                    ) & 0xffff;
                    let value = ((rank as f32 + 0.5) / (EDGE * EDGE) as f32
                        + rotation as f32 / 65_536.0)
                        .fract();
                    bytes[pixel + channel as usize] = (value * 255.0).round() as u8;
                }
            }
        }
    }
    bytes
}

fn upload_cpu(queue: &wgpu::Queue, texture: &wgpu::Texture, bytes: &[u8]) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(EDGE * 4),
            rows_per_image: Some(EDGE),
        },
        wgpu::Extent3d {
            width: EDGE,
            height: EDGE,
            depth_or_array_layers: SLICES,
        },
    );
}

fn mix_bits(mut value: u32) -> u32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^ (value >> 16)
}

fn toroidal_offset(slice: u32, channel: u32, seed: u32) -> (u32, u32) {
    let bits = mix_bits(seed ^ slice.wrapping_mul(0x9e37_79b9) ^ channel.wrapping_mul(0x85eb_ca6b));
    (bits & (EDGE - 1), (bits >> 16) & (EDGE - 1))
}

fn progressive_ranks(edge: u32, seed: u32) -> Vec<u32> {
    assert!(edge.is_power_of_two());
    let count = (edge * edge) as usize;
    let mut ranks = vec![u32::MAX; count];
    let mut nearest = vec![u32::MAX; count];
    let first = (mix_bits(seed) as usize) % count;
    let mut selected = first;
    for rank in 0..count as u32 {
        ranks[selected] = rank;
        let px = selected as u32 % edge;
        let py = selected as u32 / edge;
        for index in 0..count {
            if ranks[index] != u32::MAX {
                continue;
            }
            let x = index as u32 % edge;
            let y = index as u32 / edge;
            let dx = x.abs_diff(px).min(edge - x.abs_diff(px));
            let dy = y.abs_diff(py).min(edge - y.abs_diff(py));
            nearest[index] = nearest[index].min(dx * dx + dy * dy);
        }
        if rank + 1 < count as u32 {
            selected = (0..count)
                .filter(|&index| ranks[index] == u32::MAX)
                .max_by_key(|&index| (nearest[index], std::cmp::Reverse(index)))
                .expect("an unranked texel remains");
        }
    }
    ranks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progressive_tile_is_a_permutation_and_reproducible() {
        let a = progressive_ranks(16, SEED);
        let b = progressive_ranks(16, SEED);
        assert_eq!(a, b);
        let mut sorted = a;
        sorted.sort_unstable();
        assert_eq!(sorted, (0..256).collect::<Vec<_>>());
    }

    #[test]
    fn early_thresholds_cover_every_quadrant() {
        let edge = 16;
        let ranks = progressive_ranks(edge, SEED);
        let threshold = edge * edge / 4;
        let mut quadrants = [0_u32; 4];
        for y in 0..edge {
            for x in 0..edge {
                if ranks[(y * edge + x) as usize] < threshold {
                    quadrants[((y >= edge / 2) as usize) * 2 + (x >= edge / 2) as usize] += 1;
                }
            }
        }
        assert!(
            quadrants
                .into_iter()
                .all(|count| (12..=20).contains(&count)),
            "{quadrants:?}"
        );
    }

    #[test]
    fn temporal_channel_offsets_are_not_accidentally_shared() {
        let offsets = [
            toroidal_offset(0, 0, SEED),
            toroidal_offset(0, 1, SEED),
            toroidal_offset(1, 0, SEED),
            toroidal_offset(63, 3, SEED),
        ];
        let mut unique = offsets.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), offsets.len());
    }
}
