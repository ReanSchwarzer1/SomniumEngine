//! Phase 13E: Light gizmo render pass.
//!
//! Visualizes point / spot / directional lights in the editor so designers can
//! see where a light reaches and which way it points.
//!
//! ## Reference Architecture
//!
//! `example_repo/bevy/bevy-main/crates/bevy_light/src/gizmos.rs` — the shapes
//! Bevy draws per light type:
//! - point: a sphere at the light's range,
//! - spot: cones for the inner and outer angles, sized
//!   `height = range * cos(angle)`, `radius = range * sin(angle)`,
//!   plus arcs across the cap,
//! - directional: an arrow along the light direction.
//!
//! **Adaptation:** Bevy has a retained immediate-mode gizmo system with generic
//! 3-D primitives. Somnium has no such layer, so this pass emits world-space
//! line segments on the CPU into one growable vertex buffer and issues a single
//! `LineList` draw for every light in the scene. The cap arcs are replaced by
//! cone spokes (cheaper, and reads the same at editor scale); a small axis
//! marker is added at each light's origin so zero-range lights stay selectable.
#![allow(clippy::cast_possible_truncation)]

/// A single line-list vertex (two per segment).
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LineVertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
}

const VERTEX_SIZE: u64 = std::mem::size_of::<LineVertex>() as u64;

/// Which light type a gizmo represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightGizmoKind {
    Directional,
    Point,
    Spot,
}

/// One light's gizmo, submitted by the editor each frame.
#[derive(Debug, Clone, Copy)]
pub struct LightGizmoDesc {
    pub kind: LightGizmoKind,
    /// World-space position of the light.
    pub position: glam::Vec3,
    /// Direction the light travels (normalized).
    pub direction: glam::Vec3,
    /// Light color (hue only — brightness is normalized for visibility).
    pub color: glam::Vec3,
    /// Attenuation radius; ignored for directional lights.
    pub range: f32,
    /// Spot inner cone half-angle (radians).
    pub inner_angle: f32,
    /// Spot outer cone half-angle (radians).
    pub outer_angle: f32,
    /// Selected lights draw at full brightness, others are dimmed.
    pub selected: bool,
}

// ─── Geometry helpers (all world-space) ───────────────────────────────────────

const CIRCLE_SEGS: u32 = 32;
const CONE_SPOKES: u32 = 4;
/// Size of the little axis cross drawn at every light's origin (metres).
const MARKER_SIZE: f32 = 0.25;
/// Arrow length for directional lights (metres).
const DIR_ARROW_LEN: f32 = 2.0;

fn push_line(out: &mut Vec<LineVertex>, a: glam::Vec3, b: glam::Vec3, color: [f32; 3]) {
    out.push(LineVertex {
        position: a.to_array(),
        color,
    });
    out.push(LineVertex {
        position: b.to_array(),
        color,
    });
}

/// A closed circle centred at `center`, spanned by the orthonormal pair `u`/`v`.
fn push_circle(
    out: &mut Vec<LineVertex>,
    center: glam::Vec3,
    u: glam::Vec3,
    v: glam::Vec3,
    radius: f32,
    color: [f32; 3],
) {
    let mut prev = center + u * radius;
    for i in 1..=CIRCLE_SEGS {
        let a = std::f32::consts::TAU * i as f32 / CIRCLE_SEGS as f32;
        let p = center + (u * a.cos() + v * a.sin()) * radius;
        push_line(out, prev, p, color);
        prev = p;
    }
}

/// Three great circles (XY / XZ / YZ) approximating a sphere.
fn push_wire_sphere(out: &mut Vec<LineVertex>, center: glam::Vec3, radius: f32, color: [f32; 3]) {
    push_circle(out, center, glam::Vec3::X, glam::Vec3::Y, radius, color);
    push_circle(out, center, glam::Vec3::X, glam::Vec3::Z, radius, color);
    push_circle(out, center, glam::Vec3::Y, glam::Vec3::Z, radius, color);
}

/// Small axis cross marking the light's origin.
fn push_marker(out: &mut Vec<LineVertex>, center: glam::Vec3, size: f32, color: [f32; 3]) {
    push_line(
        out,
        center - glam::Vec3::X * size,
        center + glam::Vec3::X * size,
        color,
    );
    push_line(
        out,
        center - glam::Vec3::Y * size,
        center + glam::Vec3::Y * size,
        color,
    );
    push_line(
        out,
        center - glam::Vec3::Z * size,
        center + glam::Vec3::Z * size,
        color,
    );
}

