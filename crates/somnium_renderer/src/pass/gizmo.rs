//! Phase 11.5B: Editor transform gizmo render pass.
//!
//! Procedurally generates vertex/index geometry for Translate, Rotate, and Scale
//! gizmos. All three modes live in one vertex + one index buffer; draw calls
//! select the appropriate index range. Rendered to the swapchain AFTER tone
//! mapping so gizmo colors are always crisp and predictable.
#![allow(clippy::cast_possible_truncation)]

use wgpu;

/// Which transform operation the gizmo is manipulating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GizmoMode {
    Translate,
    Rotate,
    Scale,
}

/// Which axis the user is dragging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GizmoAxis {
    X,
    Y,
    Z,
}

impl GizmoAxis {
    pub fn world_dir(self) -> glam::Vec3 {
        match self {
            GizmoAxis::X => glam::Vec3::X,
            GizmoAxis::Y => glam::Vec3::Y,
            GizmoAxis::Z => glam::Vec3::Z,
        }
    }
}

// ─── Vertex ───────────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct GizmoVertex {
    position: [f32; 3],
    color: [f32; 3],
}

const VERTEX_SIZE: u64 = std::mem::size_of::<GizmoVertex>() as u64;

// ─── Geometry generation ──────────────────────────────────────────────────────

const RED: [f32; 3] = [0.90, 0.20, 0.20];
const GREEN: [f32; 3] = [0.20, 0.90, 0.20];
const BLUE: [f32; 3] = [0.20, 0.20, 0.90];

/// Append an arrow along `dir` (shaft + 8-sided cone) to the geometry vectors.
/// All units are in gizmo-local space where the gizmo fits inside a unit sphere.
fn push_arrow(verts: &mut Vec<GizmoVertex>, inds: &mut Vec<u32>, dir: glam::Vec3, color: [f32; 3]) {
    const SHAFT_START: f32 = 0.13;
    const SHAFT_END: f32 = 0.78;
    const SHAFT_W: f32 = 0.04;
    const CONE_START: f32 = 0.78;
    const CONE_END: f32 = 1.0;
    const CONE_R: f32 = 0.09;
    const SIDES: u32 = 8;

    let rot = glam::Quat::from_rotation_arc(glam::Vec3::X, dir);
    let base = verts.len() as u32;

    // Shaft: rectangular prism (4 corners at each end = 8 verts).
    for x in [SHAFT_START, SHAFT_END] {
        for (sy, sz) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = rot * glam::Vec3::new(x, sy * SHAFT_W, sz * SHAFT_W);
            verts.push(GizmoVertex {
                position: p.to_array(),
                color,
            });
        }
    }
    // 4 side quads (skip end caps — shaft obscures them).
    for i in 0..4u32 {
        let j = (i + 1) % 4;
        let (a, b, c, d) = (base + i, base + j, base + 4 + j, base + 4 + i);
        inds.extend_from_slice(&[a, b, c, a, c, d]);
    }

    // Cone: 8 base ring verts + 1 apex.
    let cone_base = verts.len() as u32;
    for i in 0..SIDES {
        let angle = std::f32::consts::TAU * i as f32 / SIDES as f32;
        let p = rot * glam::Vec3::new(CONE_START, CONE_R * angle.cos(), CONE_R * angle.sin());
        verts.push(GizmoVertex {
            position: p.to_array(),
            color,
        });
    }
    let apex_idx = verts.len() as u32;
    let apex = rot * glam::Vec3::new(CONE_END, 0.0, 0.0);
    verts.push(GizmoVertex {
        position: apex.to_array(),
        color,
    });

    for i in 0..SIDES {
        let j = (i + 1) % SIDES;
        inds.extend_from_slice(&[apex_idx, cone_base + i, cone_base + j]);
    }
}

