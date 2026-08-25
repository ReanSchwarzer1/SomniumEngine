//! MORROWIND-Q deterministic native asset cook.
//!
//! Source paths remain the identity boundary: [`AssetId`] is derived exactly
//! as it is by the editor database. The cook changes representation, not
//! identity. A content-addressed artifact per asset plus a replaceable manifest
//! also leaves room for Defold-style live update later: a patch can add blobs
//! and atomically replace the manifest without changing this format.

use crate::database::AssetId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use somnium_jobs::{JobDesc, JobError, JobHandle, JobPriority, JobSystem};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

const ARTIFACT_MAGIC: &[u8; 8] = b"SOMCOOK\0";
const NATIVE_VERSION: u32 = 1;
const MAX_DEPENDENCIES: usize = 65_536;
const MAX_PAYLOAD_BYTES: usize = 8 * 1024 * 1024 * 1024;

/// Version of the common cooked-asset envelope.
pub const COOK_FORMAT_VERSION: u32 = 1;

/// A native payload family. Each has a distinct durable extension and payload
/// magic even though all share the validated common envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum CookKind {
    Mesh = 1,
    Texture = 2,
    Audio = 3,
    Scene = 4,
    Prefab = 5,
    Shader = 6,
    Material = 7,
}

impl CookKind {
    /// Native file extension written into a build.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Mesh => "sommesh",
            Self::Texture => "somtex",
            Self::Audio => "somaudio",
            Self::Scene => "somscene",
            Self::Prefab => "somprefab",
            Self::Shader => "somshader",
            Self::Material => "sommatc",
        }
    }

    const fn payload_magic(self) -> [u8; 8] {
        match self {
            Self::Mesh => *b"SOMMESH\0",
            Self::Texture => *b"SOMTEX\0\0",
            Self::Audio => *b"SOMAUDIO",
            Self::Scene => *b"SOMSCENE",
            Self::Prefab => *b"SOMPREF\0",
            Self::Shader => *b"SOMSHDR\0",
            Self::Material => *b"SOMMATC\0",
        }
    }

    fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::Mesh,
            2 => Self::Texture,
            3 => Self::Audio,
            4 => Self::Scene,
            5 => Self::Prefab,
            6 => Self::Shader,
            7 => Self::Material,
            _ => return None,
        })
    }

    fn is_text(self) -> bool {
        matches!(
            self,
            Self::Scene | Self::Prefab | Self::Shader | Self::Material
        )
    }
}

/// One source asset and its direct dependency ids.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookRequest {
    pub source: String,
    pub kind: CookKind,
    #[serde(default)]
    pub dependencies: Vec<AssetId>,
}

impl CookRequest {
    /// Identity shared by source and cooked representations.
    #[must_use]
    pub fn asset_id(&self) -> AssetId {
        AssetId::from_relative_path(&self.source)
    }
}

/// Serializable tool input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookPlan {
    pub version: u32,
    pub cooker_version: u32,
    pub assets: Vec<CookRequest>,
}

/// Direct dependency graph used by incremental invalidation and diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssetDependencyGraph {
    direct: BTreeMap<AssetId, BTreeSet<AssetId>>,
}

impl AssetDependencyGraph {
    pub fn from_requests(requests: &[CookRequest]) -> Result<Self, String> {
        let ids: BTreeSet<_> = requests.iter().map(CookRequest::asset_id).collect();
        if ids.len() != requests.len() {
            return Err("cook plan contains duplicate AssetIds".into());
        }
        let mut direct = BTreeMap::new();
        for request in requests {
            let dependencies: BTreeSet<_> = request.dependencies.iter().copied().collect();
            if dependencies
                .iter()
                .any(|dependency| !ids.contains(dependency))
            {
                return Err(format!(
                    "{} names a dependency absent from the cook plan",
                    request.source
                ));
            }
            direct.insert(request.asset_id(), dependencies);
        }
        let graph = Self { direct };
        graph.topological_order()?;
        Ok(graph)
    }

