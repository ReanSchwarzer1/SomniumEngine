//! Deterministic frame capture and A/B comparison (Phase 25A-2).
//!
//! Sub-phases in this project are judged by whether an image changed, and until
//! now that meant screen-grabbing the window. Those numbers were not usable: a
//! frame-delta metric taken that way varied from 0.776 to 2.018 across three
//! runs of one identical build, and a whole session was spent chasing the
//! variance rather than the change.
//!
//! This reads the engine's own HDR target back at a **fixed frame index**,
//! before tone mapping, exposure and TAA resolve, so two runs of the same build
//! with the same input produce byte-identical output and any difference is the
//! thing under test. It reads the visibility buffer alongside it, which is what
//! lets a comparison say "changed *on terrain*" instead of "changed somewhere".
//!
//! ```text
//! SOMNIUM_CAPTURE=before.somcap  SOMNIUM_TERRAIN=1  hello_engine
//! SOMNIUM_CAPTURE_COMPARE=before.somcap  SOMNIUM_GTAO=0  SOMNIUM_TERRAIN=1  hello_engine
//! ```
//!
//! The second run logs the mean absolute luminance difference over terrain
//! pixels and over everything else, separately.

use std::sync::atomic::{AtomicBool, Ordering};

/// Set after a capture has been written. `SOMNIUM_CAPTURE_QUIT=1` polls this
/// so a headless evidence run can exit instead of sitting on a window.
static CAPTURE_FINISHED: AtomicBool = AtomicBool::new(false);

/// True once [`FrameCapture::resolve`] has written or compared a frame.
pub fn finished() -> bool {
    CAPTURE_FINISHED.load(Ordering::Relaxed)
}

/// Frame the capture fires on unless `SOMNIUM_CAPTURE_FRAME` says otherwise.
///
/// Late enough that TAA has converged, auto-exposure has settled and the ReSTIR
/// reservoirs have accumulated — all of which are deterministic functions of
/// the frame index, so both sides of an A/B land in the same state.
const DEFAULT_CAPTURE_FRAME: u64 = 240;

const MAGIC: &[u8; 8] = b"SOMCAP01";

/// One captured frame: linear HDR radiance plus a per-pixel terrain mask.
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    /// Linear RGB, `width * height * 3`.
    pub rgb: Vec<f32>,
    /// What each pixel is: [`PIXEL_MESH`], [`PIXEL_TERRAIN`] or [`PIXEL_SKY`].
    ///
    /// Sky is called out separately because it is most of the frame and none of
    /// any lighting question — averaging it in buries whatever the A/B was
    /// actually about under a constant.
    pub terrain: Vec<u8>,
}

/// Visibility-buffer geometry that is not terrain.
pub const PIXEL_MESH: u8 = 0;
/// A terrain chunk.
pub const PIXEL_TERRAIN: u8 = 1;
/// The background: nothing was drawn here.
pub const PIXEL_SKY: u8 = 2;

impl CapturedFrame {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.rgb.len() * 4 + self.terrain.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(bytemuck::cast_slice(&self.rgb));
        out.extend_from_slice(&self.terrain);
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 16 || &bytes[..8] != MAGIC {
            return Err("not a Somnium frame capture".into());
        }
        let width = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let height = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let pixels = width as usize * height as usize;
        let rgb_bytes = pixels * 3 * 4;
        if bytes.len() < 16 + rgb_bytes + pixels {
            return Err("truncated frame capture".into());
        }
        let rgb = bytemuck::cast_slice::<u8, f32>(&bytes[16..16 + rgb_bytes]).to_vec();
        let terrain = bytes[16 + rgb_bytes..16 + rgb_bytes + pixels].to_vec();
        Ok(Self {
            width,
            height,
            rgb,
            terrain,
        })
    }

    /// Rec.709 luminance of pixel `i`.
    fn luminance(&self, i: usize) -> f32 {
        0.2126 * self.rgb[i * 3] + 0.7152 * self.rgb[i * 3 + 1] + 0.0722 * self.rgb[i * 3 + 2]
    }
}

/// Mean absolute luminance difference and the count of clearly-changed pixels,
/// over one subset of the frame.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct DiffStats {
    pub pixels: usize,
    pub mean_abs: f64,
    /// Pixels whose luminance moved by more than 1% of the reference's own
    /// luminance. Relative, because HDR radiance spans orders of magnitude and
    /// a fixed threshold means "everything" in daylight and "nothing" at night.
    pub changed: usize,
}

