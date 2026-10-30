//! Fill the BC7 texture cache before a level is opened.
//!
//! The renderer compresses each imported texture the first time it uploads it
//! (`imported_bc.rs`) and keeps the result; a big level's first open would
//! otherwise spend minutes encoding. This encodes what the upload would:
//!
//! - a scene (`*.json` with entities): the textures of the parts its imported meshes use;
//! - a foliage palette (`*palette.json`): every material of each entry's model and its far LOD;
//! - a model (`*.gltf` / `*.glb`) or a folder of them: every material (held maps, reader pages).
//!
//! ```text
//! cargo run --release -p somnium_renderer --example warm_texture_cache -- <content root> <scene | palette | model | folder>...
//! cargo run --release -p somnium_renderer --example warm_texture_cache -- games/TheSomnusFracture games/TheSomnusFracture/scenes/campaign_town.scene.json games/TheSomnusFracture/assets/foliage.palette.json
//! ```
//!
//! Paths inside scenes and palettes are relative to the content root (a game's
//! folder). `SOMNIUM_TEXTURE_CACHE` moves the cache, as for the engine.
use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (root, items) = args.split_first().ok_or("usage: warm_texture_cache <content root> <scene | palette | model | folder>...")?;
    let root = PathBuf::from(root);
    let mut plan = somnium_renderer::imported_bc::WarmPlan::new();
    for item in items {
        somnium_renderer::imported_bc::plan_warm(&root, Path::new(item), &mut plan)?;
    }
    println!("cache: {}", somnium_renderer::imported_bc::cache_dir().display());
    let started = Instant::now();
    let (mut encoded, mut cached) = (0, 0);
    for (index, (source, parts)) in plan.iter().enumerate() {
        let scene = match somnium_asset::load_gltf(root.join(source)) {
            Ok(scene) => scene,
            Err(error) => {
                eprintln!("skipped {source}: {error}");
                continue;
            }
        };
        let (new, old) = somnium_renderer::renderer::SomniumRenderer::warm_texture_cache(&scene, parts.as_ref());
        encoded += new;
        cached += old;
        println!("[{}/{}] {source}: {new} encoded, {old} already cached ({:.0?})", index + 1, plan.len(), started.elapsed());
    }
    println!("done: {encoded} encoded, {cached} already cached, in {:.0?}", started.elapsed());
    Ok(())
}