    /// Asset plus every reverse-dependent that must be recooked.
    #[must_use]
    pub fn affected_by(&self, changed: AssetId) -> BTreeSet<AssetId> {
        let mut affected = BTreeSet::from([changed]);
        loop {
            let before = affected.len();
            for (asset, dependencies) in &self.direct {
                if dependencies
                    .iter()
                    .any(|dependency| affected.contains(dependency))
                {
                    affected.insert(*asset);
                }
            }
            if affected.len() == before {
                return affected;
            }
        }
    }

    fn topological_order(&self) -> Result<Vec<AssetId>, String> {
        fn visit(
            node: AssetId,
            graph: &AssetDependencyGraph,
            temporary: &mut BTreeSet<AssetId>,
            permanent: &mut BTreeSet<AssetId>,
            output: &mut Vec<AssetId>,
        ) -> Result<(), String> {
            if permanent.contains(&node) {
                return Ok(());
            }
            if !temporary.insert(node) {
                return Err(format!("asset dependency cycle reaches {node}"));
            }
            if let Some(dependencies) = graph.direct.get(&node) {
                for dependency in dependencies {
                    visit(*dependency, graph, temporary, permanent, output)?;
                }
            }
            temporary.remove(&node);
            permanent.insert(node);
            output.push(node);
            Ok(())
        }

        let mut output = Vec::with_capacity(self.direct.len());
        let mut temporary = BTreeSet::new();
        let mut permanent = BTreeSet::new();
        for node in self.direct.keys().copied() {
            visit(node, self, &mut temporary, &mut permanent, &mut output)?;
        }
        Ok(output)
    }
}

/// Decoded, integrity-checked native artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CookedAsset {
    pub kind: CookKind,
    pub asset: AssetId,
    pub cooker_version: u32,
    pub source_hash: [u8; 32],
    pub recipe_hash: [u8; 32],
    pub payload_hash: [u8; 32],
    pub dependencies: Vec<(AssetId, [u8; 32])>,
    pub payload: Vec<u8>,
}

impl CookedAsset {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut dependencies = self.dependencies.clone();
        dependencies.sort_by_key(|(asset, _)| *asset);
        let mut output = Vec::with_capacity(144 + dependencies.len() * 48 + self.payload.len());
        output.extend_from_slice(ARTIFACT_MAGIC);
        output.extend_from_slice(&COOK_FORMAT_VERSION.to_le_bytes());
        output.extend_from_slice(&self.cooker_version.to_le_bytes());
        output.push(self.kind as u8);
        output.extend_from_slice(&[0; 3]);
        output.extend_from_slice(&self.asset.raw().to_le_bytes());
        output.extend_from_slice(&self.source_hash);
        output.extend_from_slice(&self.recipe_hash);
        output.extend_from_slice(&self.payload_hash);
        output.extend_from_slice(&(dependencies.len() as u32).to_le_bytes());
        output.extend_from_slice(&(self.payload.len() as u64).to_le_bytes());
        for (asset, recipe_hash) in dependencies {
            output.extend_from_slice(&asset.raw().to_le_bytes());
            output.extend_from_slice(&recipe_hash);
        }
        output.extend_from_slice(&self.payload);
        output
    }

    /// Decode and validate framing, payload family and hashes.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        const HEADER: usize = 144;
        if bytes.len() < HEADER || &bytes[..8] != ARTIFACT_MAGIC {
            return Err("not a Somnium cooked artifact".into());
        }
        let format = read_u32(bytes, 8)?;
        if format != COOK_FORMAT_VERSION {
            return Err(format!("unsupported cooked format {format}"));
        }
        let cooker_version = read_u32(bytes, 12)?;
        let kind = CookKind::from_u8(bytes[16]).ok_or_else(|| "unknown cooked kind".to_string())?;
        let asset = AssetId::from_raw(read_u128(bytes, 20)?);
        let source_hash = read_hash(bytes, 36)?;
        let recipe_hash = read_hash(bytes, 68)?;
        let payload_hash = read_hash(bytes, 100)?;
        let dependency_count = read_u32(bytes, 132)? as usize;
        let payload_len = read_u64(bytes, 136)? as usize;
        if dependency_count > MAX_DEPENDENCIES || payload_len > MAX_PAYLOAD_BYTES {
            return Err("cooked artifact claims unreasonable sizes".into());
        }
        let payload_start = HEADER
            .checked_add(
                dependency_count
                    .checked_mul(48)
                    .ok_or("dependency overflow")?,
            )
            .ok_or("artifact overflow")?;
        let end = payload_start
            .checked_add(payload_len)
            .ok_or("payload overflow")?;
        if end != bytes.len() {
            return Err("cooked artifact is truncated or has trailing bytes".into());
        }
        let mut dependencies = Vec::with_capacity(dependency_count);
        for index in 0..dependency_count {
            let offset = HEADER + index * 48;
            dependencies.push((
                AssetId::from_raw(read_u128(bytes, offset)?),
                read_hash(bytes, offset + 16)?,
            ));
        }
        if !dependencies.windows(2).all(|pair| pair[0].0 < pair[1].0) {
            return Err("cooked dependencies are not unique and sorted".into());
        }
        let payload = bytes[payload_start..end].to_vec();
        if sha256(&payload) != payload_hash {
            return Err("cooked payload hash mismatch".into());
        }
        validate_native_payload(kind, &payload)?;
        Ok(Self {
            kind,
            asset,
            cooker_version,
            source_hash,
            recipe_hash,
            payload_hash,
            dependencies,
            payload,
        })
    }
}

