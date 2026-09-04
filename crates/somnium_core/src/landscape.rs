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
    // DOOM-I: building a map is the largest stall in a session and reported
    // nothing at all — a `.somtime` run showed 8.2 s between the renderer being
    // constructed and the first presented frame, with no log line inside it.
    // These spans exist so the next person reads a breakdown instead of
    // bisecting one. `debug!` would have hidden them from exactly the run that
    // needs them.
    let started = std::time::Instant::now();
    let preset = DefaultLandscapePreset::current();
    let terrain_id = renderer.create_terrain(render_ctx, preset.terrain);
    let allocated = started.elapsed();
    if let Some(terrain) = renderer.terrain_mut(terrain_id) {
        terrain.apply_default_relief(preset.relief_metres);
        somnium_renderer::terrain::brush::auto_splat(terrain, preset.auto_splat_height);
    }
    let relieved = started.elapsed();
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
    tracing::info!(
        terrain_alloc_ms = allocated.as_secs_f32() * 1000.0,
        relief_splat_ms = (relieved - allocated).as_secs_f32() * 1000.0,
        water_ms = (started.elapsed() - relieved).as_secs_f32() * 1000.0,
        total_ms = started.elapsed().as_secs_f32() * 1000.0,
        "Coastal landscape built"
    );
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