/// Compare two captures over the pixels of one class.
pub fn compare(before: &CapturedFrame, after: &CapturedFrame, class: u8) -> DiffStats {
    let mut stats = DiffStats::default();
    let mut sum = 0.0f64;
    for i in 0..before.terrain.len().min(after.terrain.len()) {
        if before.terrain[i] != class {
            continue;
        }
        let a = before.luminance(i);
        let b = after.luminance(i);
        let d = (a - b).abs();
        stats.pixels += 1;
        sum += d as f64;
        if d > 0.01 * a.abs().max(1e-4) {
            stats.changed += 1;
        }
    }
    if stats.pixels > 0 {
        stats.mean_abs = sum / stats.pixels as f64;
    }
    stats
}

// ── PNG output ──────────────────────────────────────────────────────────────
//
// `SOMNIUM_CAPTURE_PNG=<file>` writes the captured frame as an ordinary image.
//
// Every phase in this project is judged by looking at something, and the only
// way to look at it was to grab the desktop — which needs the window in front,
// and silently produces a picture of whatever *was* in front when it is not.
// That failed twice in one session and wasted both runs. Writing the image from
// the frame the engine already read back removes the desktop from the loop
// entirely, and needs no image crate: PNG's container is a handful of CRC'd
// chunks and its compression has a legal "store it uncompressed" mode.

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in bytes {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut typed = kind.to_vec();
    typed.extend_from_slice(data);
    out.extend_from_slice(&typed);
    out.extend_from_slice(&crc32(&typed).to_be_bytes());
}

/// Encode 8-bit RGB rows as a PNG.
///
/// Deflate is used in **stored** mode — no compression. A real encoder would
/// halve the file; this one has no dependencies and cannot get the entropy
/// coding wrong, which is the right trade for a diagnostic.
fn encode_png(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width as usize * 3 + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0); // filter: none
        let row = y * width as usize * 3;
        raw.extend_from_slice(&rgb[row..row + width as usize * 3]);
    }

    let mut zlib = vec![0x78, 0x01];
    let mut offset = 0usize;
    while offset < raw.len() {
        let n = (raw.len() - offset).min(65535);
        let last = u8::from(offset + n == raw.len());
        zlib.push(last);
        zlib.extend_from_slice(&(n as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(n as u16)).to_le_bytes());
        zlib.extend_from_slice(&raw[offset..offset + n]);
        offset += n;
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit, truecolour RGB
    png_chunk(&mut out, b"IHDR", &ihdr);
    png_chunk(&mut out, b"IDAT", &zlib);
    png_chunk(&mut out, b"IEND", &[]);
    out
}

