//! Scene-owned reservations must be reusable without invalidating imported assets.
#[path = "support/gpu.rs"]
mod gpu;

#[test]
fn repeated_scene_reservations_keep_slots_and_offsets_bounded() {
    let (device, _) = gpu::device();
    let mut textures = somnium_renderer::texture_pool::TexturePool::new(&device);
    let persistent = textures.add_texture(textures.dummy_view.clone());
    let mut geometry = somnium_renderer::geometry::GeometryPool::new(&device);
    let persistent_vertices = geometry.reserve_vertices(19).unwrap();
    let mut first_vertex = None;
    for _ in 0..1100 {
        let slot = textures.add_texture(textures.dummy_view.clone());
        assert_ne!(persistent, slot);
        assert_eq!(textures.live_count(), 2);
        assert!(textures.release(slot));
        assert!(!textures.release(slot));
        let a = geometry.reserve_vertices(4096).unwrap();
        let b = geometry.reserve_vertices(4096).unwrap();
        assert_eq!(*first_vertex.get_or_insert(a), a);
        assert_ne!(persistent_vertices, a);
        let indices = geometry.reserve_indices(24576).unwrap();
        geometry.release_vertices(a);
        geometry.release_vertices(b);
        geometry.release_indices(indices);
        assert_eq!(geometry.reserved_span_counts(), (1, 0));
    }
    assert_eq!(textures.live_count(), 1);
    // Adjacent freed reservations can serve a larger subsequent terrain tile.
    assert_eq!(geometry.reserve_vertices(8192), first_vertex);
}
