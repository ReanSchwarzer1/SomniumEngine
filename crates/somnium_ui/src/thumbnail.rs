//! Phase 27-G (Elysium) — asset thumbnails.
//!
//! The Content Drawer showed seven identical folder glyphs and two file glyphs.
//! `phase_27.md` §2.4 called it the weakest surface in the product, and §9.7-2
//! asked for real previews.
//!
//! # What this module does, and what it deliberately does not
//!
//! Two kinds of asset need two different answers:
//!
//! - **Images** (`png`, `jpg`, `tga`, `exr`, …) are decoded and downscaled here.
//!   No renderer is involved, so this half works end to end today.
//! - **Meshes, materials and scenes** need the engine to render a preview. That
//!   is a *request*, drained by the host through [`ThumbnailCache::take_requests`]
//!   and answered with [`ThumbnailCache::deliver`]. `somnium_ui` owns no
//!   renderer and must not grow one, so the hook is where the boundary is.
//!
//! Anything without a thumbnail keeps its type icon. A missing preview is a
//! normal state, not an error, and the drawer stays usable throughout.
//!
//! # Why the work is budgeted rather than threaded
//!
//! Decoding is done on the UI thread, at most [`DECODE_BUDGET_PER_FRAME`] assets
//! per frame. A background thread would need the atlas behind a lock and would
//! buy little: a 4096² source downscales in single-digit milliseconds, and the
//! budget bounds the worst case at a predictable cost per frame rather than an
//! unpredictable stall when a folder of 200 textures is opened.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

/// Side of one thumbnail cell, in atlas pixels.
pub const CELL: u32 = 64;
/// Atlas dimensions. 16 × 16 cells.
pub const ATLAS_WIDTH: u32 = CELL * 16;
pub const ATLAS_HEIGHT: u32 = CELL * 16;
/// Slots available before the cache starts refusing new work.
pub const CAPACITY: usize = 16 * 16;

/// `texture_id` reserved for the thumbnail atlas, following the font atlas (0)
/// and the icon atlas (1).
pub const THUMBNAIL_ATLAS_TEXTURE_ID: u32 = 2;

/// Assets decoded per frame. Bounds the worst case when a folder of textures is
/// opened; the rest arrive over the following frames.
pub const DECODE_BUDGET_PER_FRAME: usize = 2;

/// What a thumbnail slot currently holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThumbState {
    /// Queued; the tile shows its type icon meanwhile.
    Pending,
    /// Packed into the atlas at this slot.
    Ready(u32),
    /// Tried and cannot be previewed. Recorded so it is never retried, which is
    /// what stops a corrupt file being re-decoded every single frame.
    Failed,
}

/// A preview this crate cannot produce alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThumbnailRequest {
    pub path: PathBuf,
    /// Side length the host should render, in pixels.
    pub size: u32,
}

/// Which assets this module can preview without help.
pub fn is_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "png" | "jpg" | "jpeg" | "bmp" | "tga" | "gif" | "webp"
    )
}

/// Assets worth asking the engine to render.
pub fn needs_engine_render(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "gltf" | "glb" | "somnium"
    )
}

/// Thumbnail atlas plus the bookkeeping around it.
pub struct ThumbnailCache {
    /// RGBA8, `ATLAS_WIDTH * ATLAS_HEIGHT`.
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Set when the atlas changed; the pass uploads and clears it.
    pub dirty: bool,

    states: HashMap<PathBuf, ThumbState>,
    /// Images awaiting a decode slot on the UI thread.
    decode_queue: VecDeque<PathBuf>,
    /// Previews only the host can produce.
    render_queue: VecDeque<ThumbnailRequest>,
    next_slot: u32,
}

