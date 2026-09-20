//! GPU upload reuse for authored glTF sources within one renderer lifetime.
//! Metadata stamps match the asset watcher's invalidation contract. Only the
//! JSON header is read from GLBs; large embedded buffers are never hashed here.
use somnium_renderer::renderer::UploadedNode;
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    time::SystemTime,
};

const MAX_SOURCES: usize = 128;
const MAX_NODES: usize = 32_768;
const MAX_DEPENDENCIES: usize = 4096;
const MAX_JSON_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
    length: u64,
    modified: SystemTime,
}

fn file_stamp(path: &Path) -> Option<Option<FileStamp>> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => Some(Some(FileStamp {
            length: meta.len(),
            modified: meta.modified().ok()?,
        })),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(None),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SourceStamp {
    source: PathBuf,
    files: BTreeMap<PathBuf, Option<FileStamp>>,
}

impl SourceStamp {
    /// Unsupported or unreadable sources simply bypass the cache; the importer
    /// retains responsibility for reporting its own useful loading errors.
    pub(super) fn read(source: &Path) -> Option<Self> {
        let source = source.canonicalize().ok()?;
        let mut files = BTreeMap::new();
        let stamp = file_stamp(&source)??;
        let mut stream = File::open(&source).ok()?;
        let mut json = Vec::new();
        if source.extension()?.eq_ignore_ascii_case("glb") {
            let mut header = [0_u8; 20];
            stream.read_exact(&mut header).ok()?;
            let word = |offset: usize| {
                u32::from_le_bytes([
                    header[offset],
                    header[offset + 1],
                    header[offset + 2],
                    header[offset + 3],
                ])
            };
            if &header[..4] != b"glTF" || word(4) != 2 || &header[16..20] != b"JSON" {
                return None;
            }
            let length = u64::from(word(12));
            if length > MAX_JSON_BYTES || length + 20 > stamp.length {
                return None;
            }
            stream.take(length).read_to_end(&mut json).ok()?;
        } else if source.extension()?.eq_ignore_ascii_case("gltf") {
            if stamp.length > MAX_JSON_BYTES {
                return None;
            }
            stream.read_to_end(&mut json).ok()?;
        } else {
            return None;
        }
        files.insert(source.clone(), Some(stamp));
        let document: serde_json::Value = serde_json::from_slice(&json).ok()?;
        let parent = source.parent()?;
        for group in ["buffers", "images"] {
            for value in document
                .get(group)
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(uri) = value.get("uri").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                if uri.starts_with("data:") {
                    continue;
                }
                let dependency = parent.join(decode_uri(uri)?);
                let dependency_stamp = file_stamp(&dependency)??;
                files.insert(dependency, Some(dependency_stamp));
                if group == "images" {
                    // Match somnium_asset::sidecar_alpha_path, but retain the
                    // absent candidate too so a newly added mask invalidates.
                    let uri = uri.replace("%20", " ");
                    let image = parent.join(uri);
                    let name = image.file_name()?.to_str()?;
                    if let Some((before, after)) = name.split_once("_diff") {
                        let after = after.rsplit_once('.').map_or("", |(stem, _)| stem);
                        let mask = image.with_file_name(format!("{before}_alpha{after}.png"));
                        let stamp = file_stamp(&mask)?;
                        files.insert(mask, stamp);
                    }
                }
                if files.len() > MAX_DEPENDENCIES {
                    return None;
                }
            }
        }
        let result = Self { source, files };
        result.is_current().then_some(result)
    }

    pub(super) fn is_current(&self) -> bool {
        self.files
            .iter()
            .all(|(path, expected)| file_stamp(path).as_ref() == Some(expected))
    }
}

