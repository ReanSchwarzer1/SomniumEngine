//! Project asset inventory used by the editor.
//!
//! The inventory is immutable once published. A worker builds a fresh snapshot
//! and the editor swaps an `Arc` at a frame boundary, so search never walks the
//! filesystem and an external writer can never expose a half-updated table.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::UNIX_EPOCH,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetId(u128);

impl AssetId {
    pub const NONE: Self = Self(0);

    #[must_use]
    pub const fn from_raw(raw: u128) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u128 {
        self.0
    }

    /// Stable identity of a content-relative path. Separators and case are
    /// normalised so Windows and Linux agree.
    #[must_use]
    pub fn from_relative_path(path: impl AsRef<Path>) -> Self {
        const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
        const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
        let text = normalise_relative(path.as_ref());
        let mut hash = OFFSET;
        for byte in text.bytes() {
            hash ^= u128::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
        Self(if hash == 0 { 1 } else { hash })
    }
}

impl std::fmt::Display for AssetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl Serialize for AssetId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for AssetId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        u128::from_str_radix(&text, 16)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

impl somnium_ecs::reflect::ReflectField for AssetId {
    fn field_type() -> somnium_ecs::reflect::FieldType {
        somnium_ecs::reflect::FieldType::Asset
    }

    fn to_reflect(&self) -> somnium_ecs::reflect::ReflectValue {
        somnium_ecs::reflect::ReflectValue::Asset(
            (*self != Self::NONE).then(|| somnium_ecs::reflect::AssetRef::from_raw(self.raw())),
        )
    }

