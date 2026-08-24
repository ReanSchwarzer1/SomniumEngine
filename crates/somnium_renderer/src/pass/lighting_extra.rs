//! World cache, scene specular GI, path tracer, mesh-SDF, probes (24M/N/O/P/Q).
//!
//! One pass owns the 3-D clipmap and the 2-D aux target the shading pass binds.
//! Individual features are flag-driven and default off except where the
//! inspector turns them on. Hardware without ray query still gets dummy
//! targets so shading's bind group is stable.

const AUX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const VOLUME_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const VOLUME: u32 = 64;

pub const FLAG_CACHE: u32 = 1;
pub const FLAG_SPECULAR: u32 = 2;
pub const FLAG_PATH: u32 = 4;
pub const FLAG_SDF: u32 = 8;
pub const FLAG_PROBES: u32 = 16;

pub const PROBE_GRID: u32 = 4;
pub const SH_COEFFS: u32 = 9;
const SH_BUFFER_BYTES: u64 = (PROBE_GRID * PROBE_GRID * PROBE_GRID * SH_COEFFS * 16) as u64;

/// One mesh contribution to the world SDF clipmap.
pub struct MeshSdfDraw {
    pub model: glam::Mat4,
    pub local_min: [f32; 3],
    pub local_max: [f32; 3],
    pub vertex_offset: u32,
    pub brick: Option<std::sync::Arc<crate::geometry::MeshSdfBrick>>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ExtraParams {
    inv_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 3],
    frame: u32,
    origin: [f32; 3],
    cell_size: f32,
    flags: u32,
    intensity: f32,
    spec_rough: f32,
    path_bounces: u32,
    inv_res: [f32; 2],
    /// Bit 0 = world-cache history, bit 1 = 2-D aux history.
    history_flags: u32,
    half_cells: f32,
    probe_intensity: f32,
    _pad: [f32; 3],
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_params_struct_is_the_144_byte_uniform_layout() {
        assert_eq!(std::mem::size_of::<super::ExtraParams>(), 144);
        assert_eq!(std::mem::size_of::<super::ExtraParams>() % 16, 0);
    }

    #[test]
    fn one_encodes_as_f16_one() {
        assert_eq!(super::f32_to_f16_bits(1.0), 0x3c00);
        assert_eq!(super::f32_to_f16_bits(0.0), 0);
        assert_eq!(super::f32_to_f16_bits(-2.0), 0xc000);
    }

    #[test]
    fn the_sh_probe_buffer_holds_a_4x4x4_l2_grid() {
        assert_eq!(super::SH_BUFFER_BYTES, 64u64 * 9 * 16);
        assert_eq!(super::SH_BUFFER_BYTES % 16, 0);
    }

    #[test]
    fn unique_meshes_are_not_starved_by_instanced_foliage() {
        let foliage = || super::MeshSdfDraw {
            model: glam::Mat4::IDENTITY,
            local_min: [-0.5; 3],
            local_max: [0.5; 3],
            vertex_offset: 10,
            brick: None,
        };
        let cube = super::MeshSdfDraw {
            model: glam::Mat4::from_translation(glam::Vec3::Y),
            local_min: [-1.0; 3],
            local_max: [1.0; 3],
            vertex_offset: 20,
            brick: None,
        };
        // Many copies of one mesh, then a unique cube. The cube must still land
        // in the 256-draw budget or Mesh SDF never sees editor primitives.
        let mut draws: Vec<_> = (0..400).map(|_| foliage()).collect();
        draws.push(cube);
        let selected = super::mesh_sdf_draw_budget(&draws, 256);
        assert!(
            selected.iter().any(|d| d.local_max[0] > 0.75),
            "cube AABB should survive the foliage flood"
        );
        assert!(selected.len() <= 256);
    }

    #[test]
    fn path_history_resets_for_translation_and_rotation() {
        let pos = glam::Vec3::new(0.0, 2.0, 5.0);
        let forward = glam::Vec3::NEG_Z;
        assert!(!super::aux_camera_changed(
            Some(pos.to_array()),
            Some(forward.to_array()),
            pos,
            forward,
        ));
        assert!(super::aux_camera_changed(
            Some(pos.to_array()),
            Some(forward.to_array()),
            pos + glam::Vec3::new(0.01, 0.0, 0.0),
            forward,
        ));
        assert!(super::aux_camera_changed(
            Some(pos.to_array()),
            Some(forward.to_array()),
            pos,
            glam::Vec3::new(0.02, 0.0, -0.9998).normalize(),
        ));
    }
}