/// Append a thin torus ring (36 quads) perpendicular to `normal`.
fn push_ring(
    verts: &mut Vec<GizmoVertex>,
    inds: &mut Vec<u32>,
    normal: glam::Vec3,
    color: [f32; 3],
) {
    const INNER_R: f32 = 0.80;
    const OUTER_R: f32 = 0.88;
    const SEGS: u32 = 36;

    let rot = glam::Quat::from_rotation_arc(glam::Vec3::Z, normal);
    let base = verts.len() as u32;

    for i in 0..SEGS {
        let a = std::f32::consts::TAU * i as f32 / SEGS as f32;
        let (c, s) = (a.cos(), a.sin());
        let pi = rot * glam::Vec3::new(c * INNER_R, s * INNER_R, 0.0);
        let po = rot * glam::Vec3::new(c * OUTER_R, s * OUTER_R, 0.0);
        verts.push(GizmoVertex {
            position: pi.to_array(),
            color,
        });
        verts.push(GizmoVertex {
            position: po.to_array(),
            color,
        });
    }
    for i in 0..SEGS {
        let j = (i + 1) % SEGS;
        let (ii, io) = (base + 2 * i, base + 2 * i + 1);
        let (ji, jo) = (base + 2 * j, base + 2 * j + 1);
        inds.extend_from_slice(&[ii, ji, jo, ii, jo, io]);
    }
}

/// Append an axis line + cube endpoint for the scale gizmo.
fn push_scale_arm(
    verts: &mut Vec<GizmoVertex>,
    inds: &mut Vec<u32>,
    dir: glam::Vec3,
    color: [f32; 3],
) {
    const SHAFT_START: f32 = 0.13;
    const SHAFT_END: f32 = 0.78;
    const SHAFT_W: f32 = 0.04;
    const CUBE_HALF: f32 = 0.065;
    const CUBE_START: f32 = 0.78;
    const CUBE_END: f32 = CUBE_START + CUBE_HALF * 2.0;

    let rot = glam::Quat::from_rotation_arc(glam::Vec3::X, dir);
    let base = verts.len() as u32;

    // Shaft (same as translate).
    for x in [SHAFT_START, SHAFT_END] {
        for (sy, sz) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = rot * glam::Vec3::new(x, sy * SHAFT_W, sz * SHAFT_W);
            verts.push(GizmoVertex {
                position: p.to_array(),
                color,
            });
        }
    }
    for i in 0..4u32 {
        let j = (i + 1) % 4;
        let (a, b, c, d) = (base + i, base + j, base + 4 + j, base + 4 + i);
        inds.extend_from_slice(&[a, b, c, a, c, d]);
    }

    // Cube handle (8 corners).
    let cube_base = verts.len() as u32;
    let h = CUBE_HALF;
    let xs = [CUBE_START, CUBE_END];
    let ys = [-h, h];
    let zs = [-h, h];
    for &x in &xs {
        for &y in &ys {
            for &z in &zs {
                let p = rot * glam::Vec3::new(x, y, z);
                verts.push(GizmoVertex {
                    position: p.to_array(),
                    color,
                });
            }
        }
    }
    // 6 faces (cull_mode: None so winding doesn't matter).
    const FACES: [[u32; 4]; 6] = [
        [0, 1, 3, 2], // -X
        [4, 6, 7, 5], // +X
        [0, 4, 5, 1], // -Y
        [2, 3, 7, 6], // +Y
        [0, 2, 6, 4], // -Z
        [1, 5, 7, 3], // +Z
    ];
    for face in &FACES {
        let (a, b, c, d) = (
            cube_base + face[0],
            cube_base + face[1],
            cube_base + face[2],
            cube_base + face[3],
        );
        inds.extend_from_slice(&[a, b, c, a, c, d]);
    }
}