/// Allocate a terrain with nothing on it: flat, one material, no water.
///
/// `create_default_landscape` hands back a finished coastline — relief, an
/// altitude-driven splat and a lake. That is the right thing when someone wants
/// a scene in one click, and the wrong thing when they want to *author* one:
/// the first two brush strokes are then undoing a preset rather than building
/// anything, and the water body has to be hunted down and deleted.
///
/// This is the other half of that pair. The heightfield is flat at the datum,
/// every texel is layer 0, and the sculpt and paint brushes have somewhere to
/// start. No relief, no auto-splat, no water, no camera move — the author
/// decides all four.
pub fn create_empty_terrain(
    renderer: &mut somnium_renderer::SomniumRenderer,
    render_ctx: &somnium_renderer::RenderContext,
) -> EntitySnapshot {
    let descriptor = somnium_renderer::terrain::TerrainDescriptor::default();
    let terrain_id = renderer.create_terrain(render_ctx, descriptor);
    if let Some(terrain) = renderer.terrain_mut(terrain_id) {
        // Layer 0 everywhere. A terrain whose splatmap is all zeroes renders as
        // the fallback mean albedo — a flat grey plate that looks like a bug
        // rather than like ground waiting to be painted.
        somnium_renderer::terrain::brush::fill_layer(terrain, 0);
    }
    EntitySnapshot {
        spline: None,
        transform: Some(Transform::from_translation(glam::Vec3::ZERO)),
        name: Some(Name::new("Terrain")),
        light: None,
        audio: None,
        mesh: None,
        mat: None,
        wt: Some(WorldTransform::identity()),
        environment: false,
        decal: None,
        mesh_kind: None,
        is_particle_emitter: false,
        terrain: Some(TerrainComponent {
            terrain_id,
            chunk_cells: descriptor.chunk_cells,
            grid_x: descriptor.grid_size[0],
            grid_z: descriptor.grid_size[1],
            cell_size: descriptor.cell_size,
            height_scale: descriptor.height_scale,
            ..TerrainComponent::default()
        }),
        world_partition: Some(crate::WorldPartitionComponent::default()),
        ui_canvas: None,
        voxel_terrain: None,
        // Enabled, so a foliage stroke works the moment the terrain exists.
        // The component being absent is the difference between "paints nothing"
        // and "paints nothing and gives no reason why".
        foliage: Some(FoliageComponent::default()),
        water: None,
        parent: None,
        children: Some(Children::empty()),
    }
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

/// Fill a channel body descriptor's centreline and bounds from a spline.
///
/// One definition, called from two places: entity creation, which needs a baked
/// mask before it can allocate a mesh, and the per-frame refresh in `App`, which
/// is what makes the river follow a control point the author drags.
///
/// That refresh needs no dirty flag. `ensure_water_body` compares descriptors
/// for equality, so a moved point simply *is* a different descriptor. The
/// terrain macro tier and the TSUSHIMA horizon bake both had to be corrected
/// for a dirty flag nobody set, and this avoids owning one.
#[must_use]
pub fn channel_descriptor(
    water: WaterComponent,
    spline: &crate::SplineComponent,
    model: glam::Mat4,
) -> somnium_renderer::water_body::WaterBodyDescriptor {
    use somnium_renderer::water_body::WATER_PATH_POINTS;
    let mut descriptor = water.descriptor();
    // The sampled polyline, not the control points: the bake measures distance
    // to straight segments, and a Catmull-Rom curve through four widely spaced
    // controls bows away from its chord by metres.
    let sampled = spline.polyline();
    if sampled.len() < 2 {
        // A channel with no usable path stays empty rather than flooding its
        // bounds, which is what `bake_channel` does with a short path.
        return descriptor;
    }
    // Decimate evenly to the fixed capacity rather than truncating, so a long
    // river keeps its shape instead of losing its downstream half.
    let count = sampled.len().min(WATER_PATH_POINTS);
    let mut min = glam::Vec2::splat(f32::INFINITY);
    let mut max = glam::Vec2::splat(f32::NEG_INFINITY);
    for slot in 0..count {
        let source = slot * (sampled.len() - 1) / (count - 1).max(1);
        let world = model.transform_point3(sampled[source]);
        let xz = glam::Vec2::new(world.x, world.z);
        descriptor.path[slot] = [xz.x, xz.y];
        min = min.min(xz);
        max = max.max(xz);
    }
    descriptor.path_len = count as u32;
    // Bounds follow the path. The margin is two half-widths so the shore SDF
    // has room to go negative outside the ribbon: a bank flush against the edge
    // of the raster has no outside, and the shore fade then reads as a hard cut
    // at the bounds rather than at the water.
    let margin = descriptor.half_width.max(0.5) * 2.0;
    descriptor.bounds = [
        min.x - margin,
        min.y - margin,
        max.x + margin,
        max.y + margin,
    ];
    descriptor
}

/// Allocate a standalone body of water of one kind, ready to parent to a
/// terrain.
///
/// The landscape presets above build a terrain and its water together because
/// they are one authored object. This is the other need: an author who already
/// has terrain and wants a lake in the valley, or a river through it.
pub fn create_water_body(
    renderer: &mut somnium_renderer::SomniumRenderer,
    render_ctx: &somnium_renderer::RenderContext,
    kind: crate::WaterBodyKind,
    terrain_id: u32,
    bounds: [f32; 4],
) -> Result<EntitySnapshot, String> {
    use crate::WaterBodyKind as K;
    let water_id = renderer.allocate_water_body_id();
    let water = match kind {
        K::Lake => WaterComponent::great_lakes(water_id, terrain_id, bounds),
        K::Ocean => WaterComponent::ocean(water_id, terrain_id, bounds),
        K::Sea => WaterComponent::sea(water_id, terrain_id, bounds),
        K::River => WaterComponent::river(water_id, terrain_id, bounds),
    };

    // A river needs a path before it has any coverage at all, so the spline is
    // created with it rather than left for the author to add. Straight, along
    // the bounds' long axis, at a spacing that puts the control points far
    // enough apart to be draggable.
    let spline = (kind == K::River).then(|| {
        let span_x = bounds[2] - bounds[0];
        let span_z = bounds[3] - bounds[1];
        let along = span_x.max(span_z).max(32.0);
        crate::SplineComponent::straight(5, along * 0.2)
    });

    let descriptor = match spline.as_ref() {
        Some(spline) => channel_descriptor(water, spline, glam::Mat4::IDENTITY),
        None => water.descriptor(),
    };
    renderer.ensure_water_body(render_ctx, descriptor)?;
    let allocation = renderer.upload_water_body_mesh(render_ctx, water_id)?;

    Ok(EntitySnapshot {
        spline,
        transform: Some(Transform::from_translation(glam::Vec3::ZERO)),
        name: Some(Name::new(kind.label())),
        light: None,
        audio: None,
        mesh: Some(MeshComponent {
            vertex_offset: allocation.vertex_offset,
            index_offset: allocation.index_offset,
            index_count: allocation.index_count,
        }),
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
    })
}

fn landscape_snapshots(
    preset: &DefaultLandscapePreset,
    terrain_id: u32,
    water: WaterComponent,
    mesh: MeshComponent,
) -> (EntitySnapshot, EntitySnapshot) {
    let terrain = EntitySnapshot {
        spline: None,
        transform: Some(Transform::from_translation(preset.terrain_translation)),
        name: Some(Name::new("Terrain")),
        light: None,
        audio: None,
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
        spline: None,
        transform: Some(Transform::from_translation(preset.water_local_translation)),
        name: Some(Name::new("Water")),
        light: None,
        audio: None,
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
        // `..Default::default()` on purpose. This test is about which ids
        // `unbind_extra_bank` clears, and spelling out every field means a
        // new one — TSUSHIMA added three — breaks a test that has no opinion
        // about it.
        let mut ids = somnium_renderer::terrain::TerrainTextureIds {
            splat_maps: [1, 2, 3, 4, 5, 6, 7, 8],
            macro_map: 0,
            albedo: [1; 32],
            surface: [1; 32],
            virtual_texture: [-1, -1, -1, 0],
            ..Default::default()
        };
        ids.unbind_extra_bank();
        assert!(ids.splat_maps[4..].iter().all(|&id| id < 0));
        assert!(ids.albedo[16..].iter().all(|&id| id < 0));
        assert!(ids.surface[16..].iter().all(|&id| id < 0));
        assert_eq!(&ids.splat_maps[..4], &[1, 2, 3, 4]);
    }

    /// The four kinds have to actually differ, or the menu is four names for
    /// one thing.
    ///
    /// Optics first, because that is what separates them: a river carrying silt
    /// scatters an order of magnitude more than clear open ocean, and no amount
    /// of wave tuning makes one read as the other.
    #[test]
    fn the_water_kinds_are_optically_distinct() {
        use crate::WaterBodyKind as K;
        let bounds = [0.0, 0.0, 512.0, 512.0];
        let lake = WaterComponent::great_lakes(1, 0, bounds);
        let ocean = WaterComponent::ocean(2, 0, bounds);
        let sea = WaterComponent::sea(3, 0, bounds);
        let river = WaterComponent::river(4, 0, bounds);

        for (component, kind) in [
            (lake, K::Lake),
            (ocean, K::Ocean),
            (sea, K::Sea),
            (river, K::River),
        ] {
            assert_eq!(component.kind(), kind, "{} lost its kind", kind.label());
        }

        // Scattering rises with suspended load: ocean is clearest, then the
        // shelf sea, then the river.
        let green = |w: WaterComponent| w.scattering[1];
        assert!(
            green(ocean) < green(sea) && green(sea) < green(river),
            "scattering is not ordered ocean < sea < river: {} {} {}",
            green(ocean),
            green(sea),
            green(river)
        );
        // Sea state falls the other way. A river has no fetch at all.
        assert!(
            ocean.wave_length_a > sea.wave_length_a && sea.wave_length_a > river.wave_length_a,
            "wavelengths are not ordered ocean > sea > river"
        );
        assert!(
            river.max_depth < lake.max_depth,
            "a river should be shallower than a lake"
        );
        // Only the river is a swept channel; the rest keep their own coverage.
        assert_eq!(
            river.preset,
            somnium_renderer::water_body::WATER_PRESET_CHANNEL
        );
        for other in [lake, ocean, sea] {
            assert_ne!(
                other.preset,
                somnium_renderer::water_body::WATER_PRESET_CHANNEL,
                "{} was baked as a channel",
                other.kind().label()
            );
        }
    }

    /// A river follows its spline, and its bounds follow with it.
    ///
    /// This is the whole editing story: there is no dirty flag, so the only
    /// thing that can carry a dragged control point to the GPU is the
    /// descriptor being *different*.
    #[test]
    fn the_channel_descriptor_follows_the_spline() {
        let water = WaterComponent::river(1, 0, [0.0, 0.0, 100.0, 100.0]);
        let straight = crate::SplineComponent::straight(4, 25.0);
        let before = channel_descriptor(water, &straight, glam::Mat4::IDENTITY);
        assert!(before.path_len >= 2, "a straight spline produced no path");

        // Every sampled point is inside the bounds it produced.
        for point in before.path_points() {
            assert!(
                point[0] >= before.bounds[0]
                    && point[0] <= before.bounds[2]
                    && point[1] >= before.bounds[1]
                    && point[1] <= before.bounds[3],
                "{point:?} is outside the bounds it generated"
            );
        }

        // Move a control point: the descriptor must not compare equal, or
        // `ensure_water_body` will skip the rebake and the river will not move.
        let mut moved = straight.clone();
        moved.points[1].z += 40.0;
        let after = channel_descriptor(water, &moved, glam::Mat4::IDENTITY);
        assert_ne!(
            before, after,
            "moving a control point left the descriptor equal, so nothing rebakes"
        );
        assert!(
            after.bounds[3] > before.bounds[3],
            "the bounds did not grow with the path"
        );
    }

    /// An entity transform carries the path, exactly as it carries a mesh.
    #[test]
    fn the_channel_path_is_taken_through_the_entity_transform() {
        let water = WaterComponent::river(1, 0, [0.0, 0.0, 100.0, 100.0]);
        let spline = crate::SplineComponent::straight(4, 25.0);
        let origin = channel_descriptor(water, &spline, glam::Mat4::IDENTITY);
        let shifted = channel_descriptor(
            water,
            &spline,
            glam::Mat4::from_translation(glam::Vec3::new(200.0, 0.0, 0.0)),
        );
        assert!(
            (shifted.bounds[0] - origin.bounds[0] - 200.0).abs() < 1e-3,
            "the path ignored the entity transform"
        );
    }

    /// A spline longer than the fixed capacity keeps its shape.
    ///
    /// Truncating would silently lose the downstream half of a long river, and
    /// the symptom — water that stops in the middle of a valley — looks like a
    /// bake bug rather than a capacity one.
    #[test]
    fn a_long_spline_is_decimated_rather_than_truncated() {
        use somnium_renderer::water_body::WATER_PATH_POINTS;
        let water = WaterComponent::river(1, 0, [0.0, 0.0, 1000.0, 1000.0]);
        let long = crate::SplineComponent::straight(40, 25.0);
        let descriptor = channel_descriptor(water, &long, glam::Mat4::IDENTITY);
        assert_eq!(descriptor.path_len as usize, WATER_PATH_POINTS);

        let sampled = long.polyline();
        let first = sampled.first().expect("a straight spline has points");
        let last = sampled.last().expect("a straight spline has points");
        let path = descriptor.path_points();
        assert!(
            (path[0][0] - first.x).abs() < 1e-3,
            "the path lost its upstream end"
        );
        assert!(
            (path[path.len() - 1][0] - last.x).abs() < 1e-3,
            "the path lost its downstream end"
        );
    }
}
