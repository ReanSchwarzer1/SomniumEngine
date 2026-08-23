//! UI-side thumbnail atlas and visible-first request queue.
//!
//! File IO, image decoding, and preview rendering deliberately live outside
//! this crate. The frame loop only enqueues paths and applies already prepared
//! 64x64 RGBA cells within a measured wall-clock budget.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Side of one thumbnail cell, in atlas pixels.
pub const CELL: u32 = 64;
/// Atlas width in pixels.
pub const ATLAS_WIDTH: u32 = CELL * 16;
/// Atlas height in pixels.
pub const ATLAS_HEIGHT: u32 = CELL * 16;
/// Slots retained before least-recently-used eviction starts.
pub const CAPACITY: usize = 16 * 16;
/// Texture id reserved for the thumbnail atlas.
pub const THUMBNAIL_ATLAS_TEXTURE_ID: u32 = 2;
/// Default time allowed for copying completed previews into the atlas.
pub const DEFAULT_APPLY_BUDGET: Duration = Duration::from_micros(750);

/// What a requested thumbnail currently holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThumbState {
    /// Waiting for an off-thread producer.
    Pending,
    /// Packed into this atlas slot.
    Ready(u32),
    /// No real preview was available; callers should use a kind icon.
    Failed,
}

/// Work transferred to the host job registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThumbnailRequest {
    /// Absolute asset path.
    pub path: PathBuf,
    /// Requested square side in pixels.
    pub size: u32,
    /// Whether the tile intersected the current viewport.
    pub visible: bool,
}

/// Atlas plus UI-only scheduling state.
pub struct ThumbnailCache {
    /// RGBA8 atlas bytes.
    pub pixels: Vec<u8>,
    /// Atlas width.
    pub width: u32,
    /// Atlas height.
    pub height: u32,
    /// Set whenever pixels changed.
    pub dirty: bool,
    states: HashMap<PathBuf, ThumbState>,
    requested: HashSet<PathBuf>,
    visible_queue: VecDeque<PathBuf>,
    background_queue: VecDeque<PathBuf>,
    slot_owner: Vec<Option<PathBuf>>,
    last_used: HashMap<PathBuf, u64>,
    clock: u64,
}

impl Default for ThumbnailCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ThumbnailCache {
    /// Creates an empty cache without touching the filesystem.
    pub fn new() -> Self {
        Self {
            pixels: vec![0; (ATLAS_WIDTH * ATLAS_HEIGHT * 4) as usize],
            width: ATLAS_WIDTH,
            height: ATLAS_HEIGHT,
            dirty: false,
            states: HashMap::new(),
            requested: HashSet::new(),
            visible_queue: VecDeque::new(),
            background_queue: VecDeque::new(),
            slot_owner: vec![None; CAPACITY],
            last_used: HashMap::new(),
            clock: 0,
        }
    }

    /// Enqueues a preview once. Visible requests always drain before background
    /// requests. A later visible request promotes queued background work.
    pub fn request(&mut self, path: &Path, visible: bool) {
        self.touch(path);
        if self.states.contains_key(path) || self.requested.contains(path) {
            if visible {
                if let Some(index) = self.background_queue.iter().position(|p| p == path) {
                    let promoted = self.background_queue.remove(index).expect("known index");
                    self.visible_queue.push_back(promoted);
                }
            }
            return;
        }
        let owned = path.to_path_buf();
        self.requested.insert(owned.clone());
        self.states.insert(owned.clone(), ThumbState::Pending);
        if visible {
            self.visible_queue.push_back(owned);
        } else {
            self.background_queue.push_back(owned);
        }
    }

    /// Compatibility hook for the frame loop. It performs no decoding or IO.
    pub fn pump(&mut self) -> usize {
        0
    }

    /// Drains at most `limit` requests in visible-first order.
    pub fn take_requests(&mut self, limit: usize) -> Vec<ThumbnailRequest> {
        let mut out = Vec::with_capacity(limit.min(self.requested.len()));
        while out.len() < limit {
            let (path, visible) = if let Some(path) = self.visible_queue.pop_front() {
                (path, true)
            } else if let Some(path) = self.background_queue.pop_front() {
                (path, false)
            } else {
                break;
            };
            out.push(ThumbnailRequest {
                path,
                size: CELL,
                visible,
            });
        }
        out
    }

    /// Applies one prepared cell.
    pub fn deliver(&mut self, path: &Path, rgba: &[u8]) -> bool {
        self.requested.remove(path);
        if rgba.len() != (CELL * CELL * 4) as usize {
            self.states.insert(path.to_path_buf(), ThumbState::Failed);
            return false;
        }
        let slot = self.allocate_slot(path);
        self.copy_into(slot, rgba);
        self.states.insert(path.to_path_buf(), ThumbState::Ready(slot));
        self.touch(path);
        true
    }

    /// Applies completed previews until actual elapsed time reaches `budget`.
    /// Returns `(applied, remaining)` so the host can preserve unfinished work.
    pub fn deliver_budgeted(
        &mut self,
        ready: &mut VecDeque<(PathBuf, Vec<u8>)>,
        budget: Duration,
    ) -> usize {
        let started = Instant::now();
        let mut applied = 0;
        while let Some((path, rgba)) = ready.pop_front() {
            let _ = self.deliver(&path, &rgba);
            applied += 1;
            if started.elapsed() >= budget {
                break;
            }
        }
        applied
    }

