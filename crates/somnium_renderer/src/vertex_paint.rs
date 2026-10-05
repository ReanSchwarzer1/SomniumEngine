//! Per-instance vertex paint: an RGBA8 mask per vertex. Each channel blends in
//! a full PBR material (a *layer*) where one is assigned, and otherwise the
//! shading pass's built-in weathering: R dirt, G rust, B wetness, A blood.
//!
//! One storage buffer of `u32` words, laid out as:
//!
//! - word 0: start of this frame's instance table, or 0 when nothing is
//!   painted (the shader's early out);
//! - word 1: preview mode (0 off, 1 all channels, 2..=5 one channel);
//! - words 2..=5: the editor brush as f32 bits (centre xyz, radius; radius 0
//!   hides it), drawn as a ring where the brush sphere meets a surface;
//! - words 6..: each painted entity's slot: [`SLOT_HEADER_WORDS`] of layer
//!   settings ([`PaintLayers::words`]), then its masks, one word per vertex,
//!   in the mesh's own vertex order;
//! - from word 0's value: one word per opaque instance, the handle of its
//!   masks (0 = unpainted).
//!
//! A handle is the word index of the slot's header, so it is never 0.
//! Two entities sharing one mesh get separate slots: paint belongs to the
//! placement, not to the asset.

use std::collections::HashMap;

/// First word available to slots.
pub const HEADER_WORDS: u32 = 6;
/// Words of layer settings at the start of every slot. `shading.wgsl` reads
/// the same number as `PAINT_SLOT_HEADER`.
pub const SLOT_HEADER_WORDS: u32 = 20;
/// "This channel has no layer material": the built-in weathering applies.
pub const NO_LAYER: u32 = u32::MAX;

/// What each of the four mask channels paints, as the shader reads it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintLayers {
    /// Material pool index per channel, or [`NO_LAYER`].
    pub material: [u32; 4],
    /// Texture repeats per world metre (the layer is projected in world
    /// space, so one material keeps one texel density on every mesh).
    pub tiling: [f32; 4],
    /// 0 fades by the painted amount alone; 1 lets the two height maps decide
    /// the edge, so a layer fills the surface's crevices before it covers it.
    pub height_contrast: [f32; 4],
    /// Lowest and highest world normal Y the layer shows on (-1 and 1: any).
    pub slope_min: [f32; 4],
    pub slope_max: [f32; 4],
    /// How far world-space noise breaks up half-painted areas, `0..1`.
    pub breakup: [f32; 4],
    /// Size of that noise in metres per blotch.
    pub breakup_scale: [f32; 4],
    /// 0 replaces the surface with the layer; 1 only stains it (the surface
    /// keeps its own colour pattern, normal and relief).
    pub stain: [f32; 4],
    /// Above 0.5 the layer's maps are read twice at noise-picked offsets and
    /// blended, which hides tiling in organic textures. Off for anything with
    /// courses or joints.
    pub detile: [f32; 4],
}

impl Default for PaintLayers {
    fn default() -> Self {
        Self {
            material: [NO_LAYER; 4],
            tiling: [0.5; 4],
            height_contrast: [0.6; 4],
            slope_min: [-1.0; 4],
            slope_max: [1.0; 4],
            breakup: [0.5; 4],
            breakup_scale: [1.5; 4],
            stain: [0.0; 4],
            detile: [0.0; 4],
        }
    }
}

impl PaintLayers {
    /// The slot header: material ids, tilings, packed parameters (contrast,
    /// slope min, slope max, breakup as bytes), noise frequencies, and a
    /// second packed word (stain, de-tile).
    #[must_use]
    pub fn words(&self) -> [u32; SLOT_HEADER_WORDS as usize] {
        let byte = |v: f32| u32::from((v.clamp(0.0, 1.0) * 255.0).round() as u8);
        let mut out = [0; SLOT_HEADER_WORDS as usize];
        for i in 0..4 {
            out[i] = self.material[i];
            out[4 + i] = self.tiling[i].to_bits();
            out[8 + i] = byte(self.height_contrast[i])
                | (byte(self.slope_min[i] * 0.5 + 0.5) << 8)
                | (byte(self.slope_max[i] * 0.5 + 0.5) << 16)
                | (byte(self.breakup[i]) << 24);
            out[12 + i] = (1.0 / self.breakup_scale[i].max(0.01)).to_bits();
            out[16 + i] = byte(self.stain[i]) | (byte(self.detile[i]) << 8);
        }
        out
    }
}