fn aux_camera_changed(
    last_position: Option<[f32; 3]>,
    last_forward: Option<[f32; 3]>,
    position: glam::Vec3,
    forward: glam::Vec3,
) -> bool {
    let translated =
        last_position.is_none_or(|p| (glam::Vec3::from_array(p) - position).length() > 0.001);
    let rotated = last_forward.is_none_or(|f| glam::Vec3::from_array(f).dot(forward) < 0.999_99);
    translated || rotated
}

pub struct LightingExtraPass {
    cache_decay: Option<wgpu::ComputePipeline>,
    cache_splat: Option<wgpu::ComputePipeline>,
    specular: Option<wgpu::ComputePipeline>,
    path: Option<wgpu::ComputePipeline>,
    bake_probes: Option<wgpu::ComputePipeline>,
    layout: Option<wgpu::BindGroupLayout>,
    params: Option<wgpu::Buffer>,
    sh_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    volume: wgpu::Texture,
    volume_view: wgpu::TextureView,
    volume_hist: wgpu::Texture,
    volume_hist_view: wgpu::TextureView,
    aux: wgpu::Texture,
    aux_view: wgpu::TextureView,
    specular_hist: wgpu::Texture,
    specular_hist_view: wgpu::TextureView,
    path_hist: wgpu::Texture,
    path_hist_view: wgpu::TextureView,
    supported: bool,
    cache_history_valid: bool,
    specular_history_valid: bool,
    path_history_valid: bool,
    frame: u32,
    last_camera: Option<[f32; 3]>,
    last_camera_forward: Option<[f32; 3]>,
    last_flags: u32,
    last_cache_origin: Option<[f32; 3]>,
    last_aux_settings: Option<[u32; 3]>,
    last_projection: Option<[f32; 16]>,
    last_scene_revision: Option<u64>,
    pub flags: u32,
    pub cell_size: f32,
    pub intensity: f32,
    pub probe_intensity: f32,
    pub spec_rough: f32,
    pub path_bounces: u32,
    sdf_cpu: Vec<[f32; 4]>,
}