    fn from_reflect(
        value: &somnium_ecs::reflect::ReflectValue,
        field: &'static str,
    ) -> Result<Self, somnium_ecs::reflect::ReflectError> {
        match value {
            somnium_ecs::reflect::ReflectValue::Asset(Some(asset)) => {
                Ok(Self::from_raw(asset.raw()))
            }
            somnium_ecs::reflect::ReflectValue::Asset(None)
            | somnium_ecs::reflect::ReflectValue::Nil => Ok(Self::NONE),
            other => Err(somnium_ecs::reflect::ReflectError::TypeMismatch {
                field,
                expected: "asset".into(),
                found: other.kind(),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum AssetKind {
    Folder,
    Texture,
    Mesh,
    Material,
    Scene,
    Script,
    Audio,
    Shader,
    Font,
    Json,
    Prefab,
    Unknown,
}

impl AssetKind {
    #[must_use]
    pub const fn bit(self) -> u64 {
        1_u64 << self as u8
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Folder => "Folder",
            Self::Texture => "Texture",
            Self::Mesh => "Mesh",
            Self::Material => "Material",
            Self::Scene => "Scene",
            Self::Script => "Script",
            Self::Audio => "Audio",
            Self::Shader => "Shader",
            Self::Font => "Font",
            Self::Json => "JSON",
            Self::Prefab => "Prefab",
            Self::Unknown => "File",
        }
    }
}

pub const ASSET_KIND_ALL: u64 = u64::MAX;
pub const ASSET_KIND_TEXTURE: u64 = AssetKind::Texture.bit();
pub const ASSET_KIND_MESH: u64 = AssetKind::Mesh.bit();
pub const ASSET_KIND_MATERIAL: u64 = AssetKind::Material.bit();

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssetMetadata {
    pub bytes: u64,
    pub modified_unix_ms: u64,
    pub content_hash: String,
    pub dimensions: Option<(u32, u32)>,
    pub triangles: Option<u64>,
    pub doc_line: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetRecord {
    pub id: AssetId,
    pub relative_path: String,
    pub absolute_path: PathBuf,
    pub name: String,
    pub parent: String,
    pub kind: AssetKind,
    pub metadata: AssetMetadata,
}

impl AssetRecord {
    #[must_use]
    pub fn tooltip(&self) -> String {
        let mut facts = vec![
            self.kind.label().to_string(),
            human_bytes(self.metadata.bytes),
        ];
        if let Some((w, h)) = self.metadata.dimensions {
            facts.push(format!("{w} x {h}"));
        }
        if let Some(triangles) = self.metadata.triangles {
            facts.push(format!("{triangles} triangles"));
        }
        if let Some(doc) = self.metadata.doc_line.as_deref() {
            facts.push(doc.to_owned());
        }
        facts.join(" · ")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AssetSort {
    #[default]
    Name,
    Kind,
    Size,
    Modified,
}

#[derive(Clone, Debug, Default)]
pub struct AssetQuery {
    pub parent: String,
    pub text: String,
    pub kind_mask: u64,
    pub sort: AssetSort,
    pub descending: bool,
}

#[derive(Clone, Default)]
pub struct AssetDbSnapshot {
    root: PathBuf,
    records: Arc<[AssetRecord]>,
    by_id: Arc<BTreeMap<AssetId, usize>>,
}

impl AssetDbSnapshot {
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn records(&self) -> &[AssetRecord] {
        &self.records
    }

    #[must_use]
    pub fn get(&self, id: AssetId) -> Option<&AssetRecord> {
        self.by_id
            .get(&id)
            .and_then(|index| self.records.get(*index))
    }

    #[must_use]
    pub fn query(&self, query: &AssetQuery) -> Vec<AssetRecord> {
        let parent = normalise_text(&query.parent);
        let needle = query.text.trim().to_ascii_lowercase();
        let mask = if query.kind_mask == 0 {
            ASSET_KIND_ALL
        } else {
            query.kind_mask
        };
        let mut rows: Vec<_> = self
            .records
            .iter()
            .filter(|row| row.parent == parent)
            .filter(|row| row.kind.bit() & mask != 0)
            .filter(|row| needle.is_empty() || row.name.to_ascii_lowercase().contains(&needle))
            .cloned()
            .collect();
        rows.sort_by(|a, b| {
            let folders = (a.kind != AssetKind::Folder).cmp(&(b.kind != AssetKind::Folder));
            if folders != std::cmp::Ordering::Equal {
                return folders;
            }
            let order = match query.sort {
                AssetSort::Name => a
                    .name
                    .to_ascii_lowercase()
                    .cmp(&b.name.to_ascii_lowercase()),
                AssetSort::Kind => a.kind.label().cmp(b.kind.label()),
                AssetSort::Size => a.metadata.bytes.cmp(&b.metadata.bytes),
                AssetSort::Modified => a
                    .metadata
                    .modified_unix_ms
                    .cmp(&b.metadata.modified_unix_ms),
            };
            order.then_with(|| a.relative_path.cmp(&b.relative_path))
        });
        if query.descending {
            rows.reverse();
            // Folders remain first even in descending order.
            rows.sort_by_key(|row| row.kind != AssetKind::Folder);
        }
        rows
    }

    #[must_use]
    pub fn search(&self, text: &str, kind_mask: u64) -> Vec<AssetRecord> {
        let needle = text.trim().to_ascii_lowercase();
        let mask = if kind_mask == 0 {
            ASSET_KIND_ALL
        } else {
            kind_mask
        };
        self.records
            .iter()
            .filter(|row| row.kind != AssetKind::Folder && row.kind.bit() & mask != 0)
            .filter(|row| {
                needle.is_empty()
                    || row.name.to_ascii_lowercase().contains(&needle)
                    || row.relative_path.to_ascii_lowercase().contains(&needle)
            })
            .take(256)
            .cloned()
            .collect()
    }

    /// Stable summary used by the debounce publisher. It intentionally uses
    /// content hashes rather than mtimes, so touching an unchanged file does
    /// not invalidate previews or the UI snapshot.
    #[must_use]
    pub fn revision(&self) -> String {
        let mut hasher = Sha256::new();
        for record in self.records.iter() {
            hasher.update(record.relative_path.as_bytes());
            hasher.update(record.metadata.content_hash.as_bytes());
            hasher.update(record.kind.bit().to_le_bytes());
        }
        format!("{:x}", hasher.finalize())
    }
}

/// Two-sample publication gate for watcher/periodic scan results.
///
/// A changed snapshot is held until an identical scan follows it. This is the
/// debounce boundary that prevents a writer's intermediate file state from
/// reaching the drawer; the scan's fingerprint cache is the second stage.
#[derive(Default)]
pub struct DebouncedAssetDb {
    published_revision: Option<String>,
    published: Option<AssetDbSnapshot>,
    pending: Option<(String, AssetDbSnapshot)>,
}

impl DebouncedAssetDb {
    /// Stages a scan. The first snapshot publishes immediately; later changes
    /// publish only after two consecutive scans agree.
    pub fn stage(&mut self, candidate: AssetDbSnapshot) -> Option<AssetDbSnapshot> {
        let revision = candidate.revision();
        if self.published_revision.is_none() {
            self.published_revision = Some(revision);
            self.published = Some(candidate.clone());
            return Some(candidate);
        }
        if self.published_revision.as_deref() == Some(&revision) {
            self.pending = None;
            return None;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|(pending, _)| pending == &revision)
        {
            self.pending = None;
            self.published_revision = Some(revision);
            self.published = Some(candidate.clone());
            return Some(candidate);
        }
        self.pending = Some((revision, candidate));
        None
    }

    /// Most recently published immutable snapshot.
    #[must_use]
    pub fn published(&self) -> Option<&AssetDbSnapshot> {
        self.published.as_ref()
    }
}

#[derive(Default, Serialize, Deserialize)]
struct PersistedIndex {
    records: BTreeMap<String, PersistedFingerprint>,
}

#[derive(Clone, Serialize, Deserialize)]
struct PersistedFingerprint {
    bytes: u64,
    modified_unix_ms: u64,
    content_hash: String,
}

pub struct AssetDb;

impl AssetDb {
    /// Scan a project content root. The persisted fingerprint table implements
    /// two-stage invalidation: unchanged size/mtime reuses the hash; a touched
    /// file is hashed once and keeps the same preview when bytes still match.
    pub fn scan(root: impl AsRef<Path>) -> Result<AssetDbSnapshot, String> {
        let root = root.as_ref().to_path_buf();
        let state_dir = root.join(".somnium");
        let index_path = state_dir.join("asset-index.json");
        let old: PersistedIndex = fs::read(&index_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let mut next = PersistedIndex::default();
        let mut records = Vec::new();
        scan_dir(&root, &root, &old, &mut next, &mut records)?;
        let _ = fs::create_dir_all(&state_dir);
        if let Ok(bytes) = serde_json::to_vec_pretty(&next) {
            let _ = fs::write(&index_path, bytes);
        }
        records.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        let by_id = records
            .iter()
            .enumerate()
            .map(|(index, row)| (row.id, index))
            .collect();
        Ok(AssetDbSnapshot {
            root,
            records: records.into(),
            by_id: Arc::new(by_id),
        })
    }
}

fn scan_dir(
    root: &Path,
    dir: &Path,
    old: &PersistedIndex,
    next: &mut PersistedIndex,
    out: &mut Vec<AssetRecord>,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name.eq_ignore_ascii_case("bc7") {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let relative_path = normalise_relative(path.strip_prefix(root).unwrap_or(&path));
        let parent = path
            .parent()
            .and_then(|p| p.strip_prefix(root).ok())
            .map(normalise_relative)
            .unwrap_or_default();
        let kind = classify(&path, metadata.is_dir());
        let modified_unix_ms = modified_ms(&metadata);
        let bytes = if metadata.is_file() {
            metadata.len()
        } else {
            0
        };
        let content_hash = if metadata.is_file() {
            match old.records.get(&relative_path) {
                Some(previous)
                    if previous.bytes == bytes && previous.modified_unix_ms == modified_unix_ms =>
                {
                    previous.content_hash.clone()
                }
                _ => hash_file(&path).unwrap_or_default(),
            }
        } else {
            String::new()
        };
        if metadata.is_file() {
            next.records.insert(
                relative_path.clone(),
                PersistedFingerprint {
                    bytes,
                    modified_unix_ms,
                    content_hash: content_hash.clone(),
                },
            );
        }
        let (dimensions, triangles, doc_line) = kind_facts(&path, kind);
        out.push(AssetRecord {
            id: AssetId::from_relative_path(&relative_path),
            relative_path,
            absolute_path: path.clone(),
            name,
            parent,
            kind,
            metadata: AssetMetadata {
                bytes,
                modified_unix_ms,
                content_hash,
                dimensions,
                triangles,
                doc_line,
            },
        });
        if metadata.is_dir() {
            scan_dir(root, &path, old, next, out)?;
        }
    }
    Ok(())
}

#[must_use]
pub fn classify(path: &Path, is_dir: bool) -> AssetKind {
    if is_dir {
        return AssetKind::Folder;
    }
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" | "jpg" | "jpeg" | "bmp" | "tga" | "gif" | "webp" | "exr" | "hdr" => {
            AssetKind::Texture
        }
        "gltf" | "glb" => AssetKind::Mesh,
        "sommat" => AssetKind::Material,
        "somnium" => AssetKind::Scene,
        "somprefab" => AssetKind::Prefab,
        "luau" | "lua" | "rs" => AssetKind::Script,
        "wav" | "ogg" | "mp3" | "flac" => AssetKind::Audio,
        "wgsl" | "hlsl" | "glsl" => AssetKind::Shader,
        "ttf" | "otf" => AssetKind::Font,
        "json" => AssetKind::Json,
        _ => AssetKind::Unknown,
    }
}

fn kind_facts(path: &Path, kind: AssetKind) -> (Option<(u32, u32)>, Option<u64>, Option<String>) {
    match kind {
        AssetKind::Texture => {
            let dimensions = image::ImageReader::open(path)
                .ok()
                .and_then(|reader| reader.into_dimensions().ok());
            (dimensions, None, None)
        }
        AssetKind::Mesh => {
            let triangles = gltf::Gltf::open(path).ok().map(|gltf| {
                gltf.document
                    .meshes()
                    .flat_map(|mesh| mesh.primitives())
                    .map(|primitive| {
                        primitive.indices().map_or_else(
                            || {
                                primitive
                                    .get(&gltf::Semantic::Positions)
                                    .map_or(0, |a| a.count())
                            },
                            |a| a.count(),
                        ) as u64
                            / 3
                    })
                    .sum()
            });
            (None, triangles, None)
        }
        AssetKind::Script => {
            let doc = fs::read_to_string(path).ok().and_then(|text| {
                text.lines()
                    .map(str::trim)
                    .find(|line| line.starts_with("---") || line.starts_with("///"))
                    .map(|line| line.trim_start_matches(['-', '/']).trim().to_owned())
            });
            (None, None, doc)
        }
        _ => (None, None, None),
    }
}

fn hash_file(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).ok()?;
    Some(format!("{:x}", hasher.finalize()))
}

fn modified_ms(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| {
            duration.as_millis().min(u128::from(u64::MAX)) as u64
        })
}

fn normalise_relative(path: &Path) -> String {
    normalise_text(&path.to_string_lossy())
}

fn normalise_text(text: &str) -> String {
    text.replace('\\', "/")
        .trim_matches('/')
        .to_ascii_lowercase()
}

fn human_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / KIB)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / KIB / KIB)
    } else {
        format!("{:.1} GiB", bytes as f64 / KIB / KIB / KIB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    fn fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "somnium_asset_db_{}_{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("textures")).unwrap();
        fs::create_dir_all(root.join("models")).unwrap();
        fs::write(root.join("textures/rock.png"), b"not an image").unwrap();
        fs::write(root.join("models/ship.glb"), b"not a glb").unwrap();
        fs::write(root.join("boot.luau"), b"--- Starts the game\nreturn {}\n").unwrap();
        root
    }

    #[test]
    fn ids_normalise_case_and_separators() {
        assert_eq!(
            AssetId::from_relative_path("Textures\\Rock.PNG"),
            AssetId::from_relative_path("textures/rock.png")
        );
    }

    #[test]
    fn scan_queries_without_rewalking_the_filesystem() {
        let root = fixture();
        let db = AssetDb::scan(&root).unwrap();
        fs::remove_dir_all(root.join("textures")).unwrap();
        let rows = db.query(&AssetQuery {
            parent: "textures".into(),
            kind_mask: ASSET_KIND_TEXTURE,
            ..Default::default()
        });
        assert_eq!(
            rows.len(),
            1,
            "snapshot must survive the source disappearing"
        );
        assert_eq!(rows[0].name, "rock.png");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_touched_unchanged_file_keeps_its_content_hash() {
        let root = fixture();
        let first = AssetDb::scan(&root).unwrap();
        let id = AssetId::from_relative_path("boot.luau");
        let hash = first.get(id).unwrap().metadata.content_hash.clone();
        fs::write(root.join("boot.luau"), b"--- Starts the game\nreturn {}\n").unwrap();
        let second = AssetDb::scan(&root).unwrap();
        assert_eq!(second.get(id).unwrap().metadata.content_hash, hash);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn query_sorts_folders_before_files_and_filters_kinds() {
        let root = fixture();
        let db = AssetDb::scan(&root).unwrap();
        let all = db.query(&AssetQuery {
            kind_mask: ASSET_KIND_ALL,
            ..Default::default()
        });
        assert!(all[0].kind == AssetKind::Folder);
        let scripts = db.search("boot", AssetKind::Script.bit());
        assert_eq!(scripts.len(), 1);
        assert_eq!(
            scripts[0].metadata.doc_line.as_deref(),
            Some("Starts the game")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn debounce_never_publishes_a_single_intermediate_write() {
        let root = fixture();
        let mut gate = DebouncedAssetDb::default();
        let first = AssetDb::scan(&root).unwrap();
        assert!(gate.stage(first).is_some());
        fs::write(root.join("boot.luau"), "return { half = true }").unwrap();
        let half_written = AssetDb::scan(&root).unwrap();
        assert!(gate.stage(half_written).is_none());
        fs::write(root.join("boot.luau"), "return { complete = true }").unwrap();
        let complete = AssetDb::scan(&root).unwrap();
        assert!(gate.stage(complete.clone()).is_none());
        assert!(gate.stage(complete).is_some());
        let _ = fs::remove_dir_all(root);
    }
}
