//! Versioned default landscape shared by startup and Create -> Terrain.

use crate::{
    Children, EntitySnapshot, FoliageComponent, MeshComponent, MeshKind, Name,
    PostProcessComponent, TerrainComponent, Transform, WaterComponent, WorldTransform,
};

/// Current serialized authoring recipe. Increment when defaults change in a
/// way that should not silently alter existing scenes.
pub const DEFAULT_LANDSCAPE_VERSION: u32 = 4;

#[derive(Debug, Clone, Copy)]
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
    let water = WaterComponent::great_lakes(water_id, terrain_id, [0.0, 0.0, width, depth]);
    renderer.ensure_water_body(render_ctx, water.descriptor())?;
    let allocation = renderer.upload_water_body_mesh(render_ctx, water_id)?;
    let (terrain, water_snapshot) = landscape_snapshots(
        preset,
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
    preset: DefaultLandscapePreset,
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
        mesh_kind: None,
        is_particle_emitter: false,
        terrain: Some(TerrainComponent {
            terrain_id,
            chunk_cells: preset.terrain.chunk_cells,
            grid_x: preset.terrain.grid_size[0],
            grid_z: preset.terrain.grid_size[1],
            cell_size: preset.terrain.cell_size,
            height_scale: preset.terrain.height_scale,
        }),
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
        // Retained only for old scene compatibility; rendering is selected by
        // WaterComponent and the allocation is a finite wet-cell mesh.
        mesh_kind: Some(MeshKind::Plane),
        is_particle_emitter: false,
        terrain: None,
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
        let startup = landscape_snapshots(preset, 4, component, mesh);
        let menu = landscape_snapshots(DefaultLandscapePreset::current(), 4, component, mesh);
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
}