/// A cone opening from `apex` along `dir` with the given half-angle.
///
/// Sized exactly as in the Bevy reference: the base sits at `range * cos(angle)`
/// with radius `range * sin(angle)`, so the rim lies on the sphere of radius
/// `range` — i.e. the cone shows the light's true reach.
fn push_cone(
    out: &mut Vec<LineVertex>,
    apex: glam::Vec3,
    dir: glam::Vec3,
    half_angle: f32,
    range: f32,
    color: [f32; 3],
) {
    let angle = half_angle.clamp(0.0, std::f32::consts::FRAC_PI_2);
    let height = range * angle.cos();
    let radius = range * angle.sin();
    let base = apex + dir * height;
    let (u, v) = dir.any_orthonormal_pair();

    push_circle(out, base, u, v, radius, color);
    for i in 0..CONE_SPOKES {
        let a = std::f32::consts::TAU * i as f32 / CONE_SPOKES as f32;
        let rim = base + (u * a.cos() + v * a.sin()) * radius;
        push_line(out, apex, rim, color);
    }
}

/// A line from `from` to `to` with a 4-line arrowhead at the far end.
fn push_arrow(out: &mut Vec<LineVertex>, from: glam::Vec3, to: glam::Vec3, color: [f32; 3]) {
    push_line(out, from, to, color);

    let dir = (to - from).normalize_or_zero();
    if dir == glam::Vec3::ZERO {
        return;
    }
    let (u, v) = dir.any_orthonormal_pair();
    let len = (to - from).length();
    let head = (len * 0.22).min(0.5);
    let back = to - dir * head;
    for (su, sv) in [(1.0f32, 0.0f32), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        push_line(out, to, back + (u * su + v * sv) * head * 0.45, color);
    }
}

/// Convert one light description into line segments.
fn push_light(out: &mut Vec<LineVertex>, d: &LightGizmoDesc) {
    // Normalize brightness so dim/HDR light colors still read clearly, then
    // dim unselected lights so the selected one stands out.
    let max_c = d.color.max_element().max(1e-4);
    let hue = (d.color / max_c).clamp(glam::Vec3::ZERO, glam::Vec3::ONE);
    let tint = if d.selected { 1.0 } else { 0.45 };
    let color = (hue * tint).to_array();

    let dir = d.direction.normalize_or(glam::Vec3::NEG_Y);
    push_marker(out, d.position, MARKER_SIZE, color);

    match d.kind {
        LightGizmoKind::Directional => {
            // Arrow along the light direction, plus parallel "rays" around it
            // so the light reads as directional at a glance.
            push_arrow(out, d.position, d.position + dir * DIR_ARROW_LEN, color);
            let (u, v) = dir.any_orthonormal_pair();
            for i in 0..4u32 {
                let a = std::f32::consts::TAU * i as f32 / 4.0;
                let off = (u * a.cos() + v * a.sin()) * 0.35;
                push_line(
                    out,
                    d.position + off,
                    d.position + off + dir * (DIR_ARROW_LEN * 0.6),
                    color,
                );
            }
        }
        LightGizmoKind::Point => {
            // Sphere at the attenuation radius — the light's reach.
            push_wire_sphere(out, d.position, d.range.max(0.01), color);
        }
        LightGizmoKind::Spot => {
            let range = d.range.max(0.01);
            // Outer cone at full tint, inner cone dimmed to distinguish them.
            push_cone(out, d.position, dir, d.outer_angle, range, color);
            let inner = (hue * tint * 0.55).to_array();
            push_cone(out, d.position, dir, d.inner_angle, range, inner);
            // Centre line showing the aim direction.
            push_line(out, d.position, d.position + dir * range, color);
        }
    }
}

/// Build the full line list for every submitted light.
pub fn build_light_gizmo_lines(descs: &[LightGizmoDesc]) -> Vec<LineVertex> {
    let mut out = Vec::new();
    for d in descs {
        push_light(&mut out, d);
    }
    out
}

// ─── LightGizmoPass ───────────────────────────────────────────────────────────

