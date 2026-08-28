//! Streaming source-page cache for the terrain runtime virtual texture.
//!
//! This module deliberately contains no wgpu objects or I/O. It turns a set of
//! logical pages observed by GPU feedback into a bounded, deterministic list of
//! physical-slot uploads. The caller owns reading those pages and publishing
//! the resulting page table to the GPU.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use somnium_asset::virtual_texture::VirtualPageId;

/// One logical page that should be installed into a physical cache slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageUpload {
    /// Logical source page requested by feedback.
    pub id: VirtualPageId,
    /// Stable index of the physical slot to overwrite.
    pub physical_slot: u32,
    /// Previous logical owner of the slot, if this upload evicts one.
    pub evicted: Option<VirtualPageId>,
}

/// Cumulative counters plus current cache gauges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VirtualTextureStats {
    /// Unique feedback pages that were already resident.
    pub hits: u64,
    /// Unique feedback pages first admitted to the pending queue.
    pub misses: u64,
    /// Resident pages displaced by later uploads.
    pub evictions: u64,
    /// Uploads admitted by the per-frame budget.
    pub uploads: u64,
    /// Pages currently mapped to physical slots.
    pub resident_pages: u32,
    /// Unique misses waiting for an upload slot or a later frame's budget.
    pub pending_pages: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct PhysicalSlot {
    id: Option<VirtualPageId>,
    last_used_frame: u64,
}

/// Deterministic LRU policy for one terrain's streaming source pages.
///
/// Feedback is deduplicated before accounting. A page already pending is not a
/// second miss, and a physical page demanded in the current frame cannot be
/// selected as that frame's eviction victim. Equal-age victims are resolved by
/// the lowest physical slot index, keeping tests and captures reproducible.
pub struct VirtualTextureCache {
    slots: Vec<PhysicalSlot>,
    resident: HashMap<VirtualPageId, u32>,
    pending: VecDeque<VirtualPageId>,
    pending_set: HashSet<VirtualPageId>,
    upload_budget: usize,
    stats: VirtualTextureStats,
}

impl VirtualTextureCache {
    /// Create a cache with a fixed physical capacity and upload count per call.
    ///
    /// A zero-capacity cache is valid: it records demand without ever producing
    /// uploads. This makes a disabled or unsupported runtime tier observable
    /// instead of turning configuration data into a panic.
    #[must_use]
    pub fn new(physical_pages: u32, upload_budget: u32) -> Self {
        Self {
            slots: vec![PhysicalSlot::default(); physical_pages as usize],
            resident: HashMap::with_capacity(physical_pages as usize),
            pending: VecDeque::new(),
            pending_set: HashSet::new(),
            upload_budget: upload_budget as usize,
            stats: VirtualTextureStats::default(),
        }
    }

