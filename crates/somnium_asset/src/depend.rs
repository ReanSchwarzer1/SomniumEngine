//! What references what, across a whole project.
//!
//! MORROWIND-M item 3. [`crate::cook::AssetDependencyGraph`] answers the same
//! shape of question for a *cook plan*, where every edge was declared by
//! whoever wrote the plan. The editor cannot ask that: nobody declares that a
//! scene uses a mesh, they drop the mesh onto an entity and the edge exists.
//! So this module reads the project and works the edges out.
//!
//! Three questions, and they are not the same question:
//!
//! ```text
//!   what does this reference?      forward, direct   — "open the texture this
//!                                                      material paints with"
//!   what references this?          reverse, direct   — "who is using this?"
//!   what breaks if I delete it?    reverse, closed   — a texture deleted
//!                                                      breaks its material,
//!                                                      and every scene that
//!                                                      material appears in
//! ```
//!
//! The third is the one that has to be transitive, and it is the only one
//! anybody asks with their finger over the delete key.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    database::{AssetDbSnapshot, AssetId, AssetKind, AssetRecord},
    scene_file,
};

/// How an asset id appears inside a project file.
///
/// Two spellings, because two formats arrived at it separately: the scene
/// schema tags a reference so it survives a round trip through a generic value
/// (`{"$asset": "…"}`), and a material declares typed `AssetId` fields that
/// serialise as the bare string. Both are 32 lowercase hex digits, which is
/// what makes one scanner enough for both.
fn parse_id(text: &str) -> Option<AssetId> {
    if text.len() != 32 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let raw = u128::from_str_radix(text, 16).ok()?;
    (raw != 0).then(|| AssetId::from_raw(raw))
}

/// Every asset id mentioned anywhere in a JSON document.
///
/// Deliberately structural rather than schema-driven. Teaching this the
/// component schemas would mean a new asset field silently not showing up in
/// the dependency view until somebody remembered to teach it twice — and the
/// place people forget is exactly the place where "what breaks if I delete
/// this" starts lying.
#[must_use]
pub fn references_in_json(value: &serde_json::Value) -> BTreeSet<AssetId> {
    let mut found = BTreeSet::new();
    collect(value, &mut found);
    found
}

fn collect(value: &serde_json::Value, found: &mut BTreeSet<AssetId>) {
    match value {
        serde_json::Value::String(text) => {
            if let Some(id) = parse_id(text) {
                found.insert(id);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect(item, found);
            }
        }
        serde_json::Value::Object(fields) => {
            for field in fields.values() {
                collect(field, found);
            }
        }
        _ => {}
    }
}

/// Whether a kind stores its references somewhere this module can read them.
///
/// A `.glb` names its own textures and a `.wgsl` includes other shaders, and
/// neither is JSON. Saying so out loud is better than a dependency view that
/// quietly reports a mesh as referencing nothing.
#[must_use]
pub const fn is_scannable(kind: AssetKind) -> bool {
    matches!(
        kind,
        AssetKind::Scene
            | AssetKind::Prefab
            | AssetKind::Material
            | AssetKind::Json
            | AssetKind::UiDocument
    )
}

/// Which project files were read, and which were not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanSummary {
    /// Files opened and understood.
    pub scanned: usize,
    /// Files of a kind this module cannot read references out of.
    pub opaque: usize,
    /// Files that should have been readable and were not — missing, locked, or
    /// not the JSON their extension promised.
    pub unreadable: usize,
}

/// The project's reference graph, both ways round.
#[derive(Clone, Debug, Default)]
pub struct DependencyIndex {
    forward: BTreeMap<AssetId, BTreeSet<AssetId>>,
    reverse: BTreeMap<AssetId, BTreeSet<AssetId>>,
    dangling: BTreeMap<AssetId, BTreeSet<AssetId>>,
    summary: ScanSummary,
}

impl DependencyIndex {
    /// Read every scannable asset in the snapshot and index what it names.
    ///
    /// This touches the disk once per readable asset, so it belongs on a job
    /// rather than in a frame — see [`crate::database::AssetDb::scan`], which
    /// has the same shape and the same reason.
    #[must_use]
    pub fn build(snapshot: &AssetDbSnapshot) -> Self {
        let known: BTreeSet<AssetId> = snapshot.records().iter().map(|record| record.id).collect();
        let mut index = Self::default();
        for record in snapshot.records() {
            if record.kind == AssetKind::Folder {
                continue;
            }
            if !is_scannable(record.kind) {
                index.summary.opaque += 1;
                continue;
            }
            match read_json(record) {
                Some(document) => {
                    index.summary.scanned += 1;
                    index.insert(record.id, references_in_json(&document), &known);
                }
                None => index.summary.unreadable += 1,
            }
        }
        index
    }

