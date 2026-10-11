//! Fill the BC7 texture cache for scenes before they are opened.
//!
//! The renderer compresses each imported texture the first time it uploads it
//! (`imported_bc.rs`) and keeps the result; a big level's first open would
//! otherwise spend minutes encoding. This encodes, for every scene given, the
//! textures of the parts its imported meshes use, exactly as the upload would.
//!
//! ```text
//! cargo run --release -p somnium_renderer --example warm_texture_cache -- <content root> <scene.json>...
//! cargo run --release -p somnium_renderer --example warm_texture_cache -- games/TheSomnusFracture games/TheSomnusFracture/scenes/campaign_town.scene.json
//! ```
//!
//! The content root is what scene `ImportedMesh` sources are relative to (a
//! game's folder). `SOMNIUM_TEXTURE_CACHE` moves the cache, as for the engine.
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// `source -> parts` of every imported mesh in a scene file.
fn imported_parts(scene: &Path) -> Result<BTreeMap<String, HashSet<usize>>, String> {
    let text = std::fs::read_to_string(scene).map_err(|e| format!("{}: {e}", scene.display()))?;
    // `SOMNIUM-SCENE 1 <len>`, a header line, then the document; plain JSON otherwise.
    let body = if text.starts_with("SOMNIUM-SCENE") { text.splitn(3, '\n').nth(2).unwrap_or("") } else { &text };
    let document: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("{}: {e}", scene.display()))?;
    let mut parts: BTreeMap<String, HashSet<usize>> = BTreeMap::new();
    for entity in document["entities"].as_array().into_iter().flatten() {
        let fields = &entity["components"]["somnium.ImportedMesh"]["fields"];
        if let (Some(source), Some(node)) = (fields["source"].as_str(), fields["node"].as_u64()) {
            parts.entry(source.to_owned()).or_default().insert(node as usize);
        }
    }
    Ok(parts)
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (root, scenes) = args.split_first().ok_or("usage: warm_texture_cache <content root> <scene.json>...")?;
    let root = PathBuf::from(root);
    // One source may serve several scenes: warm it once, for every part any of them uses.
    let mut sources: BTreeMap<String, HashSet<usize>> = BTreeMap::new();
    for scene in scenes {
        for (source, parts) in imported_parts(Path::new(scene))? {
            sources.entry(source).or_default().extend(parts);
        }
    }
    println!("cache: {}", somnium_renderer::imported_bc::cache_dir().display());
    let started = Instant::now();
    let (mut encoded, mut cached) = (0, 0);
    for (index, (source, parts)) in sources.iter().enumerate() {
        let path = root.join(source);
        let scene = match somnium_asset::load_gltf(&path) {
            Ok(scene) => scene,
            Err(error) => {
                eprintln!("skipped {source}: {error}");
                continue;
            }
        };
        let (new, old) = somnium_renderer::renderer::SomniumRenderer::warm_texture_cache(&scene, Some(parts));
        encoded += new;
        cached += old;
        println!("[{}/{}] {source}: {new} encoded, {old} already cached ({:.0?})", index + 1, sources.len(), started.elapsed());
    }
    println!("done: {encoded} encoded, {cached} already cached, in {:.0?}", started.elapsed());
    Ok(())
}
