//! No new full-world entity scans, in the engine or in any game.
//!
//! `world.entities()` visits every entity. Called once per frame it is the
//! commonest way a level goes CPU-bound as it grows: Town spent days there
//! (2026-09-25..27) and no graphics setting could help. Per-frame code asks
//! the world for what it needs instead — `iter_with` / `entities_with` /
//! `first_with` over a component, and `World::change_signature` to rebuild a
//! cache only when its inputs change (see `per_frame_ecs_queries` in memory
//! and `somnium_ecs::world`).
//!
//! This is a ratchet over the count per file: a file may lose scans, never
//! gain them, and a file or a whole new game not listed here starts at zero.
//! A scan that genuinely runs once (loading, saving, an editor command) is
//! added to `BASELINE` in the same change, with the reason in its commit.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Full scans per file as of 2026-09-27, none of them per frame.
const BASELINE: &[(&str, usize)] = &[
    ("crates/somnium_core/src/ai.rs", 1),
    ("crates/somnium_core/src/ai_editor.rs", 4),
    ("crates/somnium_core/src/ai_script.rs", 1),
    ("crates/somnium_core/src/animation_authoring.rs", 1),
    ("crates/somnium_core/src/app.rs", 30),
    ("crates/somnium_core/src/app_authoring.rs", 1),
    ("crates/somnium_core/src/app_authoring_specialized.rs", 1),
    ("crates/somnium_core/src/app_designer.rs", 1),
    ("crates/somnium_core/src/authoring/transaction.rs", 3),
    ("crates/somnium_core/src/clipboard.rs", 8),
    ("crates/somnium_core/src/editor_commands.rs", 12),
    ("crates/somnium_core/src/map.rs", 1),
    ("crates/somnium_core/src/prefab.rs", 7),
    ("crates/somnium_core/src/save_game/editor.rs", 1),
    ("crates/somnium_core/src/scatter_scene.rs", 1),
    ("crates/somnium_core/src/scene_schema.rs", 5),
    ("crates/somnium_core/src/scene_serial.rs", 1),
    ("crates/somnium_core/src/script_host.rs", 2),
    ("crates/somnium_core/src/script_input.rs", 6),
    ("crates/somnium_core/src/world_partition.rs", 2),
    ("crates/somnium_ecs/src/archetype.rs", 1),
    ("crates/somnium_ecs/src/persistent.rs", 2),
    ("crates/somnium_ecs/src/world.rs", 3),
    ("games/TheSomnusFracture/src/campaign.rs", 2),
    ("games/TheSomnusFracture/src/campaign_early.rs", 2),
    ("games/TheSomnusFracture/src/campaign_end.rs", 8),
    ("games/TheSomnusFracture/src/campaign_late.rs", 2),
    ("games/TheSomnusFracture/src/campaign_town.rs", 2),
    ("games/TheSomnusFracture/src/checkpoint.rs", 2),
    ("games/TheSomnusFracture/src/checkpoint_host.rs", 5),
    ("games/TheSomnusFracture/src/endgame.rs", 3),
    ("games/TheSomnusFracture/src/epilogue.rs", 3),
    ("games/TheSomnusFracture/src/forest.rs", 6),
    ("games/TheSomnusFracture/src/frame_query.rs", 2),
    ("games/TheSomnusFracture/src/lib.rs", 12),
    ("games/TheSomnusFracture/src/pursuit.rs", 2),
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `.entities()` calls in code, comments excluded.
fn scans(source: &str) -> usize {
    source
        .lines()
        .map(|line| line.split("//").next().unwrap_or("").matches(".entities()").count())
        .sum()
}

#[test]
fn no_file_gains_a_full_world_scan() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for group in ["crates", "games"] {
        for member in std::fs::read_dir(root.join(group)).into_iter().flatten().flatten() {
            rust_files(&member.path().join("src"), &mut files);
        }
    }
    assert!(files.len() > 50, "found only {} sources under {}", files.len(), root.display());
    let baseline: BTreeMap<_, _> = BASELINE.iter().copied().collect();
    let mut grown = Vec::new();
    for path in files {
        let relative = path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        let count = scans(&std::fs::read_to_string(&path).unwrap_or_default());
        let allowed = baseline.get(relative.as_str()).copied().unwrap_or(0);
        if count > allowed {
            grown.push(format!("{relative}: {count} full-world scans (allowed {allowed})"));
        }
    }
    assert!(
        grown.is_empty(),
        "new `.entities()` scans. Per-frame code should query by component \
         (iter_with / entities_with / first_with) and cache on \
         World::change_signature; a scan that truly runs once goes into \
         BASELINE with its reason:\n{}",
        grown.join("\n")
    );
}

#[test]
fn comments_do_not_count() {
    assert_eq!(scans("let n = world.entities().count(); // world.entities()"), 1);
    assert_eq!(scans("/// Avoid world.entities() per frame."), 0);
}