impl Default for ThumbnailCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ThumbnailCache {
    pub fn new() -> Self {
        Self {
            pixels: vec![0u8; (ATLAS_WIDTH * ATLAS_HEIGHT * 4) as usize],
            width: ATLAS_WIDTH,
            height: ATLAS_HEIGHT,
            dirty: false,
            states: HashMap::new(),
            decode_queue: VecDeque::new(),
            render_queue: VecDeque::new(),
            next_slot: 0,
        }
    }

    /// Ask for a preview of `path`.
    ///
    /// Idempotent: a path already known — pending, ready or failed — is never
    /// queued twice, so calling this from the drawer's per-frame rebuild costs
    /// a hash lookup rather than a decode.
    pub fn request(&mut self, path: &Path) {
        if self.states.contains_key(path) {
            return;
        }
        if self.next_slot as usize >= CAPACITY {
            // Out of slots. Recorded as failed so the tile settles on its type
            // icon instead of asking again every frame.
            self.states.insert(path.to_path_buf(), ThumbState::Failed);
            return;
        }
        if is_image(path) {
            self.states.insert(path.to_path_buf(), ThumbState::Pending);
            self.decode_queue.push_back(path.to_path_buf());
        } else if needs_engine_render(path) {
            self.states.insert(path.to_path_buf(), ThumbState::Pending);
            self.render_queue.push_back(ThumbnailRequest {
                path: path.to_path_buf(),
                size: CELL,
            });
        } else {
            self.states.insert(path.to_path_buf(), ThumbState::Failed);
        }
    }

    /// Decode up to [`DECODE_BUDGET_PER_FRAME`] queued images. Returns how many
    /// were packed, so a caller can tell whether the atlas needs re-uploading.
    pub fn pump(&mut self) -> usize {
        let mut done = 0;
        while done < DECODE_BUDGET_PER_FRAME {
            let Some(path) = self.decode_queue.pop_front() else {
                break;
            };
            match Self::decode(&path) {
                Some(rgba) => {
                    if let Some(slot) = self.pack(&rgba) {
                        self.states.insert(path, ThumbState::Ready(slot));
                    } else {
                        self.states.insert(path, ThumbState::Failed);
                    }
                }
                None => {
                    self.states.insert(path, ThumbState::Failed);
                }
            }
            done += 1;
        }
        done
    }

    /// Previews the host should render. Draining transfers ownership: the host
    /// must answer each one with [`Self::deliver`] or [`Self::mark_failed`], or
    /// the tile stays on its type icon forever.
    pub fn take_requests(&mut self) -> Vec<ThumbnailRequest> {
        self.render_queue.drain(..).collect()
    }

    /// Supply a rendered preview. `rgba` must be `CELL * CELL * 4` bytes.
    pub fn deliver(&mut self, path: &Path, rgba: &[u8]) -> bool {
        if rgba.len() != (CELL * CELL * 4) as usize {
            self.states.insert(path.to_path_buf(), ThumbState::Failed);
            return false;
        }
        match self.pack(rgba) {
            Some(slot) => {
                self.states.insert(path.to_path_buf(), ThumbState::Ready(slot));
                true
            }
            None => {
                self.states.insert(path.to_path_buf(), ThumbState::Failed);
                false
            }
        }
    }

    /// Record that a preview could not be produced.
    pub fn mark_failed(&mut self, path: &Path) {
        self.states.insert(path.to_path_buf(), ThumbState::Failed);
    }

    pub fn state(&self, path: &Path) -> Option<ThumbState> {
        self.states.get(path).copied()
    }

    /// UV rect of a ready thumbnail: `[u0, v0, u1, v1]`.
    pub fn uv(&self, path: &Path) -> Option<[f32; 4]> {
        let ThumbState::Ready(slot) = self.states.get(path).copied()? else {
            return None;
        };
        let (x, y) = Self::slot_origin(slot);
        let (w, h) = (self.width as f32, self.height as f32);
        Some([
            x as f32 / w,
            y as f32 / h,
            (x + CELL) as f32 / w,
            (y + CELL) as f32 / h,
        ])
    }