/// Generate vertex + index data for all three gizmo modes.
/// Returns (verts, inds, translate_range, rotate_range, scale_range).
fn build_gizmo_geometry() -> (
    Vec<GizmoVertex>,
    Vec<u32>,
    std::ops::Range<u32>,
    std::ops::Range<u32>,
    std::ops::Range<u32>,
) {
    let mut verts: Vec<GizmoVertex> = Vec::new();
    let mut inds: Vec<u32> = Vec::new();

    // Translate: 3 arrows.
    let t_start = inds.len() as u32;
    push_arrow(&mut verts, &mut inds, glam::Vec3::X, RED);
    push_arrow(&mut verts, &mut inds, glam::Vec3::Y, GREEN);
    push_arrow(&mut verts, &mut inds, glam::Vec3::Z, BLUE);
    let t_end = inds.len() as u32;

    // Rotate: 3 rings (normal = rotation axis).
    let r_start = inds.len() as u32;
    push_ring(&mut verts, &mut inds, glam::Vec3::X, RED);
    push_ring(&mut verts, &mut inds, glam::Vec3::Y, GREEN);
    push_ring(&mut verts, &mut inds, glam::Vec3::Z, BLUE);
    let r_end = inds.len() as u32;

    // Scale: 3 shaft + cube arms.
    let s_start = inds.len() as u32;
    push_scale_arm(&mut verts, &mut inds, glam::Vec3::X, RED);
    push_scale_arm(&mut verts, &mut inds, glam::Vec3::Y, GREEN);
    push_scale_arm(&mut verts, &mut inds, glam::Vec3::Z, BLUE);
    let s_end = inds.len() as u32;

    (verts, inds, t_start..t_end, r_start..r_end, s_start..s_end)
}

// ─── GizmoPass ────────────────────────────────────────────────────────────────

/// Render pass that draws the active transform gizmo over the swapchain output.
pub struct GizmoPass {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    /// 64-byte storage buffer holding the gizmo model matrix (updated each frame).
    params_buffer: wgpu::Buffer,
    translate_indices: std::ops::Range<u32>,
    rotate_indices: std::ops::Range<u32>,
    scale_indices: std::ops::Range<u32>,
}

impl GizmoPass {
    /// Create the gizmo pass.
    ///
    /// - `surface_format`: swapchain format (written to after post-process).
    /// - `view_buffer`:    the 208-byte global view buffer.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        view_buffer: &wgpu::Buffer,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Gizmo Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Params buffer: 64 bytes (one Mat4), identity at start.
        let params_buffer = {
            let identity = glam::Mat4::IDENTITY.to_cols_array();
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Gizmo Params Buffer"),
                size: 64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: true,
            });
            buf.slice(..)
                .get_mapped_range_mut()
                .expect("mapped_at_creation")
                .copy_from_slice(bytemuck::bytes_of(&identity));
            buf.unmap();
            buf
        };

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Gizmo Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: view_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        });

        // Generate all gizmo geometry.
        let (geo_verts, geo_inds, t_range, r_range, s_range) = build_gizmo_geometry();

        let vb_data: &[u8] = bytemuck::cast_slice(&geo_verts);
        let vertex_buffer = {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Gizmo Vertex Buffer"),
                size: vb_data.len() as u64,
                usage: wgpu::BufferUsages::VERTEX,
                mapped_at_creation: true,
            });
            buf.slice(..)
                .get_mapped_range_mut()
                .expect("mapped_at_creation")
                .copy_from_slice(vb_data);
            buf.unmap();
            buf
        };

        let ib_data: &[u8] = bytemuck::cast_slice(&geo_inds);
        let index_buffer = {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Gizmo Index Buffer"),
                size: ib_data.len() as u64,
                usage: wgpu::BufferUsages::INDEX,
                mapped_at_creation: true,
            });
            buf.slice(..)
                .get_mapped_range_mut()
                .expect("mapped_at_creation")
                .copy_from_slice(ib_data);
            buf.unmap();
            buf
        };

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Gizmo Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/gizmo.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Gizmo Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Gizmo Pipeline"),
            layout: Some(&pipeline_layout),
            multiview_mask: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: VERTEX_SIZE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                    ],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            cache: None,
        });

        Self {
            pipeline,
            bind_group,
            vertex_buffer,
            index_buffer,
            params_buffer,
            translate_indices: t_range,
            rotate_indices: r_range,
            scale_indices: s_range,
        }
    }

    /// Upload the gizmo model matrix (entity position + screen-size scale).
    pub fn update_transform(&self, queue: &wgpu::Queue, model: glam::Mat4) {
        let data = model.to_cols_array();
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&data));
    }

    /// Record the gizmo render pass into `encoder`, compositing onto `surface_view`.
    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
        mode: GizmoMode,
    ) {
        let index_range = match mode {
            GizmoMode::Translate => self.translate_indices.clone(),
            GizmoMode::Rotate => self.rotate_indices.clone(),
            GizmoMode::Scale => self.scale_indices.clone(),
        };

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Gizmo Pass"),
            multiview_mask: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: surface_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, &self.bind_group, &[]);
        rpass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        rpass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        rpass.draw_indexed(index_range, 0, 0..1);
    }
}