fn decode_uri(uri: &str) -> Option<String> {
    // Remote/file URI schemes use importer-specific resolution; do not guess.
    if uri.contains(':') {
        return None;
    }
    let bytes = uri.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let high = (*bytes.get(i + 1)? as char).to_digit(16)?;
            let low = (*bytes.get(i + 2)? as char).to_digit(16)?;
            decoded.push((high * 16 + low) as u8);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

struct Entry {
    stamp: SourceStamp,
    nodes: Vec<UploadedNode>,
    used: u64,
}

#[derive(Default)]
pub(super) struct ImportedUploadCache {
    sources: BTreeMap<PathBuf, Entry>,
    clock: u64,
    nodes: usize,
}

impl ImportedUploadCache {
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn get(&mut self, stamp: &SourceStamp) -> Option<Vec<UploadedNode>> {
        let entry = self.sources.get_mut(&stamp.source)?;
        if entry.stamp != *stamp {
            return None;
        }
        self.clock = self.clock.saturating_add(1);
        entry.used = self.clock;
        Some(entry.nodes.clone())
    }

    pub(super) fn insert(&mut self, stamp: SourceStamp, nodes: &[UploadedNode]) {
        if nodes.is_empty() || nodes.len() > MAX_NODES || !stamp.is_current() {
            return;
        }
        if let Some(previous) = self.sources.remove(&stamp.source) {
            self.nodes -= previous.nodes.len();
        }
        while self.sources.len() >= MAX_SOURCES || self.nodes + nodes.len() > MAX_NODES {
            let Some(oldest) = self
                .sources
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(path, _)| path.clone())
            else {
                break;
            };
            if let Some(entry) = self.sources.remove(&oldest) {
                self.nodes -= entry.nodes.len();
            }
        }
        self.clock = self.clock.saturating_add(1);
        self.nodes += nodes.len();
        self.sources.insert(
            stamp.source.clone(),
            Entry {
                stamp,
                nodes: nodes.to_vec(),
                used: self.clock,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "somnium-import-cache-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("leaf_diff.png"), b"image").unwrap();
            std::fs::write(path.join("mesh.bin"), b"geometry").unwrap();
            std::fs::write(path.join("mesh.gltf"), br#"{"asset":{"version":"2.0"},"buffers":[{"uri":"mesh.bin"}],"images":[{"uri":"leaf_diff.png"}]}"#).unwrap();
            Self(path)
        }
        fn stamp(&self) -> SourceStamp {
            SourceStamp::read(&self.0.join("mesh.gltf")).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            // Only this fixture's known leaves, never a recursive delete.
            for name in ["mesh.gltf", "mesh.bin", "leaf_diff.png", "leaf_alpha.png"] {
                let _ = std::fs::remove_file(self.0.join(name));
            }
            let _ = std::fs::remove_dir(&self.0);
        }
    }

    #[test]
    fn source_buffer_texture_and_new_alpha_invalidate_uploads() {
        let fixture = Fixture::new();
        let first = fixture.stamp();
        assert_eq!(first, fixture.stamp());
        for (name, bytes) in [
            ("mesh.bin", b"changed geometry".as_slice()),
            ("leaf_diff.png", b"changed image"),
            ("leaf_alpha.png", b"new mask"),
        ] {
            let before = fixture.stamp();
            std::fs::write(fixture.0.join(name), bytes).unwrap();
            assert!(!before.is_current());
            assert_ne!(before, fixture.stamp());
        }
        let before = fixture.stamp();
        let path = fixture.0.join("mesh.gltf");
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.push(b' ');
        std::fs::write(path, bytes).unwrap();
        assert!(!before.is_current());
    }

    #[test]
    fn warm_upload_reuses_nodes_and_renderer_reset_discards_them() {
        let fixture = Fixture::new();
        let stamp = fixture.stamp();
        let node = UploadedNode {
            entity_name: "tree".into(),
            vertex_offset: 71,
            index_offset: 93,
            index_count: 24,
            material_id: 8,
            material_index: 0,
            transform: glam::Mat4::IDENTITY,
        };
        let mut cache = ImportedUploadCache::default();
        cache.insert(stamp.clone(), &[node]);
        let hit = cache.get(&fixture.stamp()).unwrap();
        assert_eq!(
            (
                hit[0].vertex_offset,
                hit[0].index_offset,
                hit[0].material_id
            ),
            (71, 93, 8)
        );
        cache.clear();
        assert!(cache.get(&stamp).is_none());
    }
}