impl LightingExtraPass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        global_layout: &wgpu::BindGroupLayout,
        supported: bool,
        width: u32,
        height: u32,
    ) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Lighting extra sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let (volume, volume_view) = volume_tex(device, "World cache");
        let (volume_hist, volume_hist_view) = volume_tex(device, "World cache history");
        let (aux, aux_view) = aux_tex(device, width, height, "Lighting aux");
        let (specular_hist, specular_hist_view) =
            aux_tex(device, width, height, "Scene specular history");
        let (path_hist, path_hist_view) = aux_tex(device, width, height, "Path tracer history");

        let sh_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("SH probes"),
            size: SH_BUFFER_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut pass = Self {
            cache_decay: None,
            cache_splat: None,
            specular: None,
            path: None,
            bake_probes: None,
            layout: None,
            params: None,
            sh_buffer,
            sampler,
            volume,
            volume_view,
            volume_hist,
            volume_hist_view,
            aux,
            aux_view,
            specular_hist,
            specular_hist_view,
            path_hist,
            path_hist_view,
            supported,
            cache_history_valid: false,
            specular_history_valid: false,
            path_history_valid: false,
            frame: 0,
            last_camera: None,
            last_camera_forward: None,
            last_flags: 0,
            last_cache_origin: None,
            last_aux_settings: None,
            last_projection: None,
            last_scene_revision: None,
            flags: 0,
            cell_size: 2.0,
            intensity: 1.0,
            probe_intensity: 1.0,
            spec_rough: 0.15,
            path_bounces: 3,
            sdf_cpu: vec![[1.0e3; 4]; (VOLUME * VOLUME * VOLUME) as usize],
        };

        if !supported {
            return pass;
        }

        // MORROWIND-C: composition is declared in `lighting_extra.wgsl` and
        // resolved by `somnium_shader`; this site no longer knows the order.
        let source = shaders.source_or_panic("lighting_extra.wgsl");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lighting_extra.wgsl"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Lighting extra BGL"),
            entries: &[
                accel_entry(0),
                depth_entry(1),
                uint_tex(2),
                float_tex(3),
                cube_entry(4),
                sampler_entry(5),
                float_tex3(6),
                storage_3d(7),
                float_tex(8),
                storage_2d(9),
                uniform_entry(10),
                sampler_entry(11),
                storage_rw(12),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Lighting extra PL"),
            bind_group_layouts: &[Some(global_layout), Some(&layout)],
            immediate_size: 0,
        });
        let make = |entry: &str, label: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        pass.cache_decay = Some(make("cache_splat", "World cache decay"));
        pass.cache_splat = Some(make("cache_from_screen", "World cache splat"));
        pass.specular = Some(make("specular_gi", "Scene specular GI"));
        pass.path = Some(make("path_trace", "Path tracer"));
        pass.bake_probes = Some(make("bake_probes", "SH probes"));
        pass.layout = Some(layout);
        pass.params = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Lighting extra params"),
            size: std::mem::size_of::<ExtraParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        pass
    }

    pub fn supported(&self) -> bool {
        self.supported
    }

    pub fn aux_view(&self) -> &wgpu::TextureView {
        &self.aux_view
    }

    pub fn volume_view(&self) -> &wgpu::TextureView {
        &self.volume_view
    }

    pub fn sh_buffer(&self) -> &wgpu::Buffer {
        &self.sh_buffer
    }

    pub fn flags_bits(&self) -> u32 {
        self.flags
    }

    /// Number of consecutive samples accumulated by the active 2-D traced
    /// estimator. Exposed for capture audits so a reset-every-frame regression
    /// is visible without inferring it from image noise.
    pub fn accumulated_frames(&self) -> u32 {
        self.frame
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (aux, aux_view) = aux_tex(device, width, height, "Lighting aux");
        let (specular_hist, specular_hist_view) =
            aux_tex(device, width, height, "Scene specular history");
        let (path_hist, path_hist_view) = aux_tex(device, width, height, "Path tracer history");
        self.aux = aux;
        self.aux_view = aux_view;
        self.specular_hist = specular_hist;
        self.specular_hist_view = specular_hist_view;
        self.path_hist = path_hist;
        self.path_hist_view = path_hist_view;
        self.specular_history_valid = false;
        self.path_history_valid = false;
        self.frame = 0;
    }

    pub fn shading_params(&self) -> [f32; 4] {
        [
            f32::from_bits(self.flags),
            self.intensity,
            self.cell_size,
            VOLUME as f32 * 0.5,
        ]
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        global_bind: &wgpu::BindGroup,
        tlas: Option<&wgpu::Tlas>,
        depth_view: &wgpu::TextureView,
        vis_view: &wgpu::TextureView,
        gi_view: &wgpu::TextureView,
        env_view: &wgpu::TextureView,
        env_sampler: &wgpu::Sampler,
        view_proj: glam::Mat4,
        view: glam::Mat4,
        projection: glam::Mat4,
        scene_revision: u64,
        camera_pos: glam::Vec3,
        width: u32,
        height: u32,
        mesh_sdf: &[MeshSdfDraw],
    ) {
        if self.flags == 0 {
            self.cache_history_valid = false;
            self.specular_history_valid = false;
            self.path_history_valid = false;
            self.frame = 0;
            self.last_flags = 0;
            return;
        }

        let camera_forward = view
            .inverse()
            .transform_vector3(glam::Vec3::NEG_Z)
            .normalize_or_zero();
        let camera_changed = aux_camera_changed(
            self.last_camera,
            self.last_camera_forward,
            camera_pos,
            camera_forward,
        );
        let aux_mode = self.flags & (FLAG_PATH | FLAG_SPECULAR);
        let previous_aux_mode = self.last_flags & (FLAG_PATH | FLAG_SPECULAR);
        let aux_settings = [
            self.intensity.to_bits(),
            self.spec_rough.to_bits(),
            self.path_bounces.clamp(1, 8),
        ];
        let projection_cols = projection.to_cols_array();
        let projection_changed = self.last_projection.is_none_or(|last| {
            last.iter()
                .zip(projection_cols)
                .any(|(a, b)| (*a - b).abs() > 1e-6)
        });
        let scene_changed = self
            .last_scene_revision
            .is_none_or(|last| last != scene_revision);
        let reset_aux = camera_changed
            || projection_changed
            || scene_changed
            || aux_mode != previous_aux_mode
            || self
                .last_aux_settings
                .is_some_and(|last| last != aux_settings);
        if reset_aux {
            // Histories are physically separate as well as logically invalid.
            // A specular -> path -> specular transition can therefore never
            // sample radiance produced by the other estimator.
            self.specular_history_valid = false;
            self.path_history_valid = false;
            self.frame = 0;
        }
        self.last_camera = Some(camera_pos.to_array());
        self.last_camera_forward = Some(camera_forward.to_array());
        self.last_aux_settings = Some(aux_settings);
        self.last_projection = Some(projection_cols);
        self.last_scene_revision = Some(scene_revision);

        if (self.flags & FLAG_SDF) != 0 && (self.flags & FLAG_CACHE) == 0 {
            self.fill_sdf(queue, camera_pos, mesh_sdf);
        }

        let Some(tlas) = tlas.filter(|_| self.supported) else {
            return;
        };
        let (Some(layout), Some(params), Some(decay), Some(splat), Some(specular), Some(path)) = (
            self.layout.as_ref(),
            self.params.as_ref(),
            self.cache_decay.as_ref(),
            self.cache_splat.as_ref(),
            self.specular.as_ref(),
            self.path.as_ref(),
        ) else {
            return;
        };

        let cell = self.cell_size.max(0.25);
        let origin = (camera_pos / cell).floor() * cell;
        if (self.flags & FLAG_CACHE) == 0 {
            self.cache_history_valid = false;
            self.last_cache_origin = None;
        } else if self
            .last_cache_origin
            .is_some_and(|last| glam::Vec3::from_array(last) != origin)
            || (self.last_flags & FLAG_CACHE) == 0
        {
            // The cache texture is camera-relative. Until it has an explicit
            // scroll/reprojection pass, sampling old UVW under a new snapped
            // origin aliases every voxel to a different world position.
            self.cache_history_valid = false;
        }
        self.last_cache_origin = Some(origin.to_array());
        let aux_history_valid = if (self.flags & FLAG_PATH) != 0 {
            self.path_history_valid
        } else if (self.flags & FLAG_SPECULAR) != 0 {
            self.specular_history_valid
        } else {
            false
        };
        let history_flags =
            u32::from(self.cache_history_valid) | (u32::from(aux_history_valid) << 1);
        self.last_flags = self.flags;
        queue.write_buffer(
            params,
            0,
            bytemuck::bytes_of(&ExtraParams {
                inv_view_proj: view_proj.inverse().to_cols_array_2d(),
                camera_pos: camera_pos.to_array(),
                frame: self.frame,
                origin: origin.to_array(),
                cell_size: cell,
                flags: self.flags,
                intensity: self.intensity,
                spec_rough: self.spec_rough,
                path_bounces: self.path_bounces.max(1).min(8),
                inv_res: [1.0 / width as f32, 1.0 / height as f32],
                history_flags,
                half_cells: VOLUME as f32 * 0.5,
                probe_intensity: self.probe_intensity,
                _pad: [0.0; 3],
            }),
        );

        let aux_history_view = if (self.flags & FLAG_PATH) != 0 {
            &self.path_hist_view
        } else {
            &self.specular_hist_view
        };
        if reset_aux {
            let texture = if (self.flags & FLAG_PATH) != 0 {
                &self.path_hist
            } else {
                &self.specular_hist
            };
            encoder.clear_texture(texture, &wgpu::ImageSubresourceRange::default());
        }

        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Lighting extra"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::AccelerationStructure(tlas),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(vis_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(gi_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(env_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(env_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&self.volume_hist_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&self.volume_view),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(aux_history_view),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&self.aux_view),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: self.sh_buffer.as_entire_binding(),
                },
            ],
        });

        if (self.flags & FLAG_CACHE) != 0 {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("World cache decay"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(decay);
            cpass.set_bind_group(0, Some(global_bind), &[]);
            cpass.set_bind_group(1, Some(&bind), &[]);
            cpass.dispatch_workgroups(VOLUME.div_ceil(8), VOLUME.div_ceil(8), VOLUME);
            drop(cpass);
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("World cache splat"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(splat);
            cpass.set_bind_group(0, Some(global_bind), &[]);
            cpass.set_bind_group(1, Some(&bind), &[]);
            cpass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
            drop(cpass);
            encoder.copy_texture_to_texture(
                self.volume.as_image_copy(),
                self.volume_hist.as_image_copy(),
                wgpu::Extent3d {
                    width: VOLUME,
                    height: VOLUME,
                    depth_or_array_layers: VOLUME,
                },
            );
            self.cache_history_valid = true;
        }

        if (self.flags & FLAG_PROBES) != 0 {
            if let Some(bake) = self.bake_probes.as_ref() {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("SH probes"),
                    timestamp_writes: None,
                });
                cpass.set_pipeline(bake);
                cpass.set_bind_group(0, Some(global_bind), &[]);
                cpass.set_bind_group(1, Some(&bind), &[]);
                cpass.dispatch_workgroups(1, 1, 1);
                drop(cpass);
            }
        }

        let hw = (width / 2).max(1);
        let hh = (height / 2).max(1);
        if (self.flags & FLAG_PATH) != 0 {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Path tracer"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(path);
            cpass.set_bind_group(0, Some(global_bind), &[]);
            cpass.set_bind_group(1, Some(&bind), &[]);
            cpass.dispatch_workgroups(hw.div_ceil(8), hh.div_ceil(8), 1);
            drop(cpass);
            encoder.copy_texture_to_texture(
                self.aux.as_image_copy(),
                self.path_hist.as_image_copy(),
                wgpu::Extent3d {
                    width: hw,
                    height: hh,
                    depth_or_array_layers: 1,
                },
            );
            self.path_history_valid = true;
            self.frame = self.frame.saturating_add(1);
        } else if (self.flags & FLAG_SPECULAR) != 0 {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Scene specular GI"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(specular);
            cpass.set_bind_group(0, Some(global_bind), &[]);
            cpass.set_bind_group(1, Some(&bind), &[]);
            cpass.dispatch_workgroups(hw.div_ceil(8), hh.div_ceil(8), 1);
            drop(cpass);
            encoder.copy_texture_to_texture(
                self.aux.as_image_copy(),
                self.specular_hist.as_image_copy(),
                wgpu::Extent3d {
                    width: hw,
                    height: hh,
                    depth_or_array_layers: 1,
                },
            );
            self.specular_history_valid = true;
            self.frame = self.frame.wrapping_add(1);
        } else {
            self.specular_history_valid = false;
            self.path_history_valid = false;
            self.frame = 0;
        }
    }

    fn fill_sdf(&mut self, queue: &wgpu::Queue, camera_pos: glam::Vec3, draws: &[MeshSdfDraw]) {
        let cell = self.cell_size.max(0.25);
        let origin = (camera_pos / cell).floor() * cell;
        let half = VOLUME as f32 * 0.5;
        for v in &mut self.sdf_cpu {
            *v = [8.0 * cell, 8.0 * cell, 8.0 * cell, 8.0 * cell];
        }
        for draw in mesh_sdf_draw_budget(draws, 256) {
            let local_min = glam::Vec3::from_array(draw.local_min);
            let local_max = glam::Vec3::from_array(draw.local_max);
            let brick_min = draw
                .brick
                .as_ref()
                .map(|b| glam::Vec3::from_array(b.min))
                .unwrap_or(local_min);
            let brick_max = draw
                .brick
                .as_ref()
                .map(|b| glam::Vec3::from_array(b.max))
                .unwrap_or(local_max);
            let mut world_min = glam::Vec3::splat(f32::INFINITY);
            let mut world_max = glam::Vec3::splat(f32::NEG_INFINITY);
            for &x in &[brick_min.x, brick_max.x] {
                for &y in &[brick_min.y, brick_max.y] {
                    for &z in &[brick_min.z, brick_max.z] {
                        let w = draw.model.transform_point3(glam::Vec3::new(x, y, z));
                        world_min = world_min.min(w);
                        world_max = world_max.max(w);
                    }
                }
            }
            let inv = draw.model.inverse();
            let expanded_min = world_min - glam::Vec3::splat(cell * 2.0);
            let expanded_max = world_max + glam::Vec3::splat(cell * 2.0);
            let i0 = (((expanded_min - origin) / cell) + glam::Vec3::splat(half))
                .floor()
                .as_ivec3();
            let i1 = (((expanded_max - origin) / cell) + glam::Vec3::splat(half))
                .ceil()
                .as_ivec3();
            let x0 = i0.x.clamp(0, VOLUME as i32 - 1);
            let y0 = i0.y.clamp(0, VOLUME as i32 - 1);
            let z0 = i0.z.clamp(0, VOLUME as i32 - 1);
            let x1 = i1.x.clamp(0, VOLUME as i32 - 1);
            let y1 = i1.y.clamp(0, VOLUME as i32 - 1);
            let z1 = i1.z.clamp(0, VOLUME as i32 - 1);
            for z in z0..=z1 {
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let p = origin
                            + (glam::Vec3::new(x as f32, y as f32, z as f32)
                                - glam::Vec3::splat(half)
                                + glam::Vec3::splat(0.5))
                                * cell;
                        let d = if let Some(brick) = draw.brick.as_deref() {
                            let local = inv.transform_point3(p);
                            crate::geometry::sample_mesh_sdf(brick, local).unwrap_or_else(|| {
                                aabb_signed_distance(p, world_min, world_max).abs()
                            })
                        } else {
                            aabb_signed_distance(p, world_min, world_max)
                        };
                        let idx =
                            (z as u32 * VOLUME * VOLUME + y as u32 * VOLUME + x as u32) as usize;
                        if d < self.sdf_cpu[idx][3] {
                            self.sdf_cpu[idx] = [0.0, 0.0, 0.0, d];
                        }
                    }
                }
            }
        }
        let mut bytes = vec![0u8; self.sdf_cpu.len() * 8];
        for (i, texel) in self.sdf_cpu.iter().enumerate() {
            for c in 0..4 {
                let h = f32_to_f16_bits(texel[c]);
                bytes[i * 8 + c * 2] = h as u8;
                bytes[i * 8 + c * 2 + 1] = (h >> 8) as u8;
            }
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.volume,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(VOLUME * 8),
                rows_per_image: Some(VOLUME),
            },
            wgpu::Extent3d {
                width: VOLUME,
                height: VOLUME,
                depth_or_array_layers: VOLUME,
            },
        );
    }
}

