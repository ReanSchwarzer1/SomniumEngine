//! Metaphor helpers: Help pages, content inventory, type icons (Phase 26-C/D).

use crate::icons::IconId;
use std::path::{Path, PathBuf};

pub const HELP_WELCOME: &str = include_str!("../../../docs/editor/welcome.md");
pub const HELP_VIEWPORT: &str = include_str!("../../../docs/editor/viewport.md");
pub const HELP_SHORTCUTS: &str = include_str!("../../../docs/editor/shortcuts.md");
pub const HELP_DRAWER: &str = include_str!("../../../docs/editor/content_drawer.md");
pub const HELP_ABOUT: &str = include_str!("../../../docs/editor/about.md");
pub const HELP_OUTLINER: &str = include_str!("../../../docs/editor/outliner.md");
pub const HELP_TERRAIN: &str = include_str!("../../../docs/editor/terrain.md");
pub const HELP_WATER: &str = include_str!("../../../docs/editor/water.md");
pub const HELP_LIGHTING: &str = include_str!("../../../docs/editor/lighting.md");

pub fn help_page(id: u8) -> &'static str {
    match id {
        1 => HELP_VIEWPORT,
        2 => HELP_SHORTCUTS,
        3 => HELP_DRAWER,
        4 => HELP_ABOUT,
        5 => HELP_OUTLINER,
        6 => HELP_TERRAIN,
        7 => HELP_WATER,
        8 => HELP_LIGHTING,
        _ => HELP_WELCOME,
    }
}

pub fn help_titles() -> &'static [&'static str] {
    &[
        "Welcome",
        "Viewport",
        "Shortcuts",
        "Content Drawer",
        "About",
        "Outliner",
        "Terrain",
        "Water",
        "Lighting",
    ]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelpBlock {
    Heading(String),
    Paragraph(String),
    Bullet(String),
}

pub fn help_blocks(id: u8) -> Vec<HelpBlock> {
    parse_help_markdown(help_page(id))
}

fn strip_md(s: &str) -> String {
    s.replace("**", "")
}

pub fn parse_help_markdown(src: &str) -> Vec<HelpBlock> {
    let mut out = Vec::new();
    let mut para = String::new();
    let flush_para = |para: &mut String, out: &mut Vec<HelpBlock>| {
        let t = para.trim();
        if !t.is_empty() {
            out.push(HelpBlock::Paragraph(strip_md(t)));
        }
        para.clear();
    };
    for line in src.lines() {
        let t = line.trim();
        if t.is_empty() {
            flush_para(&mut para, &mut out);
        } else if let Some(rest) = t.strip_prefix("## ") {
            flush_para(&mut para, &mut out);
            out.push(HelpBlock::Heading(strip_md(rest)));
        } else if let Some(rest) = t.strip_prefix("# ") {
            flush_para(&mut para, &mut out);
            out.push(HelpBlock::Heading(strip_md(rest)));
        } else if let Some(rest) = t.strip_prefix("- ") {
            flush_para(&mut para, &mut out);
            out.push(HelpBlock::Bullet(strip_md(rest)));
        } else {
            if !para.is_empty() {
                para.push(' ');
            }
            para.push_str(t);
        }
    }
    flush_para(&mut para, &mut out);
    out
}

#[derive(Clone, Debug)]
pub struct ContentEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub icon: IconId,
    pub is_engine: bool,
}

pub fn icon_for_path(path: &Path, is_dir: bool) -> IconId {
    if is_dir {
        return IconId::Folder;
    }
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "gltf" | "glb" => IconId::Mesh,
        "png" | "jpg" | "jpeg" | "exr" | "hdr" | "tga" => IconId::Texture,
        "json" => IconId::Json,
        "wgsl" | "hlsl" | "glsl" => IconId::Shader,
        "ttf" | "otf" => IconId::Font,
        "wav" | "ogg" | "mp3" => IconId::Audio,
        "md" => IconId::License,
        "rs" => IconId::Script,
        "somnium" => IconId::Scene,
        _ => IconId::Unknown,
    }
}