    /// Fold one frame's feedback into the cache and return bounded upload work.
    ///
    /// Pending work survives frames with no feedback. This is important when a
    /// burst exceeds the upload budget: demand was already observed and must not
    /// depend on the GPU reporting the same miss again.
    pub fn resolve_feedback<I>(&mut self, frame: u64, feedback: I) -> Vec<PageUpload>
    where
        I: IntoIterator<Item = VirtualPageId>,
    {
        let mut demanded = BTreeSet::new();
        let mut ordered = Vec::new();
        for id in feedback {
            if demanded.insert(id) {
                ordered.push(id);
            }
        }

        for id in ordered {
            if let Some(&slot) = self.resident.get(&id) {
                self.stats.hits = self.stats.hits.saturating_add(1);
                self.slots[slot as usize].last_used_frame = frame;
            } else if self.pending_set.insert(id) {
                self.stats.misses = self.stats.misses.saturating_add(1);
                self.pending.push_back(id);
            }
        }

        let mut uploads = Vec::with_capacity(self.upload_budget.min(self.pending.len()));
        while uploads.len() < self.upload_budget {
            let Some(id) = self.pending.pop_front() else {
                break;
            };

            // Defensive against a future caller installing a page through a
            // separate path while its old queue entry remains.
            if self.resident.contains_key(&id) {
                self.pending_set.remove(&id);
                continue;
            }

            let Some((physical_slot, evicted)) = self.allocate_slot(&demanded) else {
                // Every resident page is protected by this frame's feedback (or
                // capacity is zero). Preserve FIFO order for the next frame.
                self.pending.push_front(id);
                break;
            };

            self.pending_set.remove(&id);
            if let Some(old) = evicted {
                // Clear the forward mapping before publishing the replacement;
                // otherwise `slot(old)` could alias the new page until the end
                // of this resolve call.
                self.resident.remove(&old);
                self.stats.evictions = self.stats.evictions.saturating_add(1);
            }
            self.slots[physical_slot as usize] = PhysicalSlot {
                id: Some(id),
                last_used_frame: frame,
            };
            self.resident.insert(id, physical_slot);
            self.stats.uploads = self.stats.uploads.saturating_add(1);
            uploads.push(PageUpload {
                id,
                physical_slot,
                evicted,
            });
        }

        self.refresh_gauges();
        uploads
    }

    /// Physical slot currently assigned to `id`.
    #[must_use]
    pub fn slot(&self, id: VirtualPageId) -> Option<u32> {
        self.resident.get(&id).copied()
    }

    /// Whether `id` is waiting for the upload budget or an evictable slot.
    #[must_use]
    pub fn is_pending(&self, id: VirtualPageId) -> bool {
        self.pending_set.contains(&id)
    }

    /// Cache counters and current gauges.
    #[must_use]
    pub const fn stats(&self) -> &VirtualTextureStats {
        &self.stats
    }

    /// Number of physical pages owned by this cache.
    #[must_use]
    pub fn capacity(&self) -> u32 {
        self.slots.len().min(u32::MAX as usize) as u32
    }

    /// Change the number of new pages admitted by each feedback resolve.
    pub fn set_upload_budget(&mut self, pages: u32) {
        self.upload_budget = pages as usize;
    }

    /// Roll back a batch whose source bytes could not be read.
    ///
    /// `resolve_feedback` reserves slots before the renderer performs I/O so
    /// that one deterministic policy owns allocation. Reversing the batch
    /// restores every evicted owner and puts the failed pages back at the
    /// front of the FIFO for a later retry.
    pub fn reject_uploads(&mut self, uploads: &[PageUpload]) {
        for upload in uploads.iter().rev() {
            self.resident.remove(&upload.id);
            self.pending.push_front(upload.id);
            self.pending_set.insert(upload.id);
            self.stats.uploads = self.stats.uploads.saturating_sub(1);

            self.slots[upload.physical_slot as usize] = PhysicalSlot {
                id: upload.evicted,
                last_used_frame: 0,
            };
            if let Some(old) = upload.evicted {
                self.resident.insert(old, upload.physical_slot);
                self.stats.evictions = self.stats.evictions.saturating_sub(1);
            }
        }
        self.refresh_gauges();
    }

