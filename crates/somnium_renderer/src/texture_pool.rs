//! Bindless texture management.

use crate::bindless::MAX_BINDLESS_TEXTURES;

/// Manages a pool of textures for bindless rendering.
pub struct TexturePool {
    /// The dummy texture used for unassigned slots.
    pub dummy_view: wgpu::TextureView,
    /// All registered texture views.
    pub views: Vec<wgpu::TextureView>,
    /// Indices of free slots in the pool.
    free_indices: Vec<u32>,
}

impl TexturePool {
    pub fn new(device: &wgpu::Device) -> Self {
        let dummy_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Dummy Bindless Texture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let dummy_view = dummy_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let _views: Vec<&wgpu::TextureView> = Vec::with_capacity(MAX_BINDLESS_TEXTURES as usize);
        for _ in 0..MAX_BINDLESS_TEXTURES {
            // We can't easily store the same view multiple times in a Vec of owned views
            // but we'll manage the indices.
            // Actually, in wgpu we need to provide a slice of references to views.
        }

        Self {
            dummy_view,
            views: Vec::new(), // This will hold owned views
            free_indices: (0..MAX_BINDLESS_TEXTURES).rev().collect(),
        }
    }

    /// Add a texture to the pool and return its index.
    pub fn add_texture(&mut self, view: wgpu::TextureView) -> u32 {
        if let Some(index) = self.free_indices.pop() {
            // Ensure views vector is large enough
            if self.views.len() <= index as usize {
                self.views
                    .resize_with(index as usize + 1, || self.dummy_view.clone());
            }
            self.views[index as usize] = view;
            index
        } else {
            panic!("Texture pool exhausted!");
        }
    }
}