    /// Index one asset's references directly, for callers holding the document
    /// already — and for tests, which should not have to write a project out.
    pub fn insert(
        &mut self,
        asset: AssetId,
        references: BTreeSet<AssetId>,
        known: &BTreeSet<AssetId>,
    ) {
        for referenced in references {
            // An asset naming itself is not a dependency, and left in it would
            // make every self-referencing asset its own breakage.
            if referenced == asset {
                continue;
            }
            if known.contains(&referenced) {
                self.forward.entry(asset).or_default().insert(referenced);
                self.reverse.entry(referenced).or_default().insert(asset);
            } else {
                self.dangling.entry(asset).or_default().insert(referenced);
            }
        }
    }

    /// What `asset` names. Direct edges only.
    #[must_use]
    pub fn references(&self, asset: AssetId) -> Vec<AssetId> {
        self.forward
            .get(&asset)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// What names `asset`. Direct edges only.
    #[must_use]
    pub fn referenced_by(&self, asset: AssetId) -> Vec<AssetId> {
        self.reverse
            .get(&asset)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Everything that would gain a broken reference if `asset` went away.
    ///
    /// Transitive, and `asset` itself is not in it. A texture deleted breaks
    /// the material that paints with it, and a material that cannot be built
    /// breaks every scene it appears in — which is why the answer to the only
    /// question anyone asks under time pressure cannot be the direct edges.
    #[must_use]
    pub fn breakage(&self, asset: AssetId) -> Vec<AssetId> {
        let mut seen = BTreeSet::from([asset]);
        let mut frontier = vec![asset];
        while let Some(current) = frontier.pop() {
            let Some(dependents) = self.reverse.get(&current) else {
                continue;
            };
            for dependent in dependents {
                // A cycle would otherwise be an infinite walk; `seen` is the
                // whole guard, and a project *can* contain one — a prefab that
                // spawns a scene that places the prefab.
                if seen.insert(*dependent) {
                    frontier.push(*dependent);
                }
            }
        }
        seen.remove(&asset);
        seen.into_iter().collect()
    }

    /// References `asset` makes that resolve to nothing in the project.
    ///
    /// Usually the trail of an asset deleted from disk without the editor —
    /// the thing the reference view exists to make visible before it is a
    /// mystery at runtime.
    #[must_use]
    pub fn dangling(&self, asset: AssetId) -> Vec<AssetId> {
        self.dangling
            .get(&asset)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Assets nothing references. Folders and unreadable kinds are not in the
    /// graph at all, so this only speaks about what was actually indexed.
    #[must_use]
    pub fn is_referenced(&self, asset: AssetId) -> bool {
        self.reverse.contains_key(&asset)
    }

    #[must_use]
    pub fn summary(&self) -> ScanSummary {
        self.summary
    }

    /// How many assets have at least one outgoing edge.
    #[must_use]
    pub fn edge_sources(&self) -> usize {
        self.forward.len()
    }
}

fn read_json(record: &AssetRecord) -> Option<serde_json::Value> {
    if record.kind == AssetKind::Scene {
        // A scene is a binary header followed by the document, so the file is
        // not JSON even though its body is.
        return scene_file::read(&record.absolute_path)
            .ok()
            .map(|(_, body)| body);
    }
    let text = std::fs::read_to_string(&record.absolute_path).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(path: &str) -> AssetId {
        AssetId::from_relative_path(path)
    }

    fn known(paths: &[&str]) -> BTreeSet<AssetId> {
        paths.iter().map(|p| id(p)).collect()
    }

    #[test]
    fn both_spellings_of_a_reference_are_found() {
        // The scene schema tags its references; a material writes the bare
        // string. A scanner that knew only one would report half a project.
        let texture = id("textures/rock.png");
        let document = json!({
            "entities": [{ "components": { "mesh": { "$asset": texture.to_string() } } }],
            "albedo_map": texture.to_string(),
        });
        assert_eq!(
            references_in_json(&document),
            BTreeSet::from([texture]),
            "both spellings name the same asset once"
        );
    }

    #[test]
    fn things_that_merely_look_like_ids_are_not_ids() {
        // A content hash is 64 hex digits, a version is not hex at all, and
        // the all-zero id is `AssetId::NONE` — an *empty* asset field, which
        // is the opposite of a reference.
        let document = json!({
            "content_hash": "a".repeat(64),
            "note": "deadbeef",
            "empty_field": "0".repeat(32),
            "not_hex": "z".repeat(32),
        });
        assert!(references_in_json(&document).is_empty());
    }

    #[test]
    fn a_reference_to_something_absent_is_dangling_not_forgotten() {
        // The trail of a file deleted outside the editor. Dropping it silently
        // is how a project reaches runtime with a missing texture nobody could
        // have seen coming.
        let scene = id("maps/town.scene");
        let gone = id("textures/deleted.png");
        let mut index = DependencyIndex::default();
        index.insert(scene, BTreeSet::from([gone]), &known(&["maps/town.scene"]));

        assert!(index.references(scene).is_empty());
        assert_eq!(index.dangling(scene), vec![gone]);
        assert!(index.breakage(gone).is_empty());
    }

    #[test]
    fn deleting_a_texture_breaks_the_scene_two_steps_away() {
        // The whole point of the transitive answer. A scene does not name the
        // texture; it names the material, and the material names the texture.
        let texture = id("textures/rock.png");
        let material = id("materials/rock.mat");
        let scene = id("maps/town.scene");
        let unrelated = id("maps/cave.scene");
        let all = known(&[
            "textures/rock.png",
            "materials/rock.mat",
            "maps/town.scene",
            "maps/cave.scene",
        ]);
        let mut index = DependencyIndex::default();
        index.insert(material, BTreeSet::from([texture]), &all);
        index.insert(scene, BTreeSet::from([material]), &all);

        assert_eq!(index.references(scene), vec![material]);
        assert_eq!(index.referenced_by(texture), vec![material]);
        assert_eq!(
            index.breakage(texture),
            {
                let mut expected = vec![material, scene];
                expected.sort();
                expected
            },
            "the scene is two steps away and breaks all the same"
        );
        assert!(index.breakage(unrelated).is_empty());
        assert!(!index.is_referenced(unrelated));
    }

    #[test]
    fn a_cycle_is_walked_once_rather_than_forever() {
        // A prefab that spawns a scene that places the prefab. Rare, legal,
        // and an infinite loop for any closure without a visited set.
        let a = id("prefabs/door.prefab");
        let b = id("maps/hall.scene");
        let all = known(&["prefabs/door.prefab", "maps/hall.scene"]);
        let mut index = DependencyIndex::default();
        index.insert(a, BTreeSet::from([b]), &all);
        index.insert(b, BTreeSet::from([a]), &all);

        assert_eq!(index.breakage(a), vec![b]);
        assert_eq!(index.breakage(b), vec![a]);
    }

    #[test]
    fn an_asset_naming_itself_is_not_its_own_dependency() {
        let scene = id("maps/town.scene");
        let mut index = DependencyIndex::default();
        index.insert(scene, BTreeSet::from([scene]), &known(&["maps/town.scene"]));
        assert!(index.references(scene).is_empty());
        assert!(index.breakage(scene).is_empty());
    }

    #[test]
    fn the_kinds_this_cannot_read_are_counted_rather_than_claimed_empty() {
        // A `.glb` names its own textures and this module cannot see them. The
        // difference between "references nothing" and "could not be read" is
        // the difference between a safe delete and a broken one.
        assert!(is_scannable(AssetKind::Scene));
        assert!(is_scannable(AssetKind::Material));
        assert!(is_scannable(AssetKind::UiDocument));
        assert!(!is_scannable(AssetKind::Mesh));
        assert!(!is_scannable(AssetKind::Texture));
        assert!(!is_scannable(AssetKind::Shader));
    }
    // ── Through a real project on disk ──────────────────────────────────────

    static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    /// A three-link chain written out as the editor would write it: a texture,
    /// a material that paints with it, and a scene that places the material.
    fn project() -> std::path::PathBuf {
        use std::fs;
        let root = std::env::temp_dir().join(format!(
            "somnium_depend_{}_{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("textures")).unwrap();
        fs::create_dir_all(root.join("materials")).unwrap();
        fs::create_dir_all(root.join("maps")).unwrap();
        fs::write(root.join("textures/rock.png"), b"not an image").unwrap();

        let material = crate::material::MaterialAsset {
            albedo_map: id("textures/rock.png"),
            ..Default::default()
        };
        fs::write(
            root.join("materials/rock.sommat"),
            serde_json::to_string_pretty(&material).unwrap(),
        )
        .unwrap();

        // A scene is framed, not bare JSON, which is the half of this that a
        // unit test on the extractor cannot reach.
        let body = serde_json::json!({
            "version": 1,
            "entities": [{
                "components": {
                    "material": { "$asset": id("materials/rock.sommat").to_string() }
                }
            }]
        });
        let bytes = scene_file::encode(&scene_file::SceneHeader::default(), &body);
        fs::write(root.join("maps/town.somnium"), bytes).unwrap();
        root
    }

    #[test]
    fn a_real_project_yields_the_chain_the_editor_would_draw() {
        let root = project();
        let snapshot = crate::database::AssetDb::scan(&root).unwrap();
        let index = DependencyIndex::build(&snapshot);

        let texture = id("textures/rock.png");
        let material = id("materials/rock.sommat");
        let scene = id("maps/town.somnium");

        assert_eq!(
            index.references(material),
            vec![texture],
            "material -> texture"
        );
        assert_eq!(index.references(scene), vec![material], "scene -> material");
        assert_eq!(index.referenced_by(texture), vec![material]);
        assert_eq!(
            index.breakage(texture).len(),
            2,
            "the scene is two steps away"
        );

        // The texture is opaque to the index and says so rather than claiming
        // to reference nothing.
        assert!(!is_scannable(crate::database::AssetKind::Texture));
        let summary = index.summary();
        assert_eq!(
            summary.scanned, 2,
            "the material and the scene: {summary:?}"
        );
        assert_eq!(summary.opaque, 1, "the texture: {summary:?}");
        assert_eq!(summary.unreadable, 0, "{summary:?}");

        let _ = std::fs::remove_dir_all(&root);
    }
}
