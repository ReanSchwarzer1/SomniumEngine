//! Block-compressed imported textures: BC7 mip chains and their cache on disk.
//!
//! # Why
//!
//! Imported glTF maps were uploaded as RGBA8 with a full mip chain: 4 bytes a
//! texel, 22 MB for a 2K map. The Town (TSF) reached ~10.9 GB of them on a
//! 16 GB card, so the driver paged textures in and out mid-frame and passes
//! that only read depth (Hi-Z) took 56 ms. BC7 is 1 byte a texel, a quarter,
//! at quality close to the source for colour, packed data (ARM) and normals.
//!
//! Normal maps are BC7 too, not BC5: BC5 keeps only X and Y, so Z would be
//! rebuilt at unit length, and the shading pass reads the *length* of the
//! mip-filtered normal for its Toksvig specular anti-aliasing. BC7 keeps it.
//!
//! # The cache
//!
//! Encoding is slow next to an upload, so each texture is encoded once and
//! kept in [`cache_dir`] under its content key (the SHA-256 the importer already
//! dedupes by: size, colour flag, pixels), with the encoder settings in the
//! name. The cache lives outside the project on purpose: gigabytes of derived
//! data must not sync with a cloud folder or reach version control. A file
//! that fails any check is ignored and re-encoded.
//!
//! `SOMNIUM_TEXTURE_FORCE_RGBA8=1` keeps the old path for A/B comparisons;
//! `SOMNIUM_TEXTURE_CACHE` moves the cache.
use rayon::prelude::*;
use std::path::PathBuf;

const MAGIC: &[u8; 4] = b"SBC7";
/// Bump when the encoding or the file layout changes; old files are then ignored.
const VERSION: u32 = 1;

/// Whether imported textures are compressed on this device.
pub fn enabled(features: wgpu::Features) -> bool {
    features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
        && std::env::var("SOMNIUM_TEXTURE_FORCE_RGBA8").map_or(true, |v| v != "1")
}

/// BC textures need a base size in whole 4×4 blocks; anything else stays RGBA8.
pub fn compressible(width: u32, height: u32) -> bool {
    width >= 4 && height >= 4 && width % 4 == 0 && height % 4 == 0
}

/// Mip levels of a full chain down to 1×1, as `build_mip_chain` makes it.
pub fn level_count(width: u32, height: u32) -> usize {
    32 - width.max(height).max(1).leading_zeros() as usize
}

/// Bytes of one BC7 level: whole 4×4 blocks of 16 bytes, small levels padded up.
pub fn level_bytes(width: u32, height: u32) -> usize {
    (width.max(1).div_ceil(4) * height.max(1).div_ceil(4) * 16) as usize
}

/// `SOMNIUM_TEXTURE_CACHE`, else `%LOCALAPPDATA%/Somnium/texture-cache`, else
/// `target/texture-cache` under the working directory.
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("SOMNIUM_TEXTURE_CACHE") {
        return PathBuf::from(dir);
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("Somnium").join("texture-cache");
    }
    std::env::current_dir().unwrap_or_default().join("target").join("texture-cache")
}

fn cache_path(key: &[u8; 32], alpha: bool) -> PathBuf {
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    cache_dir().join(format!("{hex}.{}.v{VERSION}.bc7", if alpha { "a" } else { "o" }))
}

/// Whether any texel is not fully opaque (selects the alpha-aware BC7 settings).
pub fn has_alpha(rgba: &[u8]) -> bool {
    rgba.chunks_exact(4).any(|texel| texel[3] != 255)
}