// ─── Picking helpers (CPU-side ray tests) ─────────────────────────────────────

/// Test a ray (in gizmo-local space where the gizmo fits inside a unit sphere)
/// against each axis handle AABB. Returns the closest-hit axis, if any.
pub fn gizmo_hit_test(
    local_origin: glam::Vec3,
    local_dir: glam::Vec3,
    mode: GizmoMode,
) -> Option<GizmoAxis> {
    // Generous AABBs for picking (handles + heads).
    let (x_aabb, y_aabb, z_aabb) = match mode {
        GizmoMode::Translate | GizmoMode::Scale => (
            (
                glam::Vec3::new(0.10, -0.10, -0.10),
                glam::Vec3::new(1.05, 0.10, 0.10),
            ),
            (
                glam::Vec3::new(-0.10, 0.10, -0.10),
                glam::Vec3::new(0.10, 1.05, 0.10),
            ),
            (
                glam::Vec3::new(-0.10, -0.10, 0.10),
                glam::Vec3::new(0.10, 0.10, 1.05),
            ),
        ),
        GizmoMode::Rotate => (
            // Annular region around ring plane: thin slab in X, ring radius ≈ 0.84.
            (
                glam::Vec3::new(-0.10, -0.92, -0.92),
                glam::Vec3::new(0.10, 0.92, 0.92),
            ),
            (
                glam::Vec3::new(-0.92, -0.10, -0.92),
                glam::Vec3::new(0.92, 0.10, 0.92),
            ),
            (
                glam::Vec3::new(-0.92, -0.92, -0.10),
                glam::Vec3::new(0.92, 0.92, 0.10),
            ),
        ),
    };

    let mut best: Option<(GizmoAxis, f32)> = None;

    for (axis, (mn, mx)) in [
        (GizmoAxis::X, x_aabb),
        (GizmoAxis::Y, y_aabb),
        (GizmoAxis::Z, z_aabb),
    ] {
        if let Some(t) = ray_aabb(local_origin, local_dir, mn, mx) {
            if best.map_or(true, |(_, bt)| t < bt) {
                best = Some((axis, t));
            }
        }
    }
    best.map(|(a, _)| a)
}

fn ray_aabb(origin: glam::Vec3, dir: glam::Vec3, mn: glam::Vec3, mx: glam::Vec3) -> Option<f32> {
    let inv = glam::Vec3::new(
        if dir.x.abs() > 1e-9 {
            1.0 / dir.x
        } else {
            f32::INFINITY
        },
        if dir.y.abs() > 1e-9 {
            1.0 / dir.y
        } else {
            f32::INFINITY
        },
        if dir.z.abs() > 1e-9 {
            1.0 / dir.z
        } else {
            f32::INFINITY
        },
    );
    let t1 = (mn - origin) * inv;
    let t2 = (mx - origin) * inv;
    let tmin = t1.min(t2);
    let tmax = t1.max(t2);
    let enter = tmin.x.max(tmin.y).max(tmin.z);
    let exit = tmax.x.min(tmax.y).min(tmax.z);
    if exit >= enter && exit > 0.0 {
        Some(enter.max(0.0))
    } else {
        None
    }
}
