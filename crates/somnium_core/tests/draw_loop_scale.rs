//! What the game's mesh loop costs per frame as a region's entities grow (open-world render scale, 19 Oct 2026).
//!
//! The loop is hello_engine's: every entity with a world transform, a mesh and a material, its foliage verdict
//! (`foliage_visibility::imported_draw`), its LOD and its cutaway, then a draw. The world is shaped like Rimeborn's
//! composed tile: houses under `LodGroup` roots (LOD0, LOD1, LOD2 children holding the parts), imported trees under
//! a `Foliage` root, and a terrain carrying foliage that is nobody's parent. Ignored; run in release:
//!
//! ```text
//! cargo test --release -p somnium_core --test draw_loop_scale -- --ignored --nocapture
//! ```
use std::time::Instant;

use glam::{Mat4, Vec3};
use somnium_core::cutaway::Cutaways;
use somnium_core::foliage_visibility::{ImportedDraws, imported_draw};
use somnium_core::lod_group::{LodGroupComponent, Lods};
use somnium_core::{
    FoliageComponent, ImportedMesh, MaterialComponent, MeshComponent, Name, Parent, Transform, WorldTransform,
};
use somnium_ecs::{ComponentId, ComponentSet, Entity, World};

fn part(world: &mut World, parent: Entity, at: Vec3, node: u32) {
    let entity = world.spawn((
        Transform::default(),
        Parent { entity: parent },
        WorldTransform(Mat4::from_translation(at)),
        MeshComponent { vertex_offset: node, index_offset: 0, index_count: 300 },
    ));
    let _ = world.insert_component(entity, MaterialComponent { runtime_id: 1, ..Default::default() });
    let _ = world.insert_component(entity, ImportedMesh { source: "house.gltf".into(), node });
}

/// `houses` houses of `parts` LOD0 parts (a third of that at LOD1, two at LOD2) and `trees` three-part trees.
fn region(houses: usize, parts: usize, trees: usize) -> World {
    let mut world = World::new();
    world.spawn((Transform::default(), FoliageComponent { enabled: true, ..Default::default() }));
    for h in 0..houses {
        let at = Vec3::new((h % 200) as f32 * 30.0, 0.0, (h / 200) as f32 * 30.0);
        let root = world.spawn((Transform::from_translation(at), Name::new("House"), LodGroupComponent::default()));
        for (level, count) in [parts, parts / 3, 2].into_iter().enumerate() {
            let lod = world.spawn((Transform::default(), Parent { entity: root }, Name::new(&format!("LOD{level}"))));
            for p in 0..count {
                part(&mut world, lod, at, p as u32);
            }
        }
    }
    for t in 0..trees {
        let at = Vec3::new((t % 300) as f32 * 9.0, 0.0, -10.0 - (t / 300) as f32 * 9.0);
        let root = world.spawn((
            Transform::from_translation(at),
            FoliageComponent { enabled: true, cull_distance: 400.0, ..Default::default() },
        ));
        for p in 0..3 {
            part(&mut world, root, at, 1000 + p);
        }
    }
    world
}

fn draw_loop(world: &World, lods: &Lods, cutaways: &Cutaways, camera: Vec3, out: &mut Vec<(u32, Mat4, bool)>) {
    let required = ComponentSet::from_ids(vec![
        ComponentId::of::<WorldTransform>(),
        ComponentId::of::<MeshComponent>(),
        ComponentId::of::<MaterialComponent>(),
    ]);
    for archetype in world.query_archetypes(&required, &ComponentSet::empty()) {
        let wt_col = archetype.column_index(ComponentId::of::<WorldTransform>()).unwrap();
        let m_col = archetype.column_index(ComponentId::of::<MeshComponent>()).unwrap();
        for row in 0..archetype.len() {
            let wt = unsafe { archetype.column(wt_col).get::<WorldTransform>(row) };
            let mesh = unsafe { archetype.column(m_col).get::<MeshComponent>(row) };
            let entity = archetype.entities()[row];
            let Some(casts) = imported_draw(world, entity, camera) else { continue };
            if lods.is_hidden(entity) || cutaways.is_shadow_only(entity) {
                continue;
            }
            out.push((mesh.vertex_offset, wt.0, casts));
        }
    }
}