/// Prefer one instance of every mesh, then extra copies, so 8 000 grass
/// draws cannot push an editor cube out of the 256-splat budget.
fn mesh_sdf_draw_budget(draws: &[MeshSdfDraw], cap: usize) -> Vec<&MeshSdfDraw> {
    let mut selected = Vec::with_capacity(cap.min(draws.len()));
    let mut seen = std::collections::HashSet::new();
    for draw in draws {
        if !seen.insert(draw.vertex_offset) {
            continue;
        }
        selected.push(draw);
        if selected.len() >= cap {
            return selected;
        }
    }
    let mut extra: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for draw in draws {
        let n = extra.entry(draw.vertex_offset).or_insert(0);
        *n += 1;
        if *n == 1 {
            continue;
        }
        if *n > 8 {
            continue;
        }
        selected.push(draw);
        if selected.len() >= cap {
            break;
        }
    }
    selected
}

fn aabb_signed_distance(p: glam::Vec3, min: glam::Vec3, max: glam::Vec3) -> f32 {
    let q = (p - max).max(min - p).max(glam::Vec3::ZERO);
    let outside = q.length();
    let inside = (p - min).min(max - p).min_element().min(0.0);
    if min.cmple(p).all() && p.cmple(max).all() {
        inside
    } else {
        outside
    }
}