/// Tone-map linear HDR to 8-bit sRGB for viewing.
///
/// Metered off the frame's own geometry rather than a fixed exposure: the scene
/// spans 100 000 lux daylight to moonlight, and a constant would render one of
/// those as white and the other as black. Sky is excluded from the metering for
/// the same reason it is a separate class in the diff — it is most of the frame
/// and none of what is being looked at.
fn tonemap_to_srgb(frame: &CapturedFrame) -> Vec<u8> {
    let mut sum = 0.0f64;
    let mut n = 0usize;
    for i in 0..frame.terrain.len() {
        if frame.terrain[i] != PIXEL_SKY {
            sum += frame.luminance(i) as f64;
            n += 1;
        }
    }
    if n == 0 {
        // Water and other forward-rendered surfaces never enter the visibility
        // buffer, so a frame looking out to sea meters as entirely sky. Falling
        // back to the whole image is what keeps such a capture readable instead
        // of exposing it against an assumed mean of one and clipping to white.
        for i in 0..frame.terrain.len() {
            sum += frame.luminance(i) as f64;
            n += 1;
        }
    }
    let mean = if n > 0 { (sum / n as f64) as f32 } else { 1.0 };
    // Middle grey at 0.18, the usual photographic anchor.
    let exposure = 0.18 / mean.max(1e-6);

    let mut out = Vec::with_capacity(frame.rgb.len());
    for i in 0..frame.terrain.len() {
        for c in 0..3 {
            let v = (frame.rgb[i * 3 + c] * exposure).max(0.0);
            let mapped = v / (1.0 + v); // Reinhard
            let srgb = mapped.powf(1.0 / 2.2);
            out.push((srgb.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
        }
    }
    out
}

/// Half-precision to single-precision, for the `Rgba16Float` HDR target.
fn f16_to_f32(bits: u16) -> f32 {
    let sign = ((bits >> 15) & 1) as u32;
    let exp = ((bits >> 10) & 0x1f) as u32;
    let mant = (bits & 0x3ff) as u32;
    let out = match exp {
        // Zero and subnormals: rebuild by normalising the mantissa rather than
        // flushing to zero, since GTAO's difference on a dark surface can live
        // entirely down here.
        0 => {
            if mant == 0 {
                sign << 31
            } else {
                // A half subnormal is `mant * 2^-24`. Shift the mantissa left
                // until bit 10 is set, which is the implicit leading 1 of the
                // normalised form; `k` shifts means an exponent of `-14 - k`,
                // so the f32 biased exponent is `127 - 14 - k`.
                let mut k = 0u32;
                let mut m = mant;
                while m & 0x400 == 0 {
                    m <<= 1;
                    k += 1;
                }
                m &= 0x3ff;
                (sign << 31) | ((113 - k) << 23) | (m << 13)
            }
        }
        // Inf / NaN.
        0x1f => (sign << 31) | 0x7f80_0000 | (mant << 13),
        _ => (sign << 31) | ((exp + 127 - 15) << 23) | (mant << 13),
    };
    f32::from_bits(out)
}

/// Drives the readback: allocates staging buffers, records the copies, and
/// resolves them once the GPU is done.
pub struct FrameCapture {
    write_to: Option<String>,
    write_png: Option<String>,
    compare_to: Option<String>,
    target_frame: u64,
    frame: u64,
    hdr_staging: Option<wgpu::Buffer>,
    vis_staging: Option<wgpu::Buffer>,
    /// Set by `record`, consumed by `resolve`.
    pending: Option<(u32, u32)>,
}

impl FrameCapture {
    pub fn from_env() -> Self {
        Self {
            write_to: std::env::var("SOMNIUM_CAPTURE").ok(),
            write_png: std::env::var("SOMNIUM_CAPTURE_PNG").ok(),
            compare_to: std::env::var("SOMNIUM_CAPTURE_COMPARE").ok(),
            target_frame: std::env::var("SOMNIUM_CAPTURE_FRAME")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_CAPTURE_FRAME),
            frame: 0,
            hdr_staging: None,
            vis_staging: None,
            pending: None,
        }
    }

    /// Whether anything is configured at all. Keeps the per-frame cost at one
    /// bool test when it is not.
    pub fn active(&self) -> bool {
        self.write_to.is_some() || self.write_png.is_some() || self.compare_to.is_some()
    }

    /// Advance the frame counter and report whether this is the capture frame.
    pub fn tick(&mut self) -> bool {
        if !self.active() {
            return false;
        }
        self.frame += 1;
        self.frame == self.target_frame
    }

    /// Bytes per padded row for a texture of `width` texels of `texel_bytes`.
    fn padded_row(width: u32, texel_bytes: u32) -> u32 {
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        (width * texel_bytes).div_ceil(align) * align
    }

    /// Copy the HDR target and the visibility buffer into staging buffers.
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        hdr: &wgpu::Texture,
        vis: &wgpu::Texture,
        width: u32,
        height: u32,
    ) {
        let hdr_row = Self::padded_row(width, 8); // Rgba16Float
        let vis_row = Self::padded_row(width, 8); // Rg32Uint
        let alloc = |size: u64, label: &'static str| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        };
        self.hdr_staging = Some(alloc(hdr_row as u64 * height as u64, "Frame Capture HDR"));
        self.vis_staging = Some(alloc(vis_row as u64 * height as u64, "Frame Capture Vis"));

        let copy = |encoder: &mut wgpu::CommandEncoder,
                    tex: &wgpu::Texture,
                    buf: &wgpu::Buffer,
                    row: u32| {
            encoder.copy_texture_to_buffer(
                tex.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: buf,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        };
        copy(encoder, hdr, self.hdr_staging.as_ref().unwrap(), hdr_row);
        copy(encoder, vis, self.vis_staging.as_ref().unwrap(), vis_row);
        self.pending = Some((width, height));
    }

    /// Map the staging buffers, build the capture, then write and/or compare it.
    ///
    /// `is_terrain_instance` answers whether a visibility-buffer instance id
    /// belongs to a terrain chunk; the renderer knows because it built the draw
    /// queue this frame.
    pub fn resolve(&mut self, device: &wgpu::Device, is_terrain_instance: impl Fn(u32) -> bool) {
        let Some((width, height)) = self.pending.take() else {
            return;
        };
        let (Some(hdr_buf), Some(vis_buf)) = (&self.hdr_staging, &self.vis_staging) else {
            return;
        };
        let hdr_row = Self::padded_row(width, 8) as usize;
        let vis_row = Self::padded_row(width, 8) as usize;
        let pixels = width as usize * height as usize;

        let mut rgb = vec![0.0f32; pixels * 3];
        let mut terrain = vec![0u8; pixels];

        for (buf, is_hdr) in [(hdr_buf, true), (vis_buf, false)] {
            let slice = buf.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            let data = slice.get_mapped_range();
            let row_bytes = if is_hdr { hdr_row } else { vis_row };
            for y in 0..height as usize {
                let row = &data[y * row_bytes..];
                for x in 0..width as usize {
                    let i = y * width as usize + x;
                    if is_hdr {
                        let t = &row[x * 8..x * 8 + 6];
                        for c in 0..3 {
                            rgb[i * 3 + c] =
                                f16_to_f32(u16::from_le_bytes([t[c * 2], t[c * 2 + 1]]));
                        }
                    } else {
                        let t = &row[x * 8..x * 8 + 4];
                        let packed = u32::from_le_bytes([t[0], t[1], t[2], t[3]]);
                        // 0 is the sky sentinel; instance ids are stored +1.
                        terrain[i] = if packed == 0 {
                            PIXEL_SKY
                        } else if is_terrain_instance(packed - 1) {
                            PIXEL_TERRAIN
                        } else {
                            PIXEL_MESH
                        };
                    }
                }
            }
            drop(data);
            buf.unmap();
        }

        let captured = CapturedFrame {
            width,
            height,
            rgb,
            terrain,
        };
        // Mean luminance per class, logged because a diff of exactly zero means
        // the same thing whether the feature under test does nothing or the
        // readback is empty — and those need telling apart.
        let stat = |class: u8| {
            let mut sum = 0.0f64;
            let mut n = 0usize;
            for i in 0..captured.terrain.len() {
                if captured.terrain[i] == class {
                    sum += captured.luminance(i) as f64;
                    n += 1;
                }
            }
            (n, if n == 0 { 0.0 } else { sum / n as f64 })
        };
        let (n_terrain, lum_terrain) = stat(PIXEL_TERRAIN);
        let (n_mesh, lum_mesh) = stat(PIXEL_MESH);
        let (n_sky, lum_sky) = stat(PIXEL_SKY);
        tracing::info!(
            "CAPTURE frame={} {}x{} | terrain px={n_terrain} lum={lum_terrain:.4} \
             | mesh px={n_mesh} lum={lum_mesh:.4} | sky px={n_sky} lum={lum_sky:.4}",
            self.frame,
            width,
            height,
        );

        if let Some(path) = &self.write_to {
            match std::fs::write(path, captured.encode()) {
                Ok(()) => tracing::info!("CAPTURE written to {path}"),
                Err(e) => tracing::error!("CAPTURE write to {path} failed: {e}"),
            }
        }

        if let Some(path) = &self.write_png {
            let rgb = tonemap_to_srgb(&captured);
            match std::fs::write(path, encode_png(width, height, &rgb)) {
                Ok(()) => tracing::info!("CAPTURE png written to {path}"),
                Err(e) => tracing::error!("CAPTURE png write to {path} failed: {e}"),
            }
        }

        if let Some(path) = &self.compare_to {
            match std::fs::read(path)
                .map_err(|e| e.to_string())
                .and_then(|b| CapturedFrame::decode(&b))
            {
                Ok(before) if before.width == width && before.height == height => {
                    let t = compare(&before, &captured, PIXEL_TERRAIN);
                    let m = compare(&before, &captured, PIXEL_MESH);
                    let s = compare(&before, &captured, PIXEL_SKY);
                    tracing::info!(
                        "CAPTURE-DIFF vs {path}: terrain px={} mean_abs={:.4} changed={} \
                         | mesh px={} mean_abs={:.4} changed={} \
                         | sky px={} mean_abs={:.4} changed={}",
                        t.pixels,
                        t.mean_abs,
                        t.changed,
                        m.pixels,
                        m.mean_abs,
                        m.changed,
                        s.pixels,
                        s.mean_abs,
                        s.changed,
                    );
                }
                Ok(before) => tracing::error!(
                    "CAPTURE-DIFF: {path} is {}x{}, this frame is {width}x{height}",
                    before.width,
                    before.height,
                ),
                Err(e) => tracing::error!("CAPTURE-DIFF read {path} failed: {e}"),
            }
        }

        self.hdr_staging = None;
        self.vis_staging = None;
        CAPTURE_FINISHED.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(lums: &[f32], terrain: &[u8]) -> CapturedFrame {
        let mut rgb = Vec::new();
        for l in lums {
            // Grey, so luminance is exactly the value.
            rgb.extend_from_slice(&[*l, *l, *l]);
        }
        CapturedFrame {
            width: lums.len() as u32,
            height: 1,
            rgb,
            terrain: terrain.to_vec(),
        }
    }

    #[test]
    fn f16_round_trips_the_values_that_matter() {
        assert_eq!(f16_to_f32(0x0000), 0.0);
        assert_eq!(f16_to_f32(0x3C00), 1.0);
        assert_eq!(f16_to_f32(0xBC00), -1.0);
        assert_eq!(f16_to_f32(0x4000), 2.0);
        assert!((f16_to_f32(0x3555) - 0.333_251).abs() < 1e-5);
        // 65504 is the format's finite maximum, which the shading pass clamps
        // just below — it has to decode as a number, not an infinity.
        assert_eq!(f16_to_f32(0x7BFF), 65504.0);
    }

    #[test]
    fn f16_subnormals_are_not_flushed_to_zero() {
        // The smallest subnormal, 2^-24. A dark surface's GTAO difference can
        // live entirely in this range, and flushing it would report "no change"
        // for a change that is really there.
        assert!(f16_to_f32(0x0001) > 0.0);
        assert!((f16_to_f32(0x0001) - 5.960_464_5e-8).abs() < 1e-14);
    }

    #[test]
    fn the_diff_separates_terrain_from_everything_else() {
        // Two pixels of terrain, two of something else; only the terrain pair
        // changes. This is exactly the shape of the 25A acceptance test, and
        // the 25A-1 failure was that the terrain column stayed at zero.
        let before = frame(
            &[1.0, 1.0, 4.0, 900.0],
            &[PIXEL_TERRAIN, PIXEL_TERRAIN, PIXEL_MESH, PIXEL_SKY],
        );
        let after = frame(
            &[0.5, 0.75, 4.0, 900.0],
            &[PIXEL_TERRAIN, PIXEL_TERRAIN, PIXEL_MESH, PIXEL_SKY],
        );

        let t = compare(&before, &after, PIXEL_TERRAIN);
        assert_eq!(t.pixels, 2);
        assert_eq!(t.changed, 2);
        assert!((t.mean_abs - 0.375).abs() < 1e-6);

        let m = compare(&before, &after, PIXEL_MESH);
        assert_eq!(m.pixels, 1);
        assert_eq!(m.changed, 0);
        assert_eq!(m.mean_abs, 0.0);

        // Sky is its own class so a bright, unchanging background cannot dilute
        // the mesh column into looking unchanged.
        assert_eq!(compare(&before, &after, PIXEL_SKY).pixels, 1);
    }

    #[test]
    fn the_changed_threshold_is_relative_to_the_pixels_own_brightness() {
        // A 1% move counts at any exposure. An absolute threshold would call a
        // bright pixel changed and an identical relative move on a dark one
        // unchanged, which is how a real difference gets dismissed as noise.
        let before = frame(&[1000.0, 0.001], &[PIXEL_TERRAIN, PIXEL_TERRAIN]);
        let after = frame(&[1020.0, 0.00102], &[PIXEL_TERRAIN, PIXEL_TERRAIN]);
        assert_eq!(compare(&before, &after, PIXEL_TERRAIN).changed, 2);

        let unchanged = frame(&[1000.5, 0.001_004], &[PIXEL_TERRAIN, PIXEL_TERRAIN]);
        assert_eq!(compare(&before, &unchanged, PIXEL_TERRAIN).changed, 0);
    }

    #[test]
    fn a_capture_survives_a_round_trip() {
        let f = frame(&[0.25, 8.0, 0.0], &[PIXEL_TERRAIN, PIXEL_SKY, PIXEL_MESH]);
        let back = CapturedFrame::decode(&f.encode()).expect("decode");
        assert_eq!(back.width, 3);
        assert_eq!(back.height, 1);
        assert_eq!(back.rgb, f.rgb);
        assert_eq!(back.terrain, f.terrain);
    }

    #[test]
    fn a_foreign_file_is_refused_rather_than_misread() {
        assert!(CapturedFrame::decode(b"not a capture at all").is_err());
        let mut truncated = frame(&[1.0], &[PIXEL_TERRAIN]).encode();
        truncated.truncate(20);
        assert!(CapturedFrame::decode(&truncated).is_err());
    }
}
