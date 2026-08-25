//! Budgeted cooked-asset residency and hot reload (MORROWIND-R).
//!
//! Loads run through `somnium_jobs`; completed bytes enter a second, byte-
//! budgeted installation queue. Handles expose a placeholder immediately and
//! publish a complete replacement under one write lock, so readers can never
//! observe a half-installed asset. The diagnostics snapshot is deliberately UI
//! neutral: an editor panel can render it without owning residency policy.

use crate::{
    cook::{AssetResolver, CookKind, CookedAsset},
    database::AssetId,
};
use somnium_jobs::{JobContext, JobDesc, JobError, JobPriority, JobSystem};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, RwLock},
    time::{Instant, SystemTime},
};

/// A separately resident unit. Mesh LODs deliberately have distinct keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResidencyKey {
    pub asset: AssetId,
    pub lod: u8,
}

/// Observable residency state from Seam 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Residency {
    Absent,
    Requested { since_frame: u64 },
    Partial { lod: u8 },
    Resident,
    Evicting,
}

/// Immutable value published into an [`AssetHandle`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResidentAsset {
    pub asset: AssetId,
    pub kind: CookKind,
    pub lod: u8,
    pub revision: u64,
    pub payload: Arc<[u8]>,
    pub placeholder: bool,
}

impl ResidentAsset {
    fn placeholder(kind: CookKind, lod: u8) -> Self {
        Self {
            asset: AssetId::NONE,
            kind,
            lod,
            revision: 0,
            payload: Arc::from([]),
            placeholder: true,
        }
    }
}

/// Stable handle whose value changes atomically from placeholder to resident.
#[derive(Clone)]
pub struct AssetHandle {
    slot: Arc<RwLock<Arc<ResidentAsset>>>,
}

impl AssetHandle {
    /// Take a stable snapshot. It remains valid across reload and eviction.
    #[must_use]
    pub fn current(&self) -> Arc<ResidentAsset> {
        read_lock(&self.slot).clone()
    }

