//! Versioned default landscape shared by startup and Create -> Terrain.

use crate::{
    Children, EntitySnapshot, FoliageComponent, MeshComponent, MeshKind, Name,
    PostProcessComponent, TerrainComponent, Transform, WaterComponent, WorldTransform,
};

/// Current serialized authoring recipe. Increment when defaults change in a
/// way that should not silently alter existing scenes.
pub const DEFAULT_LANDSCAPE_VERSION: u32 = 4;

#[derive(Debug, Clone)]
/// Versioned authoring values shared by startup and the editor factory.
pub struct DefaultLandscapePreset {
    /// Recipe version persisted by the scene contract.
    pub version: u32,
    /// Terrain grid and height source descriptor.
    pub terrain: somnium_renderer::terrain::TerrainDescriptor,
    /// Default vertical relief in metres.
    pub relief_metres: f32,
    /// Altitude threshold used by the default material auto-splat.
    pub auto_splat_height: f32,
    /// Root terrain placement in world space.
    pub terrain_translation: glam::Vec3,
    /// Child-water placement relative to the terrain root.
    pub water_local_translation: glam::Vec3,
    /// Default editor camera position.
    pub camera_position: glam::Vec3,
    /// Default editor camera yaw in degrees.
    pub camera_yaw_degrees: f32,
    /// Default editor camera pitch in degrees.
    pub camera_pitch_degrees: f32,
    /// Default post-processing authoring values.
    pub post_process: PostProcessComponent,
}

impl DefaultLandscapePreset {
    /// Return the current immutable default-landscape recipe.
    pub fn current() -> Self {
        let terrain = somnium_renderer::terrain::TerrainDescriptor::default();
        let [width, depth] = terrain.world_size();
        let relief = somnium_renderer::terrain::DEFAULT_RELIEF_METRES;
        Self {
            version: DEFAULT_LANDSCAPE_VERSION,
            terrain,
            relief_metres: relief,
            auto_splat_height: relief * 0.48,
            terrain_translation: glam::Vec3::new(-width * 0.5, 0.0, -depth * 0.5),
            water_local_translation: glam::Vec3::new(
                width * 0.5,
                somnium_renderer::terrain::DEFAULT_WATER_LEVEL_METRES,
                depth * 0.5,
            ),
            camera_position: glam::Vec3::new(0.0, relief * 1.15 + 30.0, depth * 0.45),
            camera_yaw_degrees: -90.0,
            camera_pitch_degrees: -22.0,
            post_process: PostProcessComponent::default(),
        }
    }

    /// 512 m ocean tile with a compact FBM island. Same water look as Coastal.
    pub fn island() -> Self {
        let mut terrain = somnium_renderer::terrain::TerrainDescriptor::default();
        terrain.grid_size = [8, 8];
        let [width, depth] = terrain.world_size();
        let relief = 18.0;
        Self {
            version: DEFAULT_LANDSCAPE_VERSION,
            terrain,
            relief_metres: relief,
            auto_splat_height: 72.0,
            terrain_translation: glam::Vec3::new(-width * 0.5, 0.0, -depth * 0.5),
            water_local_translation: glam::Vec3::new(
                width * 0.5,
                somnium_renderer::terrain::DEFAULT_WATER_LEVEL_METRES,
                depth * 0.5,
            ),
            camera_position: glam::Vec3::new(0.0, 28.0, 115.0),
            camera_yaw_degrees: -90.0,
            camera_pitch_degrees: -16.0,
            post_process: PostProcessComponent::default(),
        }
    }
}

/// GPU allocations and the two snapshots consumed by either startup or the
/// editor's one-step undo command.
pub struct BuiltLandscape {
    /// Recipe used for this allocation.
    pub preset: DefaultLandscapePreset,
    /// Terrain entity snapshot ready for spawn or an editor command.
    pub terrain: EntitySnapshot,
    /// Water child snapshot ready for parent stitching and spawn.
    pub water: EntitySnapshot,
}