/// Recursively list `assets/` (project /Game) plus optional virtual /Engine.
pub fn list_content(
    root: &Path,
    show_engine: bool,
    filter: &str,
    current: &Path,
) -> Vec<ContentEntry> {
    let mut out = Vec::new();
    let filter = filter.to_ascii_lowercase();
    let walk = if current.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        current.to_path_buf()
    };
    if let Ok(rd) = std::fs::read_dir(&walk) {
        let mut ents: Vec<_> = rd.flatten().collect();
        ents.sort_by_key(|e| {
            (
                !e.path().is_dir(),
                e.file_name().to_string_lossy().to_ascii_lowercase(),
            )
        });
        for e in ents {
            let path = e.path();
            if path.file_name().and_then(|n| n.to_str()) == Some("bc7") {
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.starts_with('.') {
                continue;
            }
            if !filter.is_empty() && !name.to_ascii_lowercase().contains(&filter) && !path.is_dir()
            {
                continue;
            }
            let is_dir = path.is_dir();
            out.push(ContentEntry {
                icon: icon_for_path(&path, is_dir),
                is_dir,
                is_engine: false,
                name,
                path,
            });
        }
    }
    if show_engine && (current.as_os_str().is_empty() || current == root) {
        for (name, icon) in [
            ("Cube", IconId::Cube),
            ("Sphere", IconId::Sphere),
            ("Plane", IconId::Plane),
            ("Cylinder", IconId::Cylinder),
            ("Inter-Regular.ttf", IconId::Font),
        ] {
            if filter.is_empty() || name.to_ascii_lowercase().contains(&filter) {
                out.push(ContentEntry {
                    path: PathBuf::from("/Engine").join(name),
                    name: name.into(),
                    is_dir: false,
                    icon,
                    is_engine: true,
                });
            }
        }
    }
    out
}

pub fn icon_for_entity_name(name: &str) -> IconId {
    let n = name.to_ascii_lowercase();
    if n.contains("light") || n.contains("sun") {
        IconId::DirectionalLight
    } else if n.contains("terrain") && n.contains("voxel") {
        IconId::VoxelTerrain
    } else if n.contains("terrain") {
        IconId::Terrain
    } else if n.contains("water") {
        IconId::Water
    } else if n.contains("boat") || n.contains("vessel") {
        IconId::Vessel
    } else if n.contains("post") {
        IconId::PostFx
    } else if n.contains("particle") {
        IconId::Particle
    } else if n.contains("camera") {
        IconId::Camera
    } else {
        IconId::Cube
    }
}

#[allow(dead_code)]
pub fn create_icon(kind: crate::editor_event::CreateKind) -> IconId {
    use crate::editor_event::CreateKind::*;
    match kind {
        Cube => IconId::Cube,
        Sphere => IconId::Sphere,
        Plane => IconId::Plane,
        Cylinder => IconId::Cylinder,
        DirectionalLight => IconId::DirectionalLight,
        PointLight => IconId::PointLight,
        SpotLight => IconId::SpotLight,
        RectLight => IconId::PointLight,
        DiscLight => IconId::PointLight,
        TubeLight => IconId::SpotLight,
        Particle => IconId::Particle,
        Terrain => IconId::Terrain,
        VoxelTerrain => IconId::VoxelTerrain,
    }
}

pub fn tonemap_index(label: &str) -> usize {
    match label {
        "ACES" => 1,
        "Reinhard" => 2,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn help_pages_are_nonempty() {
        assert!(HELP_WELCOME.contains("Somnium"));
        assert!(HELP_SHORTCUTS.contains("Ctrl+Space"));
        assert_eq!(help_titles().len(), 9);
        assert_eq!(help_page(4), HELP_ABOUT);
        assert!(HELP_WATER.contains("RT Reflect"));
        assert!(HELP_WATER.contains("RT Refraction"));
        assert!(HELP_LIGHTING.contains("Disc Light"));
        assert!(HELP_LIGHTING.contains("Tube Light"));
        assert!(HELP_LIGHTING.contains("4×4×4"));
        let blocks = parse_help_markdown("# Title\n\nHello **world**.\n- item\n");
        assert_eq!(
            blocks,
            vec![
                HelpBlock::Heading("Title".into()),
                HelpBlock::Paragraph("Hello world.".into()),
                HelpBlock::Bullet("item".into()),
            ]
        );
    }

    #[test]
    fn bc7_and_dotfiles_are_hidden() {
        let tmp = std::env::temp_dir().join(format!("somnium_content_{}", std::process::id()));
        let assets = tmp.join("assets");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(assets.join("meshes")).unwrap();
        fs::create_dir_all(assets.join("terrain").join("bc7")).unwrap();
        fs::write(assets.join("meshes").join("cube.gltf"), "x").unwrap();
        fs::write(assets.join(".hidden"), "x").unwrap();
        fs::write(assets.join("terrain").join("bc7").join("pack.bin"), "x").unwrap();
        let listed = list_content(&assets, false, "", &PathBuf::new());
        assert!(listed.iter().any(|e| e.name == "meshes"));
        assert!(
            listed
                .iter()
                .all(|e| e.name != "bc7" && !e.name.starts_with('.'))
        );
        let nested = list_content(&assets, false, "", &assets.join("terrain"));
        assert!(nested.iter().all(|e| e.name != "bc7"));
        let engine = list_content(&assets, true, "cube", &PathBuf::new());
        assert!(engine.iter().any(|e| e.is_engine && e.name == "Cube"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn icon_for_gltf_is_mesh() {
        assert_eq!(
            icon_for_path(std::path::Path::new("foo.gltf"), false),
            IconId::Mesh
        );
        assert_eq!(
            icon_for_entity_name("Directional Light"),
            IconId::DirectionalLight
        );
        assert_eq!(tonemap_index("ACES"), 1);
    }
}