    #[must_use]
    pub fn is_placeholder(&self) -> bool {
        self.current().placeholder
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResidencyConfig {
    pub byte_budget: usize,
    pub upload_budget_per_frame: usize,
}

#[derive(Clone, Debug)]
pub struct AssetRequest {
    pub asset: AssetId,
    pub kind: CookKind,
    pub lod: u8,
    pub requester: String,
    pub priority: JobPriority,
    pub deadline: Option<Instant>,
}

impl AssetRequest {
    #[must_use]
    pub fn new(asset: AssetId, kind: CookKind, requester: impl Into<String>) -> Self {
        Self {
            asset,
            kind,
            lod: 0,
            requester: requester.into(),
            priority: JobPriority::Normal,
            deadline: None,
        }
    }

    #[must_use]
    pub const fn lod(mut self, lod: u8) -> Self {
        self.lod = lod;
        self
    }

    #[must_use]
    pub const fn priority(mut self, priority: JobPriority) -> Self {
        self.priority = priority;
        self
    }

    #[must_use]
    pub const fn deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResidencyError {
    InvalidAsset,
    EmptyRequester,
    LodOnlySupportedForMeshes,
    KindMismatch,
    Job(String),
}

impl From<JobError> for ResidencyError {
    fn from(value: JobError) -> Self {
        Self::Job(format!("{value:?}"))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadStats {
    pub uploaded_bytes: usize,
    pub installed: usize,
    pub evicted: usize,
    pub still_pending: usize,
}

/// One diagnostics-panel row, including the four required answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResidencyRow {
    pub asset: AssetId,
    pub kind: CookKind,
    pub lod: u8,
    pub state: Residency,
    pub loaded: bool,
    pub size_bytes: usize,
    pub why: String,
    pub requesters: Vec<String>,
    pub last_used_frame: u64,
    pub revision: u64,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResidencySnapshot {
    pub frame: u64,
    pub byte_budget: usize,
    pub resident_bytes: usize,
    pub upload_budget_per_frame: usize,
    pub queued_upload_bytes: usize,
    pub rows: Vec<ResidencyRow>,
}

struct Entry {
    kind: CookKind,
    state: Residency,
    slot: Arc<RwLock<Arc<ResidentAsset>>>,
    requesters: BTreeSet<String>,
    last_used_frame: u64,
    size_bytes: usize,
    revision: u64,
    ticket: u64,
    loading: bool,
    last_error: Option<String>,
}

struct PreparedUpload {
    key: ResidencyKey,
    kind: CookKind,
    ticket: u64,
    payload: Vec<u8>,
    consumed: usize,
}

#[derive(Default)]
struct Shared {
    entries: BTreeMap<ResidencyKey, Entry>,
    pending: VecDeque<PreparedUpload>,
    resident_bytes: usize,
    frame: u64,
    next_ticket: u64,
}

/// Runtime residency owner. Cloneable handles, not this manager, escape to
/// render/audio consumers; policy stays centralized here.
pub struct ResidencyManager {
    config: ResidencyConfig,
    shared: Arc<Mutex<Shared>>,
}

impl ResidencyManager {
    #[must_use]
    pub fn new(config: ResidencyConfig) -> Self {
        Self {
            config,
            shared: Arc::new(Mutex::new(Shared::default())),
        }
    }

    /// Request through the source/cooked resolver. The returned handle already
    /// contains a type-correct placeholder; no I/O occurs on this thread.
    pub fn request_resolved(
        &self,
        jobs: &mut JobSystem,
        resolver: Arc<AssetResolver>,
        request: AssetRequest,
    ) -> Result<AssetHandle, ResidencyError> {
        let asset = request.asset;
        let kind = request.kind;
        self.request_with_loader(jobs, request, move |context| {
            context
                .check_cancelled()
                .map_err(|error| format!("{error:?}"))?;
            let loaded = resolver.load(asset)?;
            if loaded.kind != kind {
                return Err("resolver returned the wrong cooked kind".into());
            }
            Ok(loaded.payload)
        })
    }

    /// Request with a caller-provided worker loader. This is the integration
    /// point for mesh LOD chunk readers and platform-specific decoders.
    pub fn request_with_loader<F>(
        &self,
        jobs: &mut JobSystem,
        request: AssetRequest,
        loader: F,
    ) -> Result<AssetHandle, ResidencyError>
    where
        F: FnOnce(JobContext) -> Result<Vec<u8>, String> + Send + 'static,
    {
        self.schedule(jobs, request, false, loader)
    }

    fn schedule<F>(
        &self,
        jobs: &mut JobSystem,
        request: AssetRequest,
        force: bool,
        loader: F,
    ) -> Result<AssetHandle, ResidencyError>
    where
        F: FnOnce(JobContext) -> Result<Vec<u8>, String> + Send + 'static,
    {
        validate_request(&request)?;
        let key = ResidencyKey {
            asset: request.asset,
            lod: request.lod,
        };
        let (handle, ticket, should_submit) = {
            let mut shared = lock(&self.shared);
            let frame = shared.frame;
            let entry = shared.entries.entry(key).or_insert_with(|| Entry {
                kind: request.kind,
                state: Residency::Absent,
                slot: Arc::new(RwLock::new(Arc::new(ResidentAsset::placeholder(
                    request.kind,
                    request.lod,
                )))),
                requesters: BTreeSet::new(),
                last_used_frame: frame,
                size_bytes: 0,
                revision: 0,
                ticket: 0,
                loading: false,
                last_error: None,
            });
            if entry.kind != request.kind {
                return Err(ResidencyError::KindMismatch);
            }
            entry.requesters.insert(request.requester.clone());
            entry.last_used_frame = frame;
            let handle = AssetHandle {
                slot: Arc::clone(&entry.slot),
            };
            if !force && (entry.loading || entry.size_bytes > 0) {
                (handle, entry.ticket, false)
            } else {
                shared.next_ticket = shared.next_ticket.wrapping_add(1).max(1);
                let ticket = shared.next_ticket;
                let entry = shared.entries.get_mut(&key).expect("entry was inserted");
                entry.ticket = ticket;
                entry.loading = true;
                entry.last_error = None;
                if entry.size_bytes == 0 {
                    entry.state = Residency::Requested { since_frame: frame };
                }
                (handle, ticket, true)
            }
        };
        if !should_submit {
            return Ok(handle);
        }

        let mut desc = JobDesc::new("asset.residency.load").priority(request.priority);
        if let Some(deadline) = request.deadline {
            desc = desc.deadline(deadline);
        }
        let shared = Arc::clone(&self.shared);
        let submit = jobs.submit_applied(desc, loader, move |result| {
            let mut shared = lock(&shared);
            let current = shared.entries.get(&key).map(|entry| entry.ticket);
            if current != Some(ticket) {
                return;
            }
            match result {
                Ok(payload) => shared.pending.push_back(PreparedUpload {
                    key,
                    kind: request.kind,
                    ticket,
                    payload,
                    consumed: 0,
                }),
                Err(error) => {
                    let entry = shared.entries.get_mut(&key).expect("ticket owns entry");
                    entry.loading = false;
                    entry.last_error = Some(format!("{error:?}"));
                    if entry.size_bytes == 0 {
                        entry.state = Residency::Absent;
                    }
                }
            }
        });
        if let Err(error) = submit {
            let mut shared = lock(&self.shared);
            if let Some(entry) = shared.entries.get_mut(&key) {
                if entry.ticket == ticket {
                    entry.loading = false;
                    entry.last_error = Some(format!("{error:?}"));
                    if entry.size_bytes == 0 {
                        entry.state = Residency::Absent;
                    }
                }
            }
            return Err(error.into());
        }
        Ok(handle)
    }

    /// Spend at most the configured upload bytes and install only complete
    /// values. Oversized uploads therefore span frames instead of bypassing the
    /// budget or blocking forever.
    pub fn process_frame(&self) -> UploadStats {
        let mut shared = lock(&self.shared);
        shared.frame = shared.frame.wrapping_add(1);
        let mut stats = UploadStats::default();
        let mut budget = self.config.upload_budget_per_frame;
        loop {
            let Some(front) = shared.pending.front_mut() else {
                break;
            };
            let remaining = front.payload.len().saturating_sub(front.consumed);
            let spent = remaining.min(budget);
            front.consumed += spent;
            budget -= spent;
            stats.uploaded_bytes += spent;
            if front.consumed != front.payload.len() {
                break;
            }
            let completed = shared.pending.pop_front().expect("front existed");
            if install_completed(&mut shared, self.config.byte_budget, completed, &mut stats) {
                stats.installed += 1;
            }
            if budget == 0 {
                break;
            }
        }
        stats.still_pending = shared.pending.len();
        stats
    }

    /// Mark use for deterministic least-recently-used eviction.
    pub fn touch(&self, asset: AssetId, lod: u8) -> bool {
        let mut shared = lock(&self.shared);
        let frame = shared.frame;
        let Some(entry) = shared.entries.get_mut(&ResidencyKey { asset, lod }) else {
            return false;
        };
        entry.last_used_frame = frame;
        true
    }

    pub fn release(&self, asset: AssetId, lod: u8, requester: &str) -> bool {
        lock(&self.shared)
            .entries
            .get_mut(&ResidencyKey { asset, lod })
            .is_some_and(|entry| entry.requesters.remove(requester))
    }

    #[must_use]
    pub fn snapshot(&self) -> ResidencySnapshot {
        let shared = lock(&self.shared);
        let queued_upload_bytes = shared
            .pending
            .iter()
            .map(|pending| pending.payload.len().saturating_sub(pending.consumed))
            .sum();
        let rows = shared
            .entries
            .iter()
            .map(|(key, entry)| {
                let requesters: Vec<_> = entry.requesters.iter().cloned().collect();
                let why = if requesters.is_empty() {
                    entry
                        .last_error
                        .as_ref()
                        .map_or_else(|| "unreferenced cache entry".into(), |error| error.clone())
                } else {
                    format!("requested by {}", requesters.join(", "))
                };
                ResidencyRow {
                    asset: key.asset,
                    kind: entry.kind,
                    lod: key.lod,
                    state: entry.state,
                    loaded: entry.size_bytes > 0,
                    size_bytes: entry.size_bytes,
                    why,
                    requesters,
                    last_used_frame: entry.last_used_frame,
                    revision: entry.revision,
                    last_error: entry.last_error.clone(),
                }
            })
            .collect();
        ResidencySnapshot {
            frame: shared.frame,
            byte_budget: self.config.byte_budget,
            resident_bytes: shared.resident_bytes,
            upload_budget_per_frame: self.config.upload_budget_per_frame,
            queued_upload_bytes,
            rows,
        }
    }

    /// Reload every currently tracked LOD for a changed cooked artifact. The
    /// old value remains published until the replacement validates and fully
    /// consumes its upload budget.
    pub fn hot_reload(
        &self,
        jobs: &mut JobSystem,
        change: &CookedChange,
    ) -> Result<usize, ResidencyError> {
        let lods: Vec<u8> = {
            let shared = lock(&self.shared);
            shared
                .entries
                .iter()
                .filter(|(key, entry)| key.asset == change.asset && entry.kind == change.kind)
                .map(|(key, _)| key.lod)
                .collect()
        };
        for lod in &lods {
            let path = change.path.clone();
            let asset = change.asset;
            let kind = change.kind;
            self.schedule(
                jobs,
                AssetRequest::new(asset, kind, "hot reload")
                    .lod(*lod)
                    .priority(JobPriority::Visible),
                true,
                move |context| load_cooked_change(&context, &path, asset, kind),
            )?;
        }
        Ok(lods.len())
    }
}

fn validate_request(request: &AssetRequest) -> Result<(), ResidencyError> {
    if request.asset == AssetId::NONE {
        return Err(ResidencyError::InvalidAsset);
    }
    if request.requester.trim().is_empty() {
        return Err(ResidencyError::EmptyRequester);
    }
    if request.lod != 0 && request.kind != CookKind::Mesh {
        return Err(ResidencyError::LodOnlySupportedForMeshes);
    }
    Ok(())
}

fn install_completed(
    shared: &mut Shared,
    byte_budget: usize,
    completed: PreparedUpload,
    stats: &mut UploadStats,
) -> bool {
    let current_ticket = shared.entries.get(&completed.key).map(|entry| entry.ticket);
    if current_ticket != Some(completed.ticket) {
        return false;
    }
    let old_size = shared.entries[&completed.key].size_bytes;
    let new_size = completed.payload.len();
    if new_size > byte_budget {
        let entry = shared
            .entries
            .get_mut(&completed.key)
            .expect("ticket owns entry");
        entry.loading = false;
        entry.last_error = Some(format!(
            "asset needs {new_size} bytes but the residency budget is {byte_budget}"
        ));
        if old_size == 0 {
            entry.state = Residency::Absent;
        }
        return false;
    }

    while shared
        .resident_bytes
        .saturating_sub(old_size)
        .saturating_add(new_size)
        > byte_budget
    {
        let victim = shared
            .entries
            .iter()
            .filter(|(key, entry)| **key != completed.key && entry.size_bytes > 0)
            .min_by_key(|(key, entry)| (entry.last_used_frame, **key))
            .map(|(key, _)| *key);
        let Some(victim) = victim else {
            break;
        };
        evict(shared, victim);
        stats.evicted += 1;
    }

    let entry = shared
        .entries
        .get_mut(&completed.key)
        .expect("ticket owns entry");
    let replacement = Arc::new(ResidentAsset {
        asset: completed.key.asset,
        kind: completed.kind,
        lod: completed.key.lod,
        revision: entry.revision.wrapping_add(1),
        payload: Arc::from(completed.payload),
        placeholder: false,
    });
    *write_lock(&entry.slot) = replacement;
    entry.revision = entry.revision.wrapping_add(1);
    entry.size_bytes = new_size;
    entry.loading = false;
    entry.last_error = None;
    entry.state = if entry.kind == CookKind::Mesh && completed.key.lod > 0 {
        Residency::Partial {
            lod: completed.key.lod,
        }
    } else {
        Residency::Resident
    };
    shared.resident_bytes = shared
        .resident_bytes
        .saturating_sub(old_size)
        .saturating_add(new_size);
    true
}

fn evict(shared: &mut Shared, key: ResidencyKey) {
    let Some(entry) = shared.entries.get_mut(&key) else {
        return;
    };
    entry.state = Residency::Evicting;
    shared.resident_bytes = shared.resident_bytes.saturating_sub(entry.size_bytes);
    entry.size_bytes = 0;
    *write_lock(&entry.slot) = Arc::new(ResidentAsset::placeholder(entry.kind, key.lod));
    entry.state = Residency::Absent;
}

fn load_cooked_change(
    context: &JobContext,
    path: &Path,
    expected_asset: AssetId,
    expected_kind: CookKind,
) -> Result<Vec<u8>, String> {
    context
        .check_cancelled()
        .map_err(|error| format!("{error:?}"))?;
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let cooked = CookedAsset::decode(&bytes)?;
    if cooked.asset != expected_asset || cooked.kind != expected_kind {
        return Err("hot-reload artifact identity or kind changed".into());
    }
    Ok(cooked.payload)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CookedChange {
    pub asset: AssetId,
    pub kind: CookKind,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    modified: SystemTime,
    len: u64,
}

struct WatchedCooked {
    kind: CookKind,
    path: PathBuf,
    stamp: Option<FileStamp>,
}

/// Polling watcher matching the shader watcher precedent. All [`CookKind`]
/// values use the same path; no asset family has a bespoke reload mechanism.
#[derive(Default)]
pub struct CookedAssetWatcher {
    watched: BTreeMap<AssetId, WatchedCooked>,
}

impl CookedAssetWatcher {
    pub fn watch(&mut self, asset: AssetId, kind: CookKind, path: PathBuf) {
        let stamp = file_stamp(&path);
        self.watched
            .insert(asset, WatchedCooked { kind, path, stamp });
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.watched.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.watched.is_empty()
    }

    /// Report stable, readable changes once. Missing/transiently unreadable
    /// files retain their old stamp and are retried, just like shader sources.
    pub fn poll(&mut self) -> Vec<CookedChange> {
        let mut changes = Vec::new();
        for (&asset, watched) in &mut self.watched {
            let Some(stamp) = file_stamp(&watched.path) else {
                continue;
            };
            if Some(stamp) == watched.stamp || std::fs::File::open(&watched.path).is_err() {
                continue;
            }
            watched.stamp = Some(stamp);
            changes.push(CookedChange {
                asset,
                kind: watched.kind,
                path: watched.path.clone(),
            });
        }
        changes
    }
}

fn file_stamp(path: &Path) -> Option<FileStamp> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(FileStamp {
        modified: metadata.modified().ok()?,
        len: metadata.len(),
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn read_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn write_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cook::LoadedNativeAsset;
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    fn id(name: &str) -> AssetId {
        AssetId::from_relative_path(name)
    }

    fn complete_jobs(jobs: &mut JobSystem) {
        let stats = jobs.drain_completions(Duration::from_secs(1));
        assert_eq!(stats.still_pending, 0);
    }

    #[test]
    fn request_returns_placeholder_then_installs_atomically_within_upload_budget() {
        let manager = ResidencyManager::new(ResidencyConfig {
            byte_budget: 100,
            upload_budget_per_frame: 4,
        });
        let mut jobs = JobSystem::single_threaded();
        let handle = manager
            .request_with_loader(
                &mut jobs,
                AssetRequest::new(id("mesh/a"), CookKind::Mesh, "viewport").lod(2),
                |_| Ok(vec![7; 10]),
            )
            .unwrap();
        assert!(handle.is_placeholder());
        complete_jobs(&mut jobs);
        assert_eq!(manager.process_frame().uploaded_bytes, 4);
        assert!(
            handle.is_placeholder(),
            "partial uploads are never published"
        );
        assert_eq!(manager.process_frame().uploaded_bytes, 4);
        assert!(handle.is_placeholder());
        let final_frame = manager.process_frame();
        assert_eq!(final_frame.uploaded_bytes, 2);
        assert_eq!(final_frame.installed, 1);
        let current = handle.current();
        assert!(!current.placeholder);
        assert_eq!(&*current.payload, &[7; 10]);
        assert_eq!(
            manager.snapshot().rows[0].state,
            Residency::Partial { lod: 2 }
        );
    }

    #[test]
    fn byte_budget_evicts_least_recently_used_lod_and_swaps_its_placeholder() {
        let manager = ResidencyManager::new(ResidencyConfig {
            byte_budget: 10,
            upload_budget_per_frame: 20,
        });
        let mut jobs = JobSystem::single_threaded();
        let a = manager
            .request_with_loader(
                &mut jobs,
                AssetRequest::new(id("a"), CookKind::Mesh, "camera").lod(2),
                |_| Ok(vec![1; 6]),
            )
            .unwrap();
        complete_jobs(&mut jobs);
        manager.process_frame();
        let b = manager
            .request_with_loader(
                &mut jobs,
                AssetRequest::new(id("b"), CookKind::Mesh, "camera").lod(1),
                |_| Ok(vec![2; 6]),
            )
            .unwrap();
        complete_jobs(&mut jobs);
        let stats = manager.process_frame();
        assert_eq!(stats.evicted, 1);
        assert!(a.is_placeholder());
        assert!(!b.is_placeholder());
        assert_eq!(manager.snapshot().resident_bytes, 6);
    }

    #[test]
    fn old_reload_completion_cannot_replace_a_newer_ticket() {
        let manager = ResidencyManager::new(ResidencyConfig {
            byte_budget: 100,
            upload_budget_per_frame: 100,
        });
        let mut jobs = JobSystem::single_threaded();
        let request = AssetRequest::new(id("race"), CookKind::Texture, "viewport");
        let handle = manager
            .request_with_loader(&mut jobs, request.clone(), |_| Ok(vec![1]))
            .unwrap();
        // A forced hot reload is represented by scheduling a newer ticket via
        // a watched artifact in production; this private path isolates ordering.
        manager
            .schedule(&mut jobs, request, true, |_| Ok(vec![2]))
            .unwrap();
        complete_jobs(&mut jobs);
        manager.process_frame();
        assert_eq!(&*handle.current().payload, &[2]);
    }

    #[test]
    fn diagnostics_answer_loaded_why_size_and_requester() {
        let manager = ResidencyManager::new(ResidencyConfig {
            byte_budget: 64,
            upload_budget_per_frame: 64,
        });
        let mut jobs = JobSystem::single_threaded();
        manager
            .request_with_loader(
                &mut jobs,
                AssetRequest::new(id("audio"), CookKind::Audio, "ambient emitter"),
                |_| Ok(vec![3; 12]),
            )
            .unwrap();
        complete_jobs(&mut jobs);
        manager.process_frame();
        let row = manager.snapshot().rows.remove(0);
        assert!(row.loaded);
        assert_eq!(row.size_bytes, 12);
        assert_eq!(row.requesters, ["ambient emitter"]);
        assert_eq!(row.why, "requested by ambient emitter");
    }

    #[test]
    fn non_mesh_lods_are_rejected() {
        let manager = ResidencyManager::new(ResidencyConfig {
            byte_budget: 1,
            upload_budget_per_frame: 1,
        });
        let mut jobs = JobSystem::single_threaded();
        let result = manager.request_with_loader(
            &mut jobs,
            AssetRequest::new(id("texture"), CookKind::Texture, "test").lod(1),
            |_| Ok(Vec::new()),
        );
        assert!(matches!(
            result,
            Err(ResidencyError::LodOnlySupportedForMeshes)
        ));
    }

    fn temp_dir(label: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "somnium_residency_{label}_{}_{}",
            std::process::id(),
            serial
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn watcher_reports_hot_reload_for_every_cooked_kind_once() {
        let kinds = [
            CookKind::Mesh,
            CookKind::Texture,
            CookKind::Audio,
            CookKind::Scene,
            CookKind::Prefab,
            CookKind::Shader,
            CookKind::Material,
        ];
        let dir = temp_dir("kinds");
        let mut watcher = CookedAssetWatcher::default();
        for (index, kind) in kinds.into_iter().enumerate() {
            let path = dir.join(format!("{index}.{}", kind.extension()));
            fs::write(&path, [0]).unwrap();
            watcher.watch(id(&format!("asset/{index}")), kind, path);
        }
        assert!(watcher.poll().is_empty());
        for (index, kind) in kinds.iter().enumerate() {
            let path = dir.join(format!("{index}.{}", kind.extension()));
            fs::write(path, [0, 1]).unwrap();
        }
        let changes = watcher.poll();
        assert_eq!(changes.len(), kinds.len());
        assert_eq!(
            changes
                .iter()
                .map(|change| change.kind)
                .collect::<BTreeSet<_>>(),
            kinds.into_iter().collect()
        );
        assert!(watcher.poll().is_empty());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn failed_reload_keeps_the_old_resident_value() {
        let manager = ResidencyManager::new(ResidencyConfig {
            byte_budget: 100,
            upload_budget_per_frame: 100,
        });
        let mut jobs = JobSystem::single_threaded();
        let asset = id("kept");
        let handle = manager
            .request_with_loader(
                &mut jobs,
                AssetRequest::new(asset, CookKind::Shader, "material"),
                |_| Ok(vec![9]),
            )
            .unwrap();
        complete_jobs(&mut jobs);
        manager.process_frame();
        let dir = temp_dir("bad_reload");
        let path = dir.join("bad.somshader");
        fs::write(&path, b"invalid").unwrap();
        let change = CookedChange {
            asset,
            kind: CookKind::Shader,
            path,
        };
        assert_eq!(manager.hot_reload(&mut jobs, &change).unwrap(), 1);
        complete_jobs(&mut jobs);
        assert_eq!(&*handle.current().payload, &[9]);
        assert!(manager.snapshot().rows[0].last_error.is_some());
        fs::remove_dir_all(dir).ok();
    }

    fn encoded_shader(asset: AssetId, source: &[u8]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(b"SOMSHDR\0");
        payload.extend_from_slice(&1_u32.to_le_bytes());
        payload.extend_from_slice(&(source.len() as u64).to_le_bytes());
        payload.extend_from_slice(source);
        let payload_hash: [u8; 32] = Sha256::digest(&payload).into();
        CookedAsset {
            kind: CookKind::Shader,
            asset,
            cooker_version: 1,
            source_hash: [0; 32],
            recipe_hash: [0; 32],
            payload_hash,
            dependencies: Vec::new(),
            payload,
        }
        .encode()
    }

    #[test]
    fn successful_hot_reload_keeps_old_value_until_atomic_replacement() {
        let manager = ResidencyManager::new(ResidencyConfig {
            byte_budget: 100,
            upload_budget_per_frame: 8,
        });
        let mut jobs = JobSystem::single_threaded();
        let asset = id("reloaded");
        let handle = manager
            .request_with_loader(
                &mut jobs,
                AssetRequest::new(asset, CookKind::Shader, "material"),
                |_| Ok(vec![1]),
            )
            .unwrap();
        complete_jobs(&mut jobs);
        manager.process_frame();

        let dir = temp_dir("good_reload");
        let path = dir.join("good.somshader");
        fs::write(&path, encoded_shader(asset, b"new shader")).unwrap();
        let change = CookedChange {
            asset,
            kind: CookKind::Shader,
            path,
        };
        manager.hot_reload(&mut jobs, &change).unwrap();
        complete_jobs(&mut jobs);
        assert_eq!(&*handle.current().payload, &[1]);
        while manager.process_frame().still_pending != 0 {}
        let replacement = handle.current();
        assert!(!replacement.placeholder);
        assert_eq!(replacement.revision, 2);
        assert!(replacement.payload.ends_with(b"new shader"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn over_budget_single_asset_never_bypasses_the_limit() {
        let manager = ResidencyManager::new(ResidencyConfig {
            byte_budget: 4,
            upload_budget_per_frame: 100,
        });
        let mut jobs = JobSystem::single_threaded();
        let handle = manager
            .request_with_loader(
                &mut jobs,
                AssetRequest::new(id("huge"), CookKind::Audio, "music"),
                |_| Ok(vec![0; 5]),
            )
            .unwrap();
        complete_jobs(&mut jobs);
        assert_eq!(manager.process_frame().installed, 0);
        assert!(handle.is_placeholder());
        assert_eq!(manager.snapshot().resident_bytes, 0);
    }

    #[test]
    fn loaded_native_asset_shape_stays_compatible_with_resolver_output() {
        let value = LoadedNativeAsset {
            asset: id("shape"),
            kind: CookKind::Scene,
            payload: vec![1, 2, 3],
        };
        assert_eq!(value.payload.len(), 3);
    }
}