/// Allocate the renderer resources and return matching terrain/water snapshots.
pub fn create_default_landscape(
    renderer: &mut somnium_renderer::SomniumRenderer,
    render_ctx: &somnium_renderer::RenderContext,
) -> Result<BuiltLandscape, String> {
    let preset = DefaultLandscapePreset::current();
    let terrain_id = renderer.create_terrain(render_ctx, preset.terrain);
    if let Some(terrain) = renderer.terrain_mut(terrain_id) {
        terrain.apply_default_relief(preset.relief_metres);
        somnium_renderer::terrain::brush::auto_splat(terrain, preset.auto_splat_height);
    }
    let [width, depth] = preset.terrain.world_size();
    let water_id = renderer.allocate_water_body_id();
    let mut water = WaterComponent::great_lakes(water_id, terrain_id, [0.0, 0.0, width, depth]);
    // `SOMNIUM_WATER_LEVEL` overrides the authored datum for an A/B. The
    // shoreline follows it now (`water_body::reproject_to_datum`), which is
    // what makes the override worth having: before, it would have moved the
    // plane and left the coverage behind.
    if let Ok(level) = std::env::var("SOMNIUM_WATER_LEVEL") {
        if let Ok(level) = level.trim().parse::<f32>() {
            water.surface_level = level;
        }
    }
    renderer.ensure_water_body(render_ctx, water.descriptor())?;
    let allocation = renderer.upload_water_body_mesh(render_ctx, water_id)?;
    let (terrain, water_snapshot) = landscape_snapshots(
        &preset,
        terrain_id,
        water,
        MeshComponent {
            vertex_offset: allocation.vertex_offset,
            index_offset: allocation.index_offset,
            index_count: allocation.index_count,
        },
    );
    Ok(BuiltLandscape {
        preset,
        terrain,
        water: water_snapshot,
    })
}

/// Allocate a 512 m ocean tile with a compact FBM island and hero-bank splat.
pub fn create_island_landscape(
    renderer: &mut somnium_renderer::SomniumRenderer,
    render_ctx: &somnium_renderer::RenderContext,
) -> Result<BuiltLandscape, String> {
    let preset = DefaultLandscapePreset::island();
    let terrain_id = renderer.create_terrain_hero_bank(render_ctx, preset.terrain);
    if let Some(terrain) = renderer.terrain_mut(terrain_id) {
        terrain.apply_hero_bank_gpu_budget();
        terrain.generate_island_relief(1337, preset.relief_metres);
        somnium_renderer::terrain::brush::auto_splat_island(terrain, preset.auto_splat_height);
    }
    let [width, depth] = preset.terrain.world_size();
    let water_id = renderer.allocate_water_body_id();
    let water = WaterComponent::ocean(water_id, terrain_id, [0.0, 0.0, width, depth]);
    renderer.ensure_water_body(render_ctx, water.descriptor())?;
    let allocation = renderer.upload_water_body_mesh(render_ctx, water_id)?;
    let (terrain, water_snapshot) = landscape_snapshots(
        &preset,
        terrain_id,
        water,
        MeshComponent {
            vertex_offset: allocation.vertex_offset,
            index_offset: allocation.index_offset,
            index_count: allocation.index_count,
        },
    );
    Ok(BuiltLandscape {
        preset,
        terrain,
        water: water_snapshot,
    })
}