    fn allocate_slot(
        &mut self,
        demanded: &BTreeSet<VirtualPageId>,
    ) -> Option<(u32, Option<VirtualPageId>)> {
        if let Some(index) = self.slots.iter().position(|slot| slot.id.is_none()) {
            return Some((index as u32, None));
        }

        self.slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.id.is_some_and(|id| !demanded.contains(&id)))
            .min_by_key(|(index, slot)| (slot.last_used_frame, *index))
            .map(|(index, slot)| (index as u32, slot.id))
    }

    fn refresh_gauges(&mut self) {
        self.stats.resident_pages = self.resident.len().min(u32::MAX as usize) as u32;
        self.stats.pending_pages = self.pending_set.len().min(u32::MAX as usize) as u32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_asset::virtual_texture::VirtualPageId;

    fn id(x: u16) -> VirtualPageId {
        VirtualPageId::new(0, 0, x, 0)
    }

    #[test]
    fn feedback_is_deduplicated_and_respects_the_upload_budget() {
        let mut cache = VirtualTextureCache::new(2, 1);
        let uploads = cache.resolve_feedback(1, [id(0), id(0), id(1)]);
        assert_eq!(uploads.len(), 1);
        assert_eq!(uploads[0].id, id(0));
        assert_eq!(cache.stats().misses, 2);
        assert_eq!(cache.stats().pending_pages, 1);

        let uploads = cache.resolve_feedback(2, std::iter::empty());
        assert_eq!(uploads.len(), 1);
        assert_eq!(uploads[0].id, id(1));
        assert_eq!(cache.stats().resident_pages, 2);
    }

    #[test]
    fn least_recently_used_page_is_evicted_but_a_hit_is_retained() {
        let mut cache = VirtualTextureCache::new(2, 2);
        cache.resolve_feedback(1, [id(0), id(1)]);
        cache.resolve_feedback(2, [id(0)]);
        let uploads = cache.resolve_feedback(3, [id(2)]);
        assert_eq!(uploads[0].evicted, Some(id(1)));
        assert_eq!(cache.slot(id(0)), Some(0));
        assert_eq!(cache.slot(id(1)), None);
        assert!(cache.slot(id(2)).is_some());
        assert_eq!(cache.stats().hits, 1);
    }

    #[test]
    fn slot_reuse_clears_the_evicted_pages_mapping() {
        let mut cache = VirtualTextureCache::new(1, 1);
        let first = cache.resolve_feedback(1, [id(4)]);
        assert_eq!(first[0].physical_slot, 0);

        let replacement = cache.resolve_feedback(2, [id(5)]);
        assert_eq!(replacement[0].physical_slot, 0);
        assert_eq!(replacement[0].evicted, Some(id(4)));
        assert_eq!(cache.slot(id(4)), None);
        assert_eq!(cache.slot(id(5)), Some(0));
        assert_eq!(cache.stats().evictions, 1);
    }

    #[test]
    fn pending_feedback_is_unique_and_survives_an_empty_frame() {
        let mut cache = VirtualTextureCache::new(2, 0);
        assert!(cache.resolve_feedback(1, [id(7), id(7)]).is_empty());
        assert!(cache.is_pending(id(7)));
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().pending_pages, 1);

        assert!(cache.resolve_feedback(2, [id(7)]).is_empty());
        assert_eq!(cache.stats().misses, 1, "a pending page is not a new miss");
        assert_eq!(cache.stats().pending_pages, 1);
    }

    #[test]
    fn current_demand_is_not_evicted_to_service_another_current_miss() {
        let mut cache = VirtualTextureCache::new(1, 2);
        cache.resolve_feedback(1, [id(0)]);

        let uploads = cache.resolve_feedback(2, [id(0), id(1)]);
        assert!(uploads.is_empty());
        assert_eq!(cache.slot(id(0)), Some(0));
        assert!(cache.is_pending(id(1)));

        let uploads = cache.resolve_feedback(3, std::iter::empty());
        assert_eq!(uploads[0].evicted, Some(id(0)));
        assert_eq!(cache.slot(id(1)), Some(0));
    }

    #[test]
    fn failed_batch_restores_evictions_and_requeues_uploads() {
        let mut cache = VirtualTextureCache::new(1, 1);
        cache.resolve_feedback(1, [id(0)]);
        let uploads = cache.resolve_feedback(2, [id(1)]);
        assert_eq!(cache.slot(id(1)), Some(0));

        cache.reject_uploads(&uploads);

        assert_eq!(cache.slot(id(0)), Some(0));
        assert_eq!(cache.slot(id(1)), None);
        assert!(cache.is_pending(id(1)));
        assert_eq!(cache.stats().evictions, 0);
        assert_eq!(cache.stats().uploads, 1);
    }
}