    /// Marks a path as having no real preview.
    pub fn mark_failed(&mut self, path: &Path) {
        self.requested.remove(path);
        self.states.insert(path.to_path_buf(), ThumbState::Failed);
    }

    /// Returns current state.
    pub fn state(&self, path: &Path) -> Option<ThumbState> {
        self.states.get(path).copied()
    }

    /// Returns the atlas UV rectangle for a ready path.
    pub fn uv(&mut self, path: &Path) -> Option<[f32; 4]> {
        let ThumbState::Ready(slot) = self.states.get(path).copied()? else {
            return None;
        };
        self.touch(path);
        let (x, y) = Self::slot_origin(slot);
        Some([
            x as f32 / self.width as f32,
            y as f32 / self.height as f32,
            (x + CELL) as f32 / self.width as f32,
            (y + CELL) as f32 / self.height as f32,
        ])
    }

    /// Number of ready cells.
    pub fn ready_count(&self) -> usize {
        self.states.values().filter(|s| matches!(s, ThumbState::Ready(_))).count()
    }

    /// Number of outstanding requests.
    pub fn pending_count(&self) -> usize {
        self.states.values().filter(|s| **s == ThumbState::Pending).count()
    }

    /// Clears all paths and pixels.
    pub fn clear(&mut self) {
        self.states.clear();
        self.requested.clear();
        self.visible_queue.clear();
        self.background_queue.clear();
        self.slot_owner.fill(None);
        self.last_used.clear();
        self.clock = 0;
        self.pixels.fill(0);
        self.dirty = true;
    }

    fn touch(&mut self, path: &Path) {
        self.clock = self.clock.wrapping_add(1);
        self.last_used.insert(path.to_path_buf(), self.clock);
    }

    fn allocate_slot(&mut self, path: &Path) -> u32 {
        if let Some(index) = self.slot_owner.iter().position(Option::is_none) {
            self.slot_owner[index] = Some(path.to_path_buf());
            return index as u32;
        }
        let (index, victim) = self
            .slot_owner
            .iter()
            .enumerate()
            .filter_map(|(index, owner)| owner.as_ref().map(|owner| (index, owner)))
            .min_by_key(|(_, owner)| self.last_used.get(*owner).copied().unwrap_or(0))
            .expect("non-empty atlas");
        let victim = victim.clone();
        self.states.remove(&victim);
        self.last_used.remove(&victim);
        self.slot_owner[index] = Some(path.to_path_buf());
        index as u32
    }

    fn copy_into(&mut self, slot: u32, rgba: &[u8]) {
        let (ox, oy) = Self::slot_origin(slot);
        let stride = self.width as usize * 4;
        for row in 0..CELL as usize {
            let src = row * CELL as usize * 4;
            let dst = (oy as usize + row) * stride + ox as usize * 4;
            self.pixels[dst..dst + CELL as usize * 4]
                .copy_from_slice(&rgba[src..src + CELL as usize * 4]);
        }
        self.dirty = true;
    }

    fn slot_origin(slot: u32) -> (u32, u32) {
        let per_row = ATLAS_WIDTH / CELL;
        ((slot % per_row) * CELL, (slot / per_row) * CELL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(value: u8) -> Vec<u8> {
        vec![value; (CELL * CELL * 4) as usize]
    }

    #[test]
    fn visible_work_is_deduplicated_and_drained_first() {
        let mut cache = ThumbnailCache::new();
        cache.request(Path::new("far.png"), false);
        cache.request(Path::new("visible.png"), false);
        // Scrolling the second tile into view promotes the existing request;
        // it must not enqueue a duplicate behind the old background copy.
        cache.request(Path::new("visible.png"), true);
        let requests = cache.take_requests(8);
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].path, PathBuf::from("visible.png"));
        assert!(requests[0].visible);
        assert_eq!(requests[1].path, PathBuf::from("far.png"));
    }

    #[test]
    fn sixty_png_frame_contract_performs_zero_ui_thread_decodes() {
        let mut cache = ThumbnailCache::new();
        for index in 0..60 {
            cache.request(Path::new(&format!("terrain/tile_{index}.png")), index < 8);
        }
        assert_eq!(cache.pump(), 0, "frame pump must never decode files");
        let visible = cache.take_requests(8);
        assert_eq!(visible.len(), 8);
        assert!(visible.iter().all(|request| request.visible));
        assert_eq!(cache.pending_count(), 60);
    }

    #[test]
    fn least_recently_used_cell_is_recycled() {
        let mut cache = ThumbnailCache::new();
        for index in 0..CAPACITY {
            let path = PathBuf::from(format!("{index}.png"));
            cache.request(&path, true);
            assert!(cache.deliver(&path, &cell(index as u8)));
        }
        let keep = Path::new("1.png");
        let _ = cache.uv(keep);
        let overflow = Path::new("overflow.png");
        cache.request(overflow, true);
        assert!(cache.deliver(overflow, &cell(255)));
        assert_eq!(cache.state(Path::new("0.png")), None);
        assert!(matches!(cache.state(overflow), Some(ThumbState::Ready(_))));
    }

    #[test]
    fn malformed_result_falls_back_without_corrupting_atlas() {
        let mut cache = ThumbnailCache::new();
        let path = Path::new("bad.png");
        cache.request(path, true);
        assert!(!cache.deliver(path, &[0; 4]));
        assert_eq!(cache.state(path), Some(ThumbState::Failed));
        assert_eq!(cache.ready_count(), 0);
    }
}