#[derive(Default)]
pub struct VertexPaintPool {
    /// CPU mirror of the slot region, starting at word [`HEADER_WORDS`].
    words: Vec<u32>,
    /// key -> (handle, vertex count)
    slots: HashMap<u64, (u32, u32)>,
    /// Word range of `words` that has changed since the last upload.
    dirty: Option<(usize, usize)>,
    /// Instance table of the frame being built, one word per opaque draw.
    pub frame_instances: Vec<u32>,
    /// 0 off, 1 all channels, 2..=5 one channel (dirt, rust, wet, blood).
    pub preview: u32,
    /// Editor brush ring: centre xyz and radius (0 = hidden).
    pub brush: [f32; 4],
    /// Whether the GPU header currently says "painted", so turning paint off
    /// writes the zero header once instead of every frame.
    pub header_live: bool,
}

impl VertexPaintPool {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Store `colors` and their `layers` for `key` and return its handle. Same
    /// key, same length rewrites in place, so a brush stroke keeps its handle.
    ///
    /// ponytail: a slot that changes length (or is removed) is leaked until
    /// [`VertexPaintPool::clear`]; compact if a long session ever grows it.
    pub fn set(&mut self, key: u64, layers: &PaintLayers, colors: &[u32]) -> u32 {
        let len = colors.len() as u32;
        let header = SLOT_HEADER_WORDS as usize;
        let handle = match self.slots.get(&key) {
            Some(&(handle, n)) if n == len => handle,
            _ => {
                let handle = HEADER_WORDS + self.words.len() as u32;
                self.words.resize(self.words.len() + header + colors.len(), 0);
                self.slots.insert(key, (handle, len));
                handle
            }
        };
        let start = (handle - HEADER_WORDS) as usize;
        self.words[start..start + header].copy_from_slice(&layers.words());
        self.words[start + header..start + header + colors.len()].copy_from_slice(colors);
        let end = start + header + colors.len();
        self.dirty = Some(match self.dirty {
            Some((a, b)) => (a.min(start), b.max(end)),
            None => (start, end),
        });
        handle
    }

    #[must_use]
    pub fn handle(&self, key: u64) -> u32 {
        self.slots.get(&key).map_or(0, |&(handle, _)| handle)
    }

    /// Drop every slot whose key `keep` rejects.
    pub fn retain(&mut self, mut keep: impl FnMut(u64) -> bool) {
        self.slots.retain(|key, _| keep(*key));
        if self.slots.is_empty() {
            self.clear();
        }
    }

    pub fn clear(&mut self) {
        self.words.clear();
        self.slots.clear();
        self.dirty = None;
    }

    /// Whether the shader needs a live header this frame.
    #[must_use]
    pub fn wants_header(&self) -> bool {
        !self.is_empty() || self.preview != 0 || self.brush[3] > 0.0
    }

    /// The header words for an instance table at `table` (0 = none).
    #[must_use]
    pub fn header(&self, table: u32) -> [u32; HEADER_WORDS as usize] {
        let [x, y, z, r] = self.brush.map(f32::to_bits);
        [table, self.preview, x, y, z, r]
    }

    /// Words the GPU buffer needs for the vertex region plus `instances`.
    #[must_use]
    pub fn required_words(&self, instances: usize) -> u64 {
        u64::from(HEADER_WORDS) + self.words.len() as u64 + instances as u64
    }

    /// Start of the instance table: right after the vertex region.
    #[must_use]
    pub fn instance_table_start(&self) -> u32 {
        HEADER_WORDS + self.words.len() as u32
    }