    pub fn ready_count(&self) -> usize {
        self.states
            .values()
            .filter(|s| matches!(s, ThumbState::Ready(_)))
            .count()
    }

    pub fn pending_count(&self) -> usize {
        self.states
            .values()
            .filter(|s| **s == ThumbState::Pending)
            .count()
    }

    /// Forget everything. Used when the content root changes, since paths from
    /// the old project are meaningless.
    pub fn clear(&mut self) {
        self.states.clear();
        self.decode_queue.clear();
        self.render_queue.clear();
        self.next_slot = 0;
        self.pixels.fill(0);
        self.dirty = true;
    }

    fn slot_origin(slot: u32) -> (u32, u32) {
        let per_row = ATLAS_WIDTH / CELL;
        ((slot % per_row) * CELL, (slot / per_row) * CELL)
    }

    /// Copy a `CELL × CELL` RGBA image into the next free slot.
    fn pack(&mut self, rgba: &[u8]) -> Option<u32> {
        if rgba.len() != (CELL * CELL * 4) as usize {
            return None;
        }
        let slot = self.next_slot;
        if slot as usize >= CAPACITY {
            return None;
        }
        self.next_slot += 1;
        let (ox, oy) = Self::slot_origin(slot);
        let stride = self.width as usize * 4;
        for row in 0..CELL as usize {
            let src = row * CELL as usize * 4;
            let dst = (oy as usize + row) * stride + ox as usize * 4;
            self.pixels[dst..dst + CELL as usize * 4]
                .copy_from_slice(&rgba[src..src + CELL as usize * 4]);
        }
        self.dirty = true;
        Some(slot)
    }