#[derive(Clone, Debug)]
pub struct CookConfig {
    pub source_root: PathBuf,
    pub output_root: PathBuf,
    pub cache_root: PathBuf,
    pub cooker_version: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CookStatus {
    Cooked,
    Cached,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookManifestEntry {
    pub asset: AssetId,
    pub kind: CookKind,
    pub source: String,
    pub artifact: String,
    pub source_hash: String,
    pub recipe_hash: String,
    pub artifact_hash: String,
    pub dependencies: Vec<AssetId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookManifest {
    pub format_version: u32,
    pub cooker_version: u32,
    pub assets: Vec<CookManifestEntry>,
}

impl CookManifest {
    #[must_use]
    pub fn to_json(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec_pretty(self).unwrap_or_default();
        bytes.push(b'\n');
        bytes
    }

    #[must_use]
    pub fn get(&self, asset: AssetId) -> Option<&CookManifestEntry> {
        self.assets
            .binary_search_by_key(&asset, |entry| entry.asset)
            .ok()
            .and_then(|index| self.assets.get(index))
    }
}

#[derive(Clone, Debug)]
pub struct CookReport {
    pub manifest: CookManifest,
    pub status: BTreeMap<AssetId, CookStatus>,
}

/// Deterministic incremental cooker.
pub struct AssetCooker {
    config: CookConfig,
}

impl AssetCooker {
    #[must_use]
    pub fn new(config: CookConfig) -> Self {
        Self { config }
    }

    pub fn cook(&self, requests: &[CookRequest]) -> Result<CookReport, String> {
        self.cook_with_cancel(requests, || Ok(()))
    }

    fn cook_with_cancel(
        &self,
        requests: &[CookRequest],
        mut check_cancelled: impl FnMut() -> Result<(), String>,
    ) -> Result<CookReport, String> {
        let graph = AssetDependencyGraph::from_requests(requests)?;
        let order = graph.topological_order()?;
        let requests_by_id: BTreeMap<_, _> = requests
            .iter()
            .map(|request| (request.asset_id(), request))
            .collect();
        let mut recipe_hashes = BTreeMap::new();
        let mut status = BTreeMap::new();
        let mut entries = BTreeMap::new();
        for asset in order {
            check_cancelled()?;
            let request = requests_by_id[&asset];
            let source = safe_relative(&request.source)?;
            let source_bytes = fs::read(self.config.source_root.join(&source))
                .map_err(|error| format!("read {}: {error}", request.source))?;
            let canonical = canonical_source(request.kind, &source_bytes)?;
            let source_hash = sha256(&canonical);
            let mut dependencies = request.dependencies.to_vec();
            dependencies.sort_unstable();
            dependencies.dedup();
            let dependency_recipes: Vec<_> = dependencies
                .iter()
                .map(|dependency| {
                    recipe_hashes
                        .get(dependency)
                        .copied()
                        .map(|hash| (*dependency, hash))
                        .ok_or_else(|| format!("dependency {dependency} was not cooked first"))
                })
                .collect::<Result<_, _>>()?;
            let recipe_hash = recipe_hash(
                self.config.cooker_version,
                request.kind,
                asset,
                source_hash,
                &dependency_recipes,
            );
            let payload = native_payload(request.kind, &canonical);
            let payload_hash = sha256(&payload);
            let artifact = CookedAsset {
                kind: request.kind,
                asset,
                cooker_version: self.config.cooker_version,
                source_hash,
                recipe_hash,
                payload_hash,
                dependencies: dependency_recipes,
                payload,
            };
            let cache_path = self.config.cache_root.join(format!(
                "{}.{}",
                hex(&recipe_hash),
                request.kind.extension()
            ));
            let cached = fs::read(&cache_path)
                .ok()
                .and_then(|bytes| CookedAsset::decode(&bytes).ok().map(|asset| (bytes, asset)))
                .filter(|(_, cached)| {
                    cached.asset == asset
                        && cached.kind == request.kind
                        && cached.cooker_version == self.config.cooker_version
                        && cached.recipe_hash == recipe_hash
                });
            let (artifact_bytes, cook_status) = cached.map_or_else(
                || {
                    let bytes = artifact.encode();
                    (bytes, CookStatus::Cooked)
                },
                |(bytes, _)| (bytes, CookStatus::Cached),
            );
            if cook_status == CookStatus::Cooked {
                write_atomic(&cache_path, &artifact_bytes)?;
            }
            let relative_artifact = format!("assets/{asset}.{}", request.kind.extension());
            write_if_changed(
                &self.config.output_root.join(&relative_artifact),
                &artifact_bytes,
            )?;
            recipe_hashes.insert(asset, recipe_hash);
            status.insert(asset, cook_status);
            entries.insert(
                asset,
                CookManifestEntry {
                    asset,
                    kind: request.kind,
                    source,
                    artifact: relative_artifact,
                    source_hash: hex(&source_hash),
                    recipe_hash: hex(&recipe_hash),
                    artifact_hash: hex(&sha256(&artifact_bytes)),
                    dependencies,
                },
            );
        }
        let manifest = CookManifest {
            format_version: COOK_FORMAT_VERSION,
            cooker_version: self.config.cooker_version,
            assets: entries.into_values().collect(),
        };
        write_if_changed(
            &self.config.output_root.join("manifest.somnium-cook.json"),
            &manifest.to_json(),
        )?;
        Ok(CookReport { manifest, status })
    }
}

/// Submit the complete cook through the one engine job system with an explicit
/// scheduling class and deadline.
pub fn submit_cook(
    jobs: &mut JobSystem,
    config: CookConfig,
    requests: Vec<CookRequest>,
    priority: JobPriority,
    deadline: Instant,
) -> Result<JobHandle<CookReport>, JobError> {
    jobs.submit_with(
        JobDesc::new("asset.cook")
            .priority(priority)
            .deadline(deadline),
        move |context| {
            AssetCooker::new(config).cook_with_cancel(&requests, || {
                context
                    .check_cancelled()
                    .map_err(|_| "asset cook cancelled".into())
            })
        },
    )
}

/// Which representation the resolver reads. Consumers receive the same native
/// payload and `AssetId` from [`AssetResolver::load`] in both modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetLoadMode {
    Development,
    Build,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedNativeAsset {
    pub asset: AssetId,
    pub kind: CookKind,
    pub payload: Vec<u8>,
}

pub struct AssetResolver {
    source_root: PathBuf,
    cooked_root: PathBuf,
    manifest: CookManifest,
    mode: AssetLoadMode,
}

impl AssetResolver {
    #[must_use]
    pub fn new(
        source_root: PathBuf,
        cooked_root: PathBuf,
        manifest: CookManifest,
        mode: AssetLoadMode,
    ) -> Self {
        Self {
            source_root,
            cooked_root,
            manifest,
            mode,
        }
    }

    pub fn load(&self, asset: AssetId) -> Result<LoadedNativeAsset, String> {
        let entry = self
            .manifest
            .get(asset)
            .ok_or_else(|| format!("asset {asset} is absent from the cook manifest"))?;
        let payload = match self.mode {
            AssetLoadMode::Development => {
                let source = fs::read(self.source_root.join(&entry.source))
                    .map_err(|error| format!("read {}: {error}", entry.source))?;
                native_payload(entry.kind, &canonical_source(entry.kind, &source)?)
            }
            AssetLoadMode::Build => {
                let bytes = fs::read(self.cooked_root.join(&entry.artifact))
                    .map_err(|error| format!("read {}: {error}", entry.artifact))?;
                let cooked = CookedAsset::decode(&bytes)?;
                if cooked.asset != asset || cooked.kind != entry.kind {
                    return Err("manifest and cooked artifact disagree".into());
                }
                cooked.payload
            }
        };
        Ok(LoadedNativeAsset {
            asset,
            kind: entry.kind,
            payload,
        })
    }
}

fn canonical_source(kind: CookKind, bytes: &[u8]) -> Result<Vec<u8>, String> {
    if !kind.is_text() {
        return Ok(bytes.to_vec());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "text asset is not UTF-8")?;
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    if matches!(kind, CookKind::Prefab | CookKind::Material) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&normalized) {
            return serde_json::to_vec(&json).map_err(|error| error.to_string());
        }
    }
    Ok(normalized.into_bytes())
}

fn native_payload(kind: CookKind, canonical: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(20 + canonical.len());
    payload.extend_from_slice(&kind.payload_magic());
    payload.extend_from_slice(&NATIVE_VERSION.to_le_bytes());
    payload.extend_from_slice(&(canonical.len() as u64).to_le_bytes());
    payload.extend_from_slice(canonical);
    payload
}

fn validate_native_payload(kind: CookKind, payload: &[u8]) -> Result<(), String> {
    if payload.len() < 20 || payload[..8] != kind.payload_magic() {
        return Err("native payload kind magic mismatch".into());
    }
    let version = read_u32(payload, 8)?;
    if version != NATIVE_VERSION {
        return Err(format!("unsupported native payload version {version}"));
    }
    let length = read_u64(payload, 12)? as usize;
    if length != payload.len() - 20 {
        return Err("native payload length mismatch".into());
    }
    Ok(())
}

fn recipe_hash(
    cooker_version: u32,
    kind: CookKind,
    asset: AssetId,
    source_hash: [u8; 32],
    dependencies: &[(AssetId, [u8; 32])],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(COOK_FORMAT_VERSION.to_le_bytes());
    hasher.update(cooker_version.to_le_bytes());
    hasher.update([kind as u8]);
    hasher.update(asset.raw().to_le_bytes());
    hasher.update(source_hash);
    for (dependency, hash) in dependencies {
        hasher.update(dependency.raw().to_le_bytes());
        hasher.update(hash);
    }
    hasher.finalize().into()
}

fn safe_relative(text: &str) -> Result<String, String> {
    let path = Path::new(text);
    if text.trim().is_empty()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("source path must stay relative: {text}"));
    }
    Ok(text.replace('\\', "/").trim_start_matches("./").to_owned())
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if fs::read(path).ok().as_deref() == Some(bytes) {
        return Ok(());
    }
    write_atomic(path, bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "output has no parent".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("cook")
    ));
    {
        let mut file = fs::File::create(&temporary).map_err(|error| error.to_string())?;
        file.write_all(bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
    }
    if path.exists() {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn read_hash(bytes: &[u8], offset: usize) -> Result<[u8; 32], String> {
    bytes
        .get(offset..offset + 32)
        .ok_or_else(|| "truncated cooked hash".to_string())?
        .try_into()
        .map_err(|_| "invalid cooked hash".to_string())
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "truncated cooked u32".to_string())?
        .try_into()
        .map(u32::from_le_bytes)
        .map_err(|_| "invalid cooked u32".to_string())
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    bytes
        .get(offset..offset + 8)
        .ok_or_else(|| "truncated cooked u64".to_string())?
        .try_into()
        .map(u64::from_le_bytes)
        .map_err(|_| "invalid cooked u64".to_string())
}

fn read_u128(bytes: &[u8], offset: usize) -> Result<u128, String> {
    bytes
        .get(offset..offset + 16)
        .ok_or_else(|| "truncated cooked u128".to_string())?
        .try_into()
        .map(u128::from_le_bytes)
        .map_err(|_| "invalid cooked u128".to_string())
}

/// Default explicit deadline for a user-started offline cook.
#[must_use]
pub fn default_cook_deadline() -> Instant {
    Instant::now() + Duration::from_secs(60 * 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);

    fn fixture() -> (PathBuf, CookConfig, Vec<CookRequest>) {
        let root = std::env::temp_dir().join(format!(
            "somnium_native_cook_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("source");
        fs::create_dir_all(source.join("textures")).unwrap();
        fs::create_dir_all(source.join("materials")).unwrap();
        fs::create_dir_all(source.join("meshes")).unwrap();
        fs::write(source.join("textures/rock.png"), b"texture-a").unwrap();
        fs::write(
            source.join("materials/rock.sommat"),
            b"{\r\n\"roughness\":0.5\r\n}",
        )
        .unwrap();
        fs::write(source.join("meshes/ship.glb"), b"mesh-a").unwrap();
        let texture = CookRequest {
            source: "textures/rock.png".into(),
            kind: CookKind::Texture,
            dependencies: vec![],
        };
        let material = CookRequest {
            source: "materials/rock.sommat".into(),
            kind: CookKind::Material,
            dependencies: vec![texture.asset_id()],
        };
        let mesh = CookRequest {
            source: "meshes/ship.glb".into(),
            kind: CookKind::Mesh,
            dependencies: vec![],
        };
        let config = CookConfig {
            source_root: source,
            output_root: root.join("build"),
            cache_root: root.join("cache"),
            cooker_version: 4,
        };
        (root, config, vec![material, mesh, texture])
    }

    #[test]
    fn every_required_kind_has_a_distinct_native_family() {
        let kinds = [
            CookKind::Mesh,
            CookKind::Texture,
            CookKind::Audio,
            CookKind::Scene,
            CookKind::Prefab,
            CookKind::Shader,
        ];
        assert_eq!(
            kinds
                .iter()
                .map(|kind| kind.extension())
                .collect::<BTreeSet<_>>()
                .len(),
            kinds.len()
        );
        assert_eq!(
            kinds
                .iter()
                .map(|kind| kind.payload_magic())
                .collect::<BTreeSet<_>>()
                .len(),
            kinds.len()
        );
    }

    #[test]
    fn artifacts_round_trip_and_reject_corruption() {
        let payload = native_payload(CookKind::Shader, b"fn main() {}\n");
        let asset = CookedAsset {
            kind: CookKind::Shader,
            asset: AssetId::from_relative_path("shaders/main.wgsl"),
            cooker_version: 7,
            source_hash: sha256(b"source"),
            recipe_hash: sha256(b"recipe"),
            payload_hash: sha256(&payload),
            dependencies: vec![],
            payload,
        };
        let bytes = asset.encode();
        assert_eq!(CookedAsset::decode(&bytes).unwrap(), asset);
        for length in 0..bytes.len() {
            assert!(CookedAsset::decode(&bytes[..length]).is_err());
        }
        let mut corrupt = bytes;
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(CookedAsset::decode(&corrupt).is_err());
    }

    #[test]
    fn changed_dependency_recooks_only_its_reverse_closure() {
        let (root, config, requests) = fixture();
        let cooker = AssetCooker::new(config.clone());
        let first = cooker.cook(&requests).unwrap();
        assert!(first
            .status
            .values()
            .all(|status| *status == CookStatus::Cooked));
        let second = cooker.cook(&requests).unwrap();
        assert!(second
            .status
            .values()
            .all(|status| *status == CookStatus::Cached));

        fs::write(config.source_root.join("textures/rock.png"), b"texture-b").unwrap();
        let third = cooker.cook(&requests).unwrap();
        let texture = AssetId::from_relative_path("textures/rock.png");
        let material = AssetId::from_relative_path("materials/rock.sommat");
        let mesh = AssetId::from_relative_path("meshes/ship.glb");
        assert_eq!(third.status[&texture], CookStatus::Cooked);
        assert_eq!(third.status[&material], CookStatus::Cooked);
        assert_eq!(third.status[&mesh], CookStatus::Cached);
        assert_eq!(
            AssetDependencyGraph::from_requests(&requests)
                .unwrap()
                .affected_by(texture),
            BTreeSet::from([texture, material])
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn dependency_graph_rejects_missing_edges_and_cycles() {
        let a = CookRequest {
            source: "a.somscene".into(),
            kind: CookKind::Scene,
            dependencies: vec![AssetId::from_relative_path("missing.somprefab")],
        };
        assert!(AssetDependencyGraph::from_requests(&[a]).is_err());

        let mut a = CookRequest {
            source: "a.somscene".into(),
            kind: CookKind::Scene,
            dependencies: vec![],
        };
        let b = CookRequest {
            source: "b.somprefab".into(),
            kind: CookKind::Prefab,
            dependencies: vec![a.asset_id()],
        };
        a.dependencies.push(b.asset_id());
        assert!(AssetDependencyGraph::from_requests(&[a, b]).is_err());
    }

    #[test]
    fn request_order_and_line_endings_do_not_change_cooked_bytes() {
        let (root, config, mut requests) = fixture();
        let first = AssetCooker::new(config.clone()).cook(&requests).unwrap();
        let manifest = first.manifest.to_json();
        requests.reverse();
        fs::write(
            config.source_root.join("materials/rock.sommat"),
            b"{\n\"roughness\":0.5\n}",
        )
        .unwrap();
        let second = AssetCooker::new(config.clone()).cook(&requests).unwrap();
        assert_eq!(second.manifest.to_json(), manifest);
        assert!(second
            .status
            .values()
            .all(|status| *status == CookStatus::Cached));

        let relocated = CookConfig {
            source_root: config.source_root.clone(),
            output_root: root.join("relocated-build"),
            cache_root: root.join("relocated-cache"),
            cooker_version: config.cooker_version,
        };
        let third = AssetCooker::new(relocated.clone()).cook(&requests).unwrap();
        assert_eq!(third.manifest.to_json(), manifest);
        for entry in &third.manifest.assets {
            assert_eq!(
                fs::read(config.output_root.join(&entry.artifact)).unwrap(),
                fs::read(relocated.output_root.join(&entry.artifact)).unwrap()
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn development_and_build_resolve_the_same_id_and_native_payload() {
        let (root, config, requests) = fixture();
        let report = AssetCooker::new(config.clone()).cook(&requests).unwrap();
        let id = AssetId::from_relative_path("materials/rock.sommat");
        let development = AssetResolver::new(
            config.source_root.clone(),
            config.output_root.clone(),
            report.manifest.clone(),
            AssetLoadMode::Development,
        )
        .load(id)
        .unwrap();
        fs::remove_dir_all(&config.source_root).unwrap();
        let build = AssetResolver::new(
            config.source_root.clone(),
            config.output_root.clone(),
            report.manifest,
            AssetLoadMode::Build,
        )
        .load(id)
        .unwrap();
        assert_eq!(development, build);
        assert_eq!(development.asset, id);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cooker_version_is_part_of_the_cache_key() {
        let (root, config, requests) = fixture();
        AssetCooker::new(config.clone()).cook(&requests).unwrap();
        let mut changed = config;
        changed.cooker_version += 1;
        let report = AssetCooker::new(changed).cook(&requests).unwrap();
        assert!(report
            .status
            .values()
            .all(|status| *status == CookStatus::Cooked));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_cook_is_a_named_priority_deadline_job() {
        let (root, config, requests) = fixture();
        let mut jobs = JobSystem::single_threaded();
        let handle = submit_cook(
            &mut jobs,
            config,
            requests,
            JobPriority::User,
            default_cook_deadline(),
        )
        .unwrap();
        assert_eq!(handle.snapshot().priority, JobPriority::User);
        assert!(handle.try_take().unwrap().is_ok());
        let _ = fs::remove_dir_all(root);
    }
}