/// The same loop with the LOD test first and the frame's verdict cache.
fn cached_loop(
    world: &World,
    lods: &Lods,
    cutaways: &Cutaways,
    draws: &mut ImportedDraws,
    camera: Vec3,
    out: &mut Vec<(u32, Mat4, bool)>,
) {
    draws.begin_frame();
    let required = ComponentSet::from_ids(vec![
        ComponentId::of::<WorldTransform>(),
        ComponentId::of::<MeshComponent>(),
        ComponentId::of::<MaterialComponent>(),
    ]);
    for archetype in world.query_archetypes(&required, &ComponentSet::empty()) {
        let wt_col = archetype.column_index(ComponentId::of::<WorldTransform>()).unwrap();
        let m_col = archetype.column_index(ComponentId::of::<MeshComponent>()).unwrap();
        for row in 0..archetype.len() {
            let entity = archetype.entities()[row];
            if lods.is_hidden(entity) {
                continue;
            }
            let Some(casts) = draws.draw_row(world, archetype, row, camera) else { continue };
            if cutaways.is_shadow_only(entity) {
                continue;
            }
            let wt = unsafe { archetype.column(wt_col).get::<WorldTransform>(row) };
            let mesh = unsafe { archetype.column(m_col).get::<MeshComponent>(row) };
            out.push((mesh.vertex_offset, wt.0, casts));
        }
    }
}

#[test]
#[ignore = "a measurement: run in release with --nocapture"]
fn the_mesh_loop_at_region_scale() {
    for (houses, parts, trees) in [(300, 45, 3000), (800, 45, 8000), (1600, 45, 0)] {
        let world = region(houses, parts, trees);
        let camera = Vec3::new(1000.0, 60.0, 300.0);
        let mut lods = Lods::default();
        lods.update(&world, camera);
        let mut cutaways = Cutaways::default();
        cutaways.update(&world, camera);
        let mut out = Vec::new();
        let frames = 30;
        let started = Instant::now();
        for _ in 0..frames {
            out.clear();
            draw_loop(&world, &lods, &cutaways, camera, &mut out);
        }
        let loop_ms = started.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        let meshes = world.entities_with::<MeshComponent>().count();
        let started = Instant::now();
        let mut kept = 0;
        for _ in 0..frames {
            for entity in world.entities_with::<MeshComponent>() {
                kept += usize::from(imported_draw(&world, entity, camera).is_some());
            }
        }
        let verdict_ms = started.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        let started = Instant::now();
        for _ in 0..frames {
            lods.update(&world, camera);
        }
        let lods_ms = started.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        println!(
            "{meshes} mesh entities ({} drawn): loop {loop_ms:.2} ms a frame ({:.0} ns each), of which foliage verdicts \
             {verdict_ms:.2} ms; Lods::update {lods_ms:.2} ms",
            out.len(),
            loop_ms * 1e6 / meshes as f64,
        );
        assert!(kept > 0);
        let mut draws = ImportedDraws::default();
        let mut cached = Vec::new();
        let started = Instant::now();
        for _ in 0..frames {
            cached.clear();
            cached_loop(&world, &lods, &cutaways, &mut draws, camera, &mut cached);
        }
        let cached_ms = started.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        let started = Instant::now();
        for _ in 0..frames {
            draws.begin_frame();
            for entity in world.entities_with::<MeshComponent>() {
                kept += usize::from(draws.draw(&world, entity, camera).is_some());
            }
        }
        let cached_verdict_ms = started.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        println!(
            "  LOD first + ImportedDraws: loop {cached_ms:.2} ms; ImportedDraws alone over every mesh {cached_verdict_ms:.2} ms"
        );
        assert_eq!(cached, out, "the same draws");
        // What walking the rows and asking for the LOD costs on its own.
        let required = ComponentSet::from_ids(vec![
            ComponentId::of::<WorldTransform>(),
            ComponentId::of::<MeshComponent>(),
            ComponentId::of::<MaterialComponent>(),
        ]);
        let started = Instant::now();
        let mut shown = 0usize;
        for _ in 0..frames {
            for archetype in world.query_archetypes(&required, &ComponentSet::empty()) {
                for &entity in archetype.entities() {
                    shown += usize::from(!lods.is_hidden(entity));
                }
            }
        }
        let rows_ms = started.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        println!("  rows and LOD test alone: {rows_ms:.2} ms ({} shown)", shown / frames);
    }
}