fn landscape_snapshots(
    preset: &DefaultLandscapePreset,
    terrain_id: u32,
    water: WaterComponent,
    mesh: MeshComponent,
) -> (EntitySnapshot, EntitySnapshot) {
    let terrain = EntitySnapshot {
        transform: Some(Transform::from_translation(preset.terrain_translation)),
        name: Some(Name::new("Terrain")),
        light: None,
        mesh: None,
        mat: None,
        wt: Some(WorldTransform::identity()),
        environment: false,
        decal: None,
        mesh_kind: None,
        is_particle_emitter: false,
        terrain: Some(TerrainComponent {
            terrain_id,
            chunk_cells: preset.terrain.chunk_cells,
            grid_x: preset.terrain.grid_size[0],
            grid_z: preset.terrain.grid_size[1],
            cell_size: preset.terrain.cell_size,
            height_scale: preset.terrain.height_scale,
            ..TerrainComponent::default()
        }),
        world_partition: Some(crate::WorldPartitionComponent::default()),
        ui_canvas: None,
        voxel_terrain: None,
        foliage: Some(FoliageComponent::default()),
        water: None,
        parent: None,
        children: Some(Children::empty()),
    };
    let water = EntitySnapshot {
        transform: Some(Transform::from_translation(preset.water_local_translation)),
        name: Some(Name::new("Water")),
        light: None,
        mesh: Some(mesh),
        mat: None,
        wt: Some(WorldTransform::identity()),
        environment: false,
        decal: None,
        // Retained only for old scene compatibility; rendering is selected by
        // WaterComponent and the allocation is a finite wet-cell mesh.
        mesh_kind: Some(MeshKind::Plane),
        is_particle_emitter: false,
        terrain: None,
        world_partition: None,
        ui_canvas: None,
        voxel_terrain: None,
        foliage: None,
        water: Some(water),
        parent: None,
        children: None,
    };
    (terrain, water)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_recipe_is_versioned_and_water_matches_terrain() {
        let preset = DefaultLandscapePreset::current();
        let [width, depth] = preset.terrain.world_size();
        assert_eq!(preset.version, DEFAULT_LANDSCAPE_VERSION);
        assert_eq!(
            preset.terrain_translation,
            glam::Vec3::new(-width * 0.5, 0.0, -depth * 0.5)
        );
        assert_eq!(
            glam::Vec2::new(
                preset.water_local_translation.x,
                preset.water_local_translation.z
            ),
            glam::Vec2::new(width * 0.5, depth * 0.5)
        );
        assert_eq!(
            preset.water_local_translation.y,
            somnium_renderer::terrain::DEFAULT_WATER_LEVEL_METRES
        );
    }

    #[test]
    fn startup_and_create_menu_share_the_same_structural_snapshots() {
        let preset = DefaultLandscapePreset::current();
        let component = WaterComponent::great_lakes(7, 4, [0.0, 0.0, 1024.0, 1024.0]);
        let mesh = MeshComponent {
            vertex_offset: 10,
            index_offset: 20,
            index_count: 30,
        };
        let startup = landscape_snapshots(&preset, 4, component, mesh);
        let menu = landscape_snapshots(&DefaultLandscapePreset::current(), 4, component, mesh);
        assert_eq!(
            startup.0.name.unwrap().as_str(),
            menu.0.name.unwrap().as_str()
        );
        assert_eq!(
            startup.0.transform.unwrap().translation,
            menu.0.transform.unwrap().translation
        );
        assert_eq!(startup.0.terrain, menu.0.terrain);
        assert_eq!(
            startup.1.name.unwrap().as_str(),
            menu.1.name.unwrap().as_str()
        );
        assert_eq!(
            startup.1.transform.unwrap().translation,
            menu.1.transform.unwrap().translation
        );
        assert_eq!(startup.1.water, menu.1.water);
        assert_eq!(
            startup.1.mesh.unwrap().index_count,
            menu.1.mesh.unwrap().index_count
        );
        assert_eq!(startup.0.children.unwrap().count, 0);
        assert!(startup.1.parent.is_none());
    }

    #[test]
    fn island_recipe_is_512m_with_the_same_water_datum() {
        let preset = DefaultLandscapePreset::island();
        let [width, depth] = preset.terrain.world_size();
        assert_eq!(preset.terrain.grid_size, [8, 8]);
        assert_eq!((width, depth), (512.0, 512.0));
        assert_eq!(
            preset.water_local_translation.y,
            somnium_renderer::terrain::DEFAULT_WATER_LEVEL_METRES
        );
        assert!(preset.camera_position.z > 0.0 && preset.camera_position.z < depth * 0.4);
    }

    #[test]
    fn hero_bank_unbind_matches_the_island_gpu_budget() {
        let mut ids = somnium_renderer::terrain::TerrainTextureIds {
            splat_maps: [1, 2, 3, 4, 5, 6, 7, 8],
            macro_map: 0,
            albedo: [1; 32],
            surface: [1; 32],
            virtual_texture: [-1, -1, -1, 0],
        };
        ids.unbind_extra_bank();
        assert!(ids.splat_maps[4..].iter().all(|&id| id < 0));
        assert!(ids.albedo[16..].iter().all(|&id| id < 0));
        assert!(ids.surface[16..].iter().all(|&id| id < 0));
        assert_eq!(&ids.splat_maps[..4], &[1, 2, 3, 4]);
    }
}
