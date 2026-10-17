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
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// `source -> parts` (`None` = every part) to warm.
type Plan = BTreeMap<String, Option<HashSet<usize>>>;

fn read_json(path: &Path) -> Result<serde_json::Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // `SOMNIUM-SCENE 1 <len>`, a header line, then the document; plain JSON otherwise.
    let body = if text.starts_with("SOMNIUM-SCENE") { text.splitn(3, '\n').nth(2).unwrap_or("") } else { &text };
    serde_json::from_str(body).map_err(|e| format!("{}: {e}", path.display()))
}

fn add(plan: &mut Plan, source: String, parts: Option<HashSet<usize>>) {
    let entry = plan.entry(source).or_insert_with(|| Some(HashSet::new()));
    match (entry.as_mut(), parts) {
        (Some(have), Some(more)) => have.extend(more),
        _ => *entry = None,
    }
}

fn plan_for(root: &Path, arg: &Path, plan: &mut Plan) -> Result<(), String> {
    let relative = |path: &Path| path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/");
    if arg.is_dir() {
        for entry in std::fs::read_dir(arg).map_err(|e| format!("{}: {e}", arg.display()))?.flatten() {
            let path = entry.path();
            if path.is_dir() || path.extension().is_some_and(|e| e == "gltf" || e == "glb") {
                plan_for(root, &path, plan)?;
            }
        }
        return Ok(());
    }
    match arg.extension().and_then(|e| e.to_str()) {
        Some("gltf" | "glb") => add(plan, relative(arg), None),
        Some("json") => {
            let document = read_json(arg)?;
            if let Some(entries) = document["entries"].as_array() {
                for entry in entries {
                    for source in [&entry["source"], &entry["lod"]["source"]] {
                        if let Some(source) = source.as_str() {
                            add(plan, source.to_owned(), None);
                        }
                    }
                }
            }
            for entity in document["entities"].as_array().into_iter().flatten() {
                let fields = &entity["components"]["somnium.ImportedMesh"]["fields"];
                if let (Some(source), Some(node)) = (fields["source"].as_str(), fields["node"].as_u64()) {
                    add(plan, source.to_owned(), Some(HashSet::from([node as usize])));
                }
            }
        }
        _ => return Err(format!("{}: not a scene, palette, model or folder", arg.display())),
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (root, items) = args.split_first().ok_or("usage: warm_texture_cache <content root> <scene | palette | model | folder>...")?;
    let root = PathBuf::from(root);
    let mut plan = Plan::new();
    for item in items {
        plan_for(&root, Path::new(item), &mut plan)?;
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