/// IEEE-754 binary16 bits for a CPU upload into `Rgba16Float`.
fn f32_to_f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x007f_ffff;
    if exp == 255 {
        return sign | 0x7c00 | u16::from(mant != 0);
    }
    let exp16 = exp - 127 + 15;
    if exp16 >= 31 {
        return sign | 0x7c00;
    }
    if exp16 <= 0 {
        if exp16 < -10 {
            return sign;
        }
        let mant16 = (mant | 0x0080_0000) >> (14 - exp16);
        return sign | mant16 as u16;
    }
    sign | ((exp16 as u16) << 10) | (mant >> 13) as u16
}

fn volume_tex(device: &wgpu::Device, label: &'static str) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: VOLUME,
            height: VOLUME,
            depth_or_array_layers: VOLUME,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: VOLUME_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn aux_tex(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    label: &'static str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let w = (width / 2).max(1);
    let h = (height / 2).max(1);
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: AUX_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn accel_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::AccelerationStructure {
            vertex_return: false,
        },
        count: None,
    }
}
fn depth_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
fn uint_tex(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Uint,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
fn float_tex(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
fn float_tex3(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D3,
            multisampled: false,
        },
        count: None,
    }
}
fn cube_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::Cube,
            multisampled: false,
        },
        count: None,
    }
}
fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}
fn storage_3d(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: VOLUME_FORMAT,
            view_dimension: wgpu::TextureViewDimension::D3,
        },
        count: None,
    }
}
fn storage_2d(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: AUX_FORMAT,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}
fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
fn storage_rw(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