/// Pad a level to whole blocks (at least one) by repeating its last row and column.
fn pad_to_blocks(rgba: &[u8], width: u32, height: u32) -> (u32, u32, std::borrow::Cow<'_, [u8]>) {
    let pw = width.max(1).div_ceil(4) * 4;
    let ph = height.max(1).div_ceil(4) * 4;
    if pw == width && ph == height {
        return (width, height, std::borrow::Cow::Borrowed(rgba));
    }
    let mut out = vec![0u8; (pw * ph * 4) as usize];
    for y in 0..ph {
        let sy = y.min(height - 1);
        for x in 0..pw {
            let sx = x.min(width - 1);
            let src = ((sy * width + sx) * 4) as usize;
            let dst = ((y * pw + x) * 4) as usize;
            out[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
        }
    }
    (pw, ph, std::borrow::Cow::Owned(out))
}

/// BC7 blocks of one RGBA8 level. Bands of block rows are independent in BC7
/// and land in row-major block order, so they are encoded in parallel and joined.
fn encode_level(rgba: &[u8], width: u32, height: u32, alpha: bool) -> Vec<u8> {
    let settings = if alpha {
        intel_tex_2::bc7::alpha_basic_settings()
    } else {
        intel_tex_2::bc7::opaque_basic_settings()
    };
    let (pw, ph, padded) = pad_to_blocks(rgba, width, height);
    const BAND_ROWS: u32 = 64; // 16 block rows a band
    let bands: Vec<u32> = (0..ph).step_by(BAND_ROWS as usize).collect();
    let parts: Vec<Vec<u8>> = bands
        .par_iter()
        .map(|&y0| {
            let rows = BAND_ROWS.min(ph - y0);
            let start = (y0 * pw * 4) as usize;
            let surface = intel_tex_2::RgbaSurface {
                data: &padded[start..start + (rows * pw * 4) as usize],
                width: pw,
                height: rows,
                stride: pw * 4,
            };
            intel_tex_2::bc7::compress_blocks(&settings, &surface)
        })
        .collect();
    parts.concat()
}

/// BC7 for every level of a mip chain (`(width, height, rgba8)` per level).
pub fn encode(levels: &[(u32, u32, Vec<u8>)], alpha: bool) -> Vec<Vec<u8>> {
    levels.iter().map(|(w, h, data)| encode_level(data, *w, *h, alpha)).collect()
}

/// Whether a chain for this texture is on disk (not read or checked).
pub fn is_cached(key: &[u8; 32], alpha: bool) -> bool {
    cache_path(key, alpha).is_file()
}

/// A cached chain for this texture, if one was stored and checks out.
pub fn load(key: &[u8; 32], width: u32, height: u32, alpha: bool) -> Option<Vec<Vec<u8>>> {
    let bytes = std::fs::read(cache_path(key, alpha)).ok()?;
    parse(&bytes, width, height)
}

fn parse(bytes: &[u8], width: u32, height: u32) -> Option<Vec<Vec<u8>>> {
    let word = |at: usize| -> Option<u32> { Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?)) };
    if bytes.get(0..4)? != MAGIC || word(4)? != VERSION || word(8)? != width || word(12)? != height {
        return None;
    }
    let count = word(16)? as usize;
    if count != level_count(width, height) {
        return None;
    }
    let mut at = 20;
    let mut levels = Vec::with_capacity(count);
    let (mut w, mut h) = (width, height);
    for _ in 0..count {
        let len = level_bytes(w, h);
        levels.push(bytes.get(at..at + len)?.to_vec());
        at += len;
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    (at == bytes.len()).then_some(levels)
}

fn serialize(width: u32, height: u32, levels: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(20 + levels.iter().map(Vec::len).sum::<usize>());
    out.extend_from_slice(MAGIC);
    for word in [VERSION, width, height, levels.len() as u32] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    for level in levels {
        out.extend_from_slice(level);
    }
    out
}

/// Keep a chain for next time. Written beside and renamed, so a reader never
/// sees half a file; a failure only means it is encoded again.
pub fn store(key: &[u8; 32], width: u32, height: u32, alpha: bool, levels: &[Vec<u8>]) {
    let path = cache_path(key, alpha);
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let temp = path.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&temp, serialize(width, height, levels)).is_ok() && std::fs::rename(&temp, &path).is_err() {
        let _ = std::fs::remove_file(&temp);
    }
}

/// What to fill the cache with before a level opens: each glTF source (relative to the content root) and the parts
/// of it that levels use (`None`: every part). Built by [`plan_warm`], encoded by
/// `SomniumRenderer::warm_texture_cache`; `examples/warm_texture_cache.rs` and a game's warm-up screen share it.
pub type WarmPlan = std::collections::BTreeMap<String, Option<std::collections::HashSet<usize>>>;

fn read_json(path: &std::path::Path) -> Result<serde_json::Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // `SOMNIUM-SCENE 1 <len>`, a header line, then the document; plain JSON otherwise.
    let body = if text.starts_with("SOMNIUM-SCENE") { text.splitn(3, '\n').nth(2).unwrap_or("") } else { &text };
    serde_json::from_str(body).map_err(|e| format!("{}: {e}", path.display()))
}

fn add_to_plan(plan: &mut WarmPlan, source: String, parts: Option<std::collections::HashSet<usize>>) {
    let entry = plan.entry(source).or_insert_with(|| Some(Default::default()));
    match (entry.as_mut(), parts) {
        (Some(have), Some(more)) => have.extend(more),
        _ => *entry = None,
    }
}