    /// The changed vertex words as `(first word index, words)`, clearing the
    /// record. `all` returns the whole region (after the buffer was recreated).
    pub fn take_dirty(&mut self, all: bool) -> Option<(u32, &[u32])> {
        let range = if all {
            (!self.words.is_empty()).then_some((0, self.words.len()))
        } else {
            self.dirty
        };
        self.dirty = None;
        range.map(|(a, b)| (HEADER_WORDS + a as u32, &self.words[a..b]))
    }
}

/// Pack a 0..1 RGBA mask the way the shader's `unpack4x8unorm` reads it.
#[must_use]
pub fn pack(mask: [f32; 4]) -> u32 {
    mask.iter().enumerate().fold(0, |word, (i, v)| {
        word | (u32::from((v.clamp(0.0, 1.0) * 255.0).round() as u8) << (8 * i))
    })
}

#[must_use]
pub fn unpack(word: u32) -> [f32; 4] {
    std::array::from_fn(|i| ((word >> (8 * i)) & 255) as f32 / 255.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_keep_their_handle_and_never_collide_with_the_header() {
        let mut pool = VertexPaintPool::default();
        let layers = PaintLayers::default();
        let a = pool.set(1, &layers, &[1, 2, 3]);
        let b = pool.set(2, &layers, &[4, 5]);
        assert_eq!(a, HEADER_WORDS);
        assert_eq!(b, HEADER_WORDS + SLOT_HEADER_WORDS + 3);
        assert_eq!(pool.set(1, &layers, &[7, 8, 9]), a, "same length rewrites in place");
        assert_eq!(pool.handle(2), b);
        assert_eq!(pool.handle(3), 0);
        let (first, words) = pool.take_dirty(false).unwrap();
        assert_eq!(first, HEADER_WORDS);
        let h = SLOT_HEADER_WORDS as usize;
        assert_eq!(&words[h..h + 3], &[7, 8, 9]);
        assert_eq!(&words[2 * h + 3..], &[4, 5]);
        assert!(pool.take_dirty(false).is_none());
        pool.retain(|k| k == 2);
        assert_eq!(pool.handle(1), 0);
        assert_eq!(pool.instance_table_start(), HEADER_WORDS + 2 * SLOT_HEADER_WORDS + 5);
    }

    #[test]
    fn a_slot_header_carries_the_layer_settings_the_shader_unpacks() {
        let layers = PaintLayers {
            material: [7, NO_LAYER, NO_LAYER, 3],
            tiling: [0.25, 0.5, 0.5, 2.0],
            height_contrast: [1.0, 0.6, 0.6, 0.0],
            slope_min: [0.0, -1.0, -1.0, -1.0],
            slope_max: [1.0; 4],
            breakup: [1.0, 0.5, 0.5, 0.0],
            breakup_scale: [2.0, 1.5, 1.5, 0.5],
            stain: [0.0, 1.0, 0.0, 0.0],
            detile: [1.0, 0.0, 0.0, 0.0],
        };
        let w = layers.words();
        assert_eq!((w[0], w[1], w[3]), (7, NO_LAYER, 3));
        assert_eq!(f32::from_bits(w[4]), 0.25);
        // contrast 255, slope min 0 -> 128, slope max 1 -> 255, breakup 255.
        assert_eq!(w[8], 255 | (128 << 8) | (255 << 16) | (255 << 24));
        assert_eq!(w[11] & 255, 0);
        assert_eq!(f32::from_bits(w[12]), 0.5, "noise frequency is 1 / blotch size");
        assert_eq!((w[16], w[17]), (255 << 8, 255), "de-tile in byte 1, stain in byte 0");
    }

    #[test]
    fn pack_matches_unpack4x8unorm_channel_order() {
        let word = pack([1.0, 0.0, 0.5, 0.25]);
        assert_eq!(word & 255, 255);
        assert_eq!((word >> 16) & 255, 128);
        let back = unpack(word);
        assert!((back[3] - 0.25).abs() < 0.01);
    }
}
