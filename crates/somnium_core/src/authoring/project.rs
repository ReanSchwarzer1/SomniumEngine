//! Explicit project roots and contained file resolution.
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// Private project manifest. Paths are relative to the manifest's directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectManifest {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub content: PathBuf,
    pub scenes: PathBuf,
    pub source_assets: PathBuf,
    pub documents: PathBuf,
    pub cooked: PathBuf,
    pub staging: PathBuf,
    pub saves: PathBuf,
    pub logs: PathBuf,
    pub captures: PathBuf,
    pub runtime: PathBuf,
}

/// Resolved project with one authority for source and generated paths.
#[derive(Debug, Clone)]
pub struct ProjectPaths {
    pub root: PathBuf,
    pub manifest: ProjectManifest,
}

impl ProjectPaths {
    pub fn open(root: &Path) -> Result<Self, String> {
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let bytes = std::fs::read(root.join("game.project.json")).map_err(|e| e.to_string())?;
        let manifest: ProjectManifest =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if manifest.version != 1 || manifest.id.is_empty() {
            return Err("unsupported or unnamed project".into());
        }
        let paths = Self { root, manifest };
        for path in paths.declared_paths() {
            paths.resolve(path)?;
        }
        Ok(paths)
    }

    pub fn declared_paths(&self) -> [&Path; 11] {
        let m = &self.manifest;
        [
            &m.content,
            &m.scenes,
            &m.source_assets,
            &m.documents,
            &m.cooked,
            &m.staging,
            &m.saves,
            &m.logs,
            &m.captures,
            &m.runtime,
            Path::new("backups"),
        ]
    }

    /// Resolve an existing or prospective path, rejecting traversal and junction escapes.
    pub fn resolve(&self, relative: &Path) -> Result<PathBuf, String> {
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        {
            return Err("project path must be a non-empty relative path without traversal".into());
        }
        if relative.components().any(|c| match c {
            Component::Normal(v) => {
                let text = v.to_string_lossy();
                text.contains(':') || text.ends_with('.') || text.ends_with(' ')
            }
            _ => false,
        }) {
            return Err(
                "project paths may not use alternate streams or ambiguous Windows names".into(),
            );
        }
        let candidate = self.root.join(relative);
        let mut ancestor = candidate.as_path();
        while !ancestor.exists() {
            ancestor = ancestor.parent().ok_or("path has no existing ancestor")?;
        }
        let canonical = ancestor.canonicalize().map_err(|e| e.to_string())?;
        if !canonical.starts_with(&self.root) {
            return Err("project path escapes through a junction or symbolic link".into());
        }
        Ok(candidate)
    }

    pub fn create_directories(&self) -> Result<(), String> {
        for path in self.declared_paths() {
            let resolved = self.resolve(path)?;
            std::fs::create_dir_all(resolved).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_parent_absolute_and_junction_escapes() {
        let root =
            std::env::temp_dir().join(format!("somnium-project-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let m = ProjectManifest {
            version: 1,
            id: "test".into(),
            name: "Test".into(),
            content: "assets".into(),
            scenes: "scenes".into(),
            source_assets: "source-assets".into(),
            documents: "data".into(),
            cooked: "cooked".into(),
            staging: "staging".into(),
            saves: "saves".into(),
            logs: "logs".into(),
            captures: "captures".into(),
            runtime: "runtime".into(),
        };
        std::fs::write(
            root.join("game.project.json"),
            serde_json::to_vec(&m).unwrap(),
        )
        .unwrap();
        let project = ProjectPaths::open(&root).unwrap();
        assert!(project.resolve(Path::new("../escape")).is_err());
        assert!(project.resolve(&root).is_err());
        assert!(
            project
                .resolve(Path::new("assets/models/new.glb"))
                .unwrap()
                .starts_with(&project.root)
        );
        std::fs::remove_file(root.join("game.project.json")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