/// Add what uploading `item` would encode: for a scene (`*.json` with entities), the parts its imported meshes use;
/// for a foliage palette (`*palette.json`), every material of each entry's model and its far LOD; for a model
/// (`*.gltf` / `*.glb`) or a folder of them, every material. Paths inside scenes and palettes are relative to `root`.
pub fn plan_warm(root: &std::path::Path, item: &std::path::Path, plan: &mut WarmPlan) -> Result<(), String> {
    let relative = |path: &std::path::Path| path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/");
    if item.is_dir() {
        for entry in std::fs::read_dir(item).map_err(|e| format!("{}: {e}", item.display()))?.flatten() {
            let path = entry.path();
            if path.is_dir() || path.extension().is_some_and(|e| e == "gltf" || e == "glb") {
                plan_warm(root, &path, plan)?;
            }
        }
        return Ok(());
    }
    match item.extension().and_then(|e| e.to_str()) {
        Some("gltf" | "glb") => add_to_plan(plan, relative(item), None),
        Some("json") => {
            let document = read_json(item)?;
            for entry in document["entries"].as_array().into_iter().flatten() {
                for source in [&entry["source"], &entry["lod"]["source"]] {
                    if let Some(source) = source.as_str() {
                        add_to_plan(plan, source.to_owned(), None);
                    }
                }
            }
            for entity in document["entities"].as_array().into_iter().flatten() {
                let fields = &entity["components"]["somnium.ImportedMesh"]["fields"];
                if let (Some(source), Some(node)) = (fields["source"].as_str(), fields["node"].as_u64()) {
                    add_to_plan(plan, source.to_owned(), Some(std::collections::HashSet::from([node as usize])));
                }
            }
        }
        _ => return Err(format!("{}: not a scene, palette, model or folder", item.display())),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32, alpha: u8) -> Vec<u8> {
        (0..w * h).flat_map(|i| [(i % w * 255 / w) as u8, (i / w * 255 / h) as u8, 90, alpha]).collect()
    }

    #[test]
    fn a_chain_has_one_level_per_halving_and_whole_blocks_at_every_level() {
        assert_eq!(level_count(2048, 2048), 12);
        assert_eq!(level_count(1024, 256), 11);
        assert_eq!(level_count(1, 1), 1);
        assert_eq!(level_bytes(2048, 2048), 512 * 512 * 16);
        assert_eq!(level_bytes(2, 1), 16, "a 2×1 level is still one padded block");
        assert!(compressible(1024, 768) && !compressible(670, 736) && !compressible(2, 2));
    }

    #[test]
    fn encoded_levels_are_the_sizes_the_upload_expects_and_survive_the_cache_file() {
        let (w, h) = (64, 32);
        let mut levels = vec![(w, h, gradient(w, h, 255))];
        while levels.last().map_or(false, |(lw, lh, _)| *lw > 1 || *lh > 1) {
            let (lw, lh, _) = levels.last().unwrap();
            let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
            levels.push((nw, nh, gradient(nw, nh, 255)));
        }
        assert_eq!(levels.len(), level_count(w, h));
        let blocks = encode(&levels, false);
        for ((lw, lh, _), b) in levels.iter().zip(&blocks) {
            assert_eq!(b.len(), level_bytes(*lw, *lh), "{lw}×{lh}");
        }
        let file = serialize(w, h, &blocks);
        assert_eq!(parse(&file, w, h).as_deref(), Some(&blocks[..]));
        assert!(parse(&file, w * 2, h).is_none(), "another size is not this texture");
        assert!(parse(&file[..file.len() - 1], w, h).is_none(), "a cut file is rejected");
    }

    #[test]
    fn bands_join_into_the_same_blocks_as_one_pass() {
        // 200 rows: more than one 64-row band and not a multiple of it.
        let (w, h) = (64, 200);
        let rgba = gradient(w, h, 255);
        let whole = intel_tex_2::bc7::compress_blocks(
            &intel_tex_2::bc7::opaque_basic_settings(),
            &intel_tex_2::RgbaSurface { data: &rgba, width: w, height: h, stride: w * 4 },
        );
        assert_eq!(encode_level(&rgba, w, h, false), whole);
    }

    #[test]
    fn alpha_is_seen_only_where_a_texel_is_not_opaque() {
        assert!(!has_alpha(&gradient(8, 8, 255)));
        assert!(has_alpha(&gradient(8, 8, 254)));
    }
}

#[cfg(test)]
mod timing {
    /// `SOMNIUM_BC_SAMPLE=<2K png/jpg> cargo test -p somnium_renderer --lib imported_bc::timing -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn encode_time_of_one_real_map() {
        let Some(path) = std::env::var_os("SOMNIUM_BC_SAMPLE") else { return };
        let image = image::open(path).unwrap().into_rgba8();
        let (w, h) = image.dimensions();
        let rgba = image.into_raw();
        let start = std::time::Instant::now();
        let mut levels = vec![(w, h, rgba)];
        while levels.last().is_some_and(|(lw, lh, _)| *lw > 1 || *lh > 1) {
            let (lw, lh, data) = levels.last().unwrap();
            let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
            let next = (0..nw * nh).flat_map(|i| {
                let (x, y) = ((i % nw * 2).min(lw - 1), (i / nw * 2).min(lh - 1));
                let at = ((y * lw + x) * 4) as usize;
                [data[at], data[at + 1], data[at + 2], data[at + 3]]
            }).collect();
            levels.push((nw, nh, next));
        }
        let mips = start.elapsed();
        let start = std::time::Instant::now();
        let blocks = super::encode(&levels, false);
        println!("{w}x{h}: mips {mips:?}, BC7 encode {:?}, {} MB", start.elapsed(), blocks.iter().map(Vec::len).sum::<usize>() >> 20);
    }
}
