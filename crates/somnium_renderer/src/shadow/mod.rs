//! Shadow map resources — atlas texture, views, and samplers.
//!
//! The shadow atlas is a single 4096×4096 Depth32Float texture divided into
//! four 2048×2048 quadrants, one per cascade:
//!
//! ```
//! ┌──────────┬──────────┐
//! │ cascade 0│ cascade 1│  (0,0)..(2048,2048) / (2048,0)..(4096,2048)
//! ├──────────┼──────────┤
//! │ cascade 2│ cascade 3│  (0,2048)..(2048,4096) / (2048,2048)..(4096,4096)
//! └──────────┴──────────┘
//! ```

pub mod cascade;

use bytemuck::{Pod, Zeroable};
use wgpu;

pub const ATLAS_SIZE: u32 = 4096;
pub const CASCADE_SIZE: u32 = 2048;
pub const NUM_CASCADES: usize = 4;

/// Cascade viewport regions within the atlas (x, y, w, h).
pub const CASCADE_VIEWPORTS: [(f32, f32, f32, f32); 4] = [
    (0.0,                  0.0,                  CASCADE_SIZE as f32, CASCADE_SIZE as f32),
    (CASCADE_SIZE as f32,  0.0,                  CASCADE_SIZE as f32, CASCADE_SIZE as f32),
    (0.0,                  CASCADE_SIZE as f32,  CASCADE_SIZE as f32, CASCADE_SIZE as f32),
    (CASCADE_SIZE as f32,  CASCADE_SIZE as f32,  CASCADE_SIZE as f32, CASCADE_SIZE as f32),
];

/// UV offsets for each cascade in atlas space (used in shading.wgsl).
pub const CASCADE_UV_OFFSETS: [(f32, f32); 4] = [
    (0.0, 0.0),
    (0.5, 0.0),
    (0.0, 0.5),
    (0.5, 0.5),
];

/// GPU-uploadable directional light struct (320 bytes, std140-aligned).
///
/// Layout:
/// ```
/// offset   0 :  direction     vec3<f32>          (12 bytes)
/// offset  12 :  _pad0         f32                ( 4 bytes)
/// offset  16 :  color         vec3<f32>          (12 bytes)  pre-multiplied by intensity
/// offset  28 :  _pad1         f32                ( 4 bytes)
/// offset  32 :  view_proj     array<mat4x4, 4>   (256 bytes) one VP per cascade
/// offset 288 :  cascade_splits vec4<f32>         (16 bytes)  view-space far Z per cascade
/// offset 304 :  shadow_map_size f32              ( 4 bytes)  total atlas size in texels
/// offset 308 :  ibl_intensity f32                (4 bytes)
/// offset 312 :  _pad2         [f32; 2]           (8 bytes)
///              total                             320 bytes
/// ```
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct GpuDirectionalLight {
    pub direction: [f32; 3],
    pub _pad0: f32,
    pub color: [f32; 3],
    pub _pad1: f32,
    /// Four cascade view-projection matrices, each column-major.
    pub view_proj: [[[f32; 4]; 4]; 4],
    pub cascade_splits: [f32; 4],
    pub shadow_map_size: f32,
    /// Scene-wide indirect-light strength (Phase 22C).
    ///
    /// Rides in this buffer's former padding because every pass that needs it
    /// -- shading, transparent, terrain, water -- already binds the light.
    pub ibl_intensity: f32,
    pub _pad2: [f32; 2],
}

impl Default for GpuDirectionalLight {
    fn default() -> Self {
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        Self {
            direction: [0.447, 0.894, -0.0],
            _pad0: 0.0,
            color: [5.0, 5.0, 5.0],
            _pad1: 0.0,
            view_proj: [identity; 4],
            cascade_splits: [5.0, 20.0, 50.0, 100.0],
            shadow_map_size: ATLAS_SIZE as f32,
            ibl_intensity: 0.35,
            _pad2: [0.0; 2],
        }
    }
}

/// Owns the shadow atlas texture, its views, and the comparison sampler.
pub struct ShadowMapResources {
    pub atlas_texture: wgpu::Texture,
    /// Full-texture view used as a depth render attachment in the shadow pass.
    pub atlas_view: wgpu::TextureView,
    /// Depth-only view used for `textureSampleCompare` in the shading pass.
    pub atlas_depth_view: wgpu::TextureView,
    /// PCF comparison sampler (`LessEqual`, linear filtering).
    pub comparison_sampler: wgpu::Sampler,
}

impl ShadowMapResources {
    pub fn new(device: &wgpu::Device) -> Self {
        let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Shadow Atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let atlas_depth_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor {
            aspect: wgpu::TextureAspect::DepthOnly,
            ..Default::default()
        });

        let comparison_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Shadow Comparison Sampler"),
            compare: Some(wgpu::CompareFunction::LessEqual),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        Self {
            atlas_texture,
            atlas_view,
            atlas_depth_view,
            comparison_sampler,
        }
    }
}