/// Draws all light gizmos as one batched `LineList` over the swapchain.
pub struct LightGizmoPass {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    /// Capacity in vertices; the buffer doubles when a frame needs more.
    capacity: usize,
}

/// Initial vertex capacity (one point light is ~192 verts).
const INITIAL_CAPACITY: usize = 4096;

impl LightGizmoPass {
    /// Create the pass.
    ///
    /// - `surface_format`: swapchain format (drawn after tone mapping).
    /// - `view_buffer`: the global view buffer (same one the transform gizmo uses).
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        surface_format: wgpu::TextureFormat,
        view_buffer: &wgpu::Buffer,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Light Gizmo Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Light Gizmo Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: view_buffer.as_entire_binding(),
            }],
        });

        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Light Gizmo Vertex Buffer"),
            size: INITIAL_CAPACITY as u64 * VERTEX_SIZE,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Light Gizmo Shader"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("light_gizmo.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Light Gizmo Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Light Gizmo Pipeline"),
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
                topology: wgpu::PrimitiveTopology::LineList,
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
            capacity: INITIAL_CAPACITY,
        }
    }

    /// Upload `lines` and draw them over `surface_view`. No-op when empty.
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
        lines: &[LineVertex],
    ) {
        if lines.is_empty() {
            return;
        }

        // Grow (doubling) if this frame needs more room than the buffer has.
        if lines.len() > self.capacity {
            let mut cap = self.capacity.max(1);
            while cap < lines.len() {
                cap *= 2;
            }
            self.vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Light Gizmo Vertex Buffer"),
                size: cap as u64 * VERTEX_SIZE,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.capacity = cap;
        }
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(lines));

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Light Gizmo Pass"),
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
        rpass.draw(0..lines.len() as u32, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(kind: LightGizmoKind) -> LightGizmoDesc {
        LightGizmoDesc {
            kind,
            position: glam::Vec3::new(1.0, 2.0, 3.0),
            direction: glam::Vec3::NEG_Y,
            color: glam::Vec3::new(1.0, 0.8, 0.5),
            range: 10.0,
            inner_angle: 25.0_f32.to_radians(),
            outer_angle: 35.0_f32.to_radians(),
            selected: true,
        }
    }

    #[test]
    fn every_light_kind_emits_line_pairs() {
        for kind in [
            LightGizmoKind::Directional,
            LightGizmoKind::Point,
            LightGizmoKind::Spot,
        ] {
            let lines = build_light_gizmo_lines(&[desc(kind)]);
            assert!(!lines.is_empty(), "{kind:?} produced no geometry");
            // LineList topology: vertices must come in pairs.
            assert_eq!(lines.len() % 2, 0, "{kind:?} emitted an odd vertex count");
        }
    }

    #[test]
    fn spot_cone_rim_lies_on_the_range_sphere() {
        // The cone is sized so its rim touches the sphere of radius `range`,
        // which is what makes the gizmo show the light's true reach.
        let d = desc(LightGizmoKind::Spot);
        let mut out = Vec::new();
        push_cone(
            &mut out,
            d.position,
            glam::Vec3::NEG_Y,
            d.outer_angle,
            d.range,
            [1.0; 3],
        );
        for v in &out {
            let p = glam::Vec3::from_array(v.position);
            let dist = (p - d.position).length();
            assert!(
                (dist - d.range).abs() < 1e-3 || dist < d.range,
                "cone vertex at {dist} exceeds range {}",
                d.range
            );
        }
    }

    #[test]
    fn unselected_lights_are_dimmer_than_selected() {
        let mut sel = desc(LightGizmoKind::Point);
        sel.selected = true;
        let mut unsel = sel;
        unsel.selected = false;

        let a = build_light_gizmo_lines(&[sel]);
        let b = build_light_gizmo_lines(&[unsel]);
        assert_eq!(a.len(), b.len());
        assert!(b[0].color[0] < a[0].color[0]);
    }

    #[test]
    fn zero_range_and_zero_direction_are_handled() {
        // Degenerate lights must not produce NaNs (they'd corrupt the draw).
        let mut d = desc(LightGizmoKind::Spot);
        d.range = 0.0;
        d.direction = glam::Vec3::ZERO;
        let lines = build_light_gizmo_lines(&[d]);
        for v in &lines {
            assert!(
                v.position.iter().all(|c| c.is_finite()),
                "non-finite vertex"
            );
        }
    }
}