    /// Decode and downscale an image file to one cell.
    ///
    /// Fits rather than fills, on a transparent ground: a non-square texture
    /// keeps its aspect ratio, because a stretched preview is worse than a
    /// small one for telling two assets apart.
    fn decode(path: &Path) -> Option<Vec<u8>> {
        let img = image::open(path).ok()?;
        let fitted = img.thumbnail(CELL, CELL).to_rgba8();
        let (fw, fh) = (fitted.width(), fitted.height());
        if fw == 0 || fh == 0 {
            return None;
        }
        let mut cell = vec![0u8; (CELL * CELL * 4) as usize];
        let ox = (CELL - fw.min(CELL)) / 2;
        let oy = (CELL - fh.min(CELL)) / 2;
        for y in 0..fh.min(CELL) {
            for x in 0..fw.min(CELL) {
                let p = fitted.get_pixel(x, y).0;
                let d = (((oy + y) * CELL + (ox + x)) * 4) as usize;
                cell[d..d + 4].copy_from_slice(&p);
            }
        }
        Some(cell)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_cell(v: u8) -> Vec<u8> {
        vec![v; (CELL * CELL * 4) as usize]
    }

    #[test]
    fn image_and_mesh_extensions_route_to_different_answers() {
        assert!(is_image(Path::new("rock.png")));
        assert!(is_image(Path::new("ROCK.JPEG")), "case-insensitive");
        assert!(!is_image(Path::new("ship.glb")));
        assert!(needs_engine_render(Path::new("ship.glb")));
        assert!(needs_engine_render(Path::new("level.somnium")));
        assert!(!needs_engine_render(Path::new("rock.png")));
    }

    #[test]
    fn a_mesh_becomes_a_host_request_and_an_image_does_not() {
        let mut c = ThumbnailCache::new();
        c.request(Path::new("ship.glb"));
        c.request(Path::new("rock.png"));
        let reqs = c.take_requests();
        assert_eq!(reqs.len(), 1, "only the mesh needs the engine");
        assert_eq!(reqs[0].path, PathBuf::from("ship.glb"));
        assert_eq!(reqs[0].size, CELL);
    }

    #[test]
    fn an_unpreviewable_asset_settles_on_failed_immediately() {
        // A script has no picture. It must not sit Pending forever, or the tile
        // never stops asking.
        let mut c = ThumbnailCache::new();
        c.request(Path::new("boot.luau"));
        assert_eq!(c.state(Path::new("boot.luau")), Some(ThumbState::Failed));
        assert!(c.take_requests().is_empty());
    }

    #[test]
    fn requesting_the_same_path_twice_queues_it_once() {
        // The drawer rebuilds its tiles on every refresh, so this is the hot
        // path: it must cost a lookup, not a decode.
        let mut c = ThumbnailCache::new();
        for _ in 0..10 {
            c.request(Path::new("ship.glb"));
        }
        assert_eq!(c.take_requests().len(), 1);
    }

    #[test]
    fn a_delivered_preview_becomes_sampleable() {
        let mut c = ThumbnailCache::new();
        let p = Path::new("ship.glb");
        c.request(p);
        assert_eq!(c.uv(p), None, "pending has no uv");
        assert!(c.deliver(p, &solid_cell(200)));
        let uv = c.uv(p).expect("ready must have a uv");
        assert!(uv[0] < uv[2] && uv[1] < uv[3], "uv must be a real rect");
        assert!(uv.iter().all(|v| (0.0..=1.0).contains(v)));
        assert!(c.dirty, "packing must flag the atlas for upload");
        assert_eq!(c.ready_count(), 1);
    }

    #[test]
    fn a_wrong_sized_delivery_is_rejected_rather_than_corrupting_the_atlas() {
        let mut c = ThumbnailCache::new();
        let p = Path::new("ship.glb");
        c.request(p);
        assert!(!c.deliver(p, &[0u8; 16]));
        assert_eq!(c.state(p), Some(ThumbState::Failed));
        assert_eq!(c.uv(p), None);
    }

    #[test]
    fn slots_do_not_overlap() {
        let mut c = ThumbnailCache::new();
        let mut seen = Vec::new();
        for i in 0..8 {
            let p = PathBuf::from(format!("mesh{i}.glb"));
            c.request(&p);
            c.deliver(&p, &solid_cell(i as u8 + 1));
            let uv = c.uv(&p).expect("ready");
            assert!(!seen.contains(&uv), "slot {i} reused an earlier region");
            seen.push(uv);
        }
    }

    #[test]
    fn a_full_atlas_fails_gracefully_instead_of_panicking() {
        let mut c = ThumbnailCache::new();
        for i in 0..CAPACITY {
            let p = PathBuf::from(format!("m{i}.glb"));
            c.request(&p);
            c.deliver(&p, &solid_cell(1));
        }
        // Drain what filling the atlas queued, so the assertion below is about
        // the overflow and not about the 256 requests that preceded it.
        let _ = c.take_requests();

        let overflow = PathBuf::from("one_too_many.glb");
        c.request(&overflow);
        assert_eq!(
            c.state(&overflow),
            Some(ThumbState::Failed),
            "past capacity a tile must settle on its type icon"
        );
        assert!(c.take_requests().is_empty(), "and must not ask the host");
    }

    #[test]
    fn pump_respects_its_per_frame_budget() {
        // Opening a folder of textures must cost a bounded amount per frame.
        let mut c = ThumbnailCache::new();
        for i in 0..10 {
            c.request(&PathBuf::from(format!("t{i}.png")));
        }
        let first = c.pump();
        assert!(
            first <= DECODE_BUDGET_PER_FRAME,
            "decoded {first} in one frame"
        );
    }

    #[test]
    fn clearing_releases_every_slot() {
        let mut c = ThumbnailCache::new();
        let p = Path::new("ship.glb");
        c.request(p);
        c.deliver(p, &solid_cell(9));
        c.clear();
        assert_eq!(c.state(p), None);
        assert_eq!(c.ready_count(), 0);
        // And the next request starts from slot zero again.
        c.request(p);
        c.deliver(p, &solid_cell(9));
        assert_eq!(c.uv(p).map(|uv| uv[0]), Some(0.0));
    }
}
