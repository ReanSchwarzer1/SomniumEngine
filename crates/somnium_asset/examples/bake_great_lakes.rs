//! Deterministic Phase IV bake for Motion Forge Pictures' Great Lakes asset.
//!
//! Usage:
//! `cargo run -p somnium_asset --example bake_great_lakes -- <source-dir> <output-dir>`

use image::{DynamicImage, ImageBuffer, Luma, Rgba};
use std::{
    fs,
    path::{Path, PathBuf},
};

const SOURCE_SIZE: u32 = 2048;
const HEIGHT_SIZE: u32 = 1025;
const WATER_SIZE: u32 = 2048;
const MACRO_SIZE: u32 = 512;
const PLATEAU: f32 = 0.040_073_775;
const PLATEAU_EPSILON: f32 = 0.000_1;
const SOURCE_MIN: f32 = 0.039_686_963;
const SOURCE_MAX: f32 = 0.221_651_301;
const WATER_LEVEL_M: f32 = 15.0;
const MAX_DEPTH_M: f32 = 12.0;
const LAND_RELIEF_M: f32 = 90.0;
const TOTAL_HEIGHT_M: f32 = WATER_LEVEL_M + LAND_RELIEF_M;
// 72 world metres at the 0.5 m source-texel scale.
const SHORE_WIDTH_PX: f32 = 144.0;

fn scalar_exr(path: &Path) -> Result<(u32, u32, Vec<f32>), String> {
    use exr::prelude::*;
    let image = read_first_flat_layer_from_file(path).map_err(|e| e.to_string())?;
    let layer = image.layer_data;
    let channel = layer
        .channel_data
        .list
        .iter()
        .find(|channel| channel.name.eq_case_insensitive("Y"))
        .or_else(|| layer.channel_data.list.first())
        .ok_or("EXR contains no flat channel")?;
    Ok((
        layer.size.0 as u32,
        layer.size.1 as u32,
        channel.sample_data.values_as_f32().collect(),
    ))
}

fn rgb_image(image: DynamicImage) -> (u32, u32, Vec<[f32; 3]>) {
    let (w, h) = (image.width(), image.height());
    let values = match image {
        DynamicImage::ImageRgb32F(img) => img.pixels().map(|p| [p[0], p[1], p[2]]).collect(),
        DynamicImage::ImageRgba32F(img) => img.pixels().map(|p| [p[0], p[1], p[2]]).collect(),
        other => other
            .to_rgb8()
            .pixels()
            .map(|p| {
                [
                    p[0] as f32 / 255.0,
                    p[1] as f32 / 255.0,
                    p[2] as f32 / 255.0,
                ]
            })
            .collect(),
    };
    (w, h, values)
}

fn box_scalar(src: &[f32], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<f32> {
    let mut out = vec![0.0; (dw * dh) as usize];
    for y in 0..dh {
        let cy = y as f32 * (sh - 1) as f32 / (dh - 1).max(1) as f32;
        let sy0 = (cy - sh as f32 / dh as f32 * 0.5).round().max(0.0) as u32;
        let sy1 = (cy + sh as f32 / dh as f32 * 0.5)
            .round()
            .min((sh - 1) as f32) as u32;
        for x in 0..dw {
            let cx = x as f32 * (sw - 1) as f32 / (dw - 1).max(1) as f32;
            let sx0 = (cx - sw as f32 / dw as f32 * 0.5).round().max(0.0) as u32;
            let sx1 = (cx + sw as f32 / dw as f32 * 0.5)
                .round()
                .min((sw - 1) as f32) as u32;
            let mut sum = 0.0;
            let mut count = 0;
            for sy in sy0..=sy1 {
                for sx in sx0..=sx1 {
                    sum += src[(sy * sw + sx) as usize];
                    count += 1;
                }
            }
            out[(y * dw + x) as usize] = sum / count as f32;
        }
    }
    out
}

fn cell_mask(src: &[f32], sw: u32) -> Vec<u8> {
    let mut mask = vec![0; (WATER_SIZE * WATER_SIZE) as usize];
    for y in 0..WATER_SIZE {
        for x in 0..WATER_SIZE {
            let sx0 = x * sw / WATER_SIZE;
            let sx1 = ((x + 1) * sw / WATER_SIZE).max(sx0 + 1).min(sw);
            let sy0 = y * sw / WATER_SIZE;
            let sy1 = ((y + 1) * sw / WATER_SIZE).max(sy0 + 1).min(sw);
            let mut hits = 0;
            let mut samples = 0;
            for sy in sy0..sy1 {
                for sx in sx0..sx1 {
                    let h = src[(sy * sw + sx) as usize];
                    hits += u32::from((h - PLATEAU).abs() <= PLATEAU_EPSILON);
                    samples += 1;
                }
            }
            mask[(y * WATER_SIZE + x) as usize] = if hits * 4 >= samples * 3 { 255 } else { 0 };
        }
    }
    mask
}

fn chamfer_distance(mask: &[u8], inside: bool) -> Vec<f32> {
    let n = WATER_SIZE as usize;
    let mut d: Vec<f32> = vec![1.0e9; n * n];
    for i in 0..d.len() {
        if (mask[i] != 0) != inside {
            d[i] = 0.0;
        }
    }
    let diag = std::f32::consts::SQRT_2;
    for y in 0..n {
        for x in 0..n {
            let i = y * n + x;
            if x > 0 {
                d[i] = d[i].min(d[i - 1] + 1.0);
            }
            if y > 0 {
                d[i] = d[i].min(d[i - n] + 1.0);
            }
            if x > 0 && y > 0 {
                d[i] = d[i].min(d[i - n - 1] + diag);
            }
            if x + 1 < n && y > 0 {
                d[i] = d[i].min(d[i - n + 1] + diag);
            }
        }
    }
    for y in (0..n).rev() {
        for x in (0..n).rev() {
            let i = y * n + x;
            if x + 1 < n {
                d[i] = d[i].min(d[i + 1] + 1.0);
            }
            if y + 1 < n {
                d[i] = d[i].min(d[i + n] + 1.0);
            }
            if x + 1 < n && y + 1 < n {
                d[i] = d[i].min(d[i + n + 1] + diag);
            }
            if x > 0 && y + 1 < n {
                d[i] = d[i].min(d[i + n - 1] + diag);
            }
        }
    }
    d
}

fn srgb(linear: f32) -> u8 {
    let x = linear.max(0.0);
    let y = if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    (y.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn save_u16(path: &Path, size: u32, values: impl Iterator<Item = u16>) -> Result<(), String> {
    let image: ImageBuffer<Luma<u16>, Vec<u16>> =
        ImageBuffer::from_vec(size, size, values.collect())
            .ok_or_else(|| format!("bad image length for {}", path.display()))?;
    image
        .save(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn main() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let source = PathBuf::from(args.next().ok_or("missing source directory")?);
    let output = PathBuf::from(args.next().ok_or("missing output directory")?);
    fs::create_dir_all(&output).map_err(|e| e.to_string())?;

    let (hw, hh, source_height) = scalar_exr(&source.join("Height Map.exr"))?;
    let (dw, dh, diffuse) =
        rgb_image(image::open(source.join("Diffuse Map.exr")).map_err(|e| e.to_string())?);
    if (hw, hh, dw, dh) != (SOURCE_SIZE, SOURCE_SIZE, SOURCE_SIZE, SOURCE_SIZE) {
        return Err(format!(
            "unexpected source dimensions: height {hw}x{hh}, diffuse {dw}x{dh}"
        ));
    }
    if source_height.iter().any(|h| !h.is_finite()) {
        return Err("non-finite source height".into());
    }
    let found_min = source_height.iter().copied().fold(f32::INFINITY, f32::min);
    let found_max = source_height
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    if (found_min - SOURCE_MIN).abs() > 1e-6 || (found_max - SOURCE_MAX).abs() > 1e-6 {
        return Err(format!("source range changed: {found_min}..{found_max}"));
    }

    let mask = cell_mask(&source_height, hw);
    let inside = chamfer_distance(&mask, true);
    let outside = chamfer_distance(&mask, false);
    let depth: Vec<f32> = inside
        .iter()
        .enumerate()
        .map(|(i, d)| {
            if mask[i] == 0 {
                0.0
            } else {
                let t = (d / SHORE_WIDTH_PX).clamp(0.0, 1.0);
                (t * t * (3.0 - 2.0 * t)) * MAX_DEPTH_M
            }
        })
        .collect();

    let resampled = box_scalar(&source_height, hw, hh, HEIGHT_SIZE, HEIGHT_SIZE);
    let mut metres = vec![0.0; resampled.len()];
    for y in 0..HEIGHT_SIZE {
        for x in 0..HEIGHT_SIZE {
            let cx = x * (WATER_SIZE - 1) / (HEIGHT_SIZE - 1);
            let cy = y * (WATER_SIZE - 1) / (HEIGHT_SIZE - 1);
            let ci = (cy * WATER_SIZE + cx) as usize;
            let i = (y * HEIGHT_SIZE + x) as usize;
            metres[i] = if mask[ci] != 0 {
                WATER_LEVEL_M - depth[ci]
            } else {
                let dry = ((resampled[i] - PLATEAU) / (SOURCE_MAX - PLATEAU)).clamp(0.0, 1.0);
                WATER_LEVEL_M + 0.35 + dry * (LAND_RELIEF_M - 0.35)
            };
        }
    }
    save_u16(
        &output.join("height.png"),
        HEIGHT_SIZE,
        metres.iter().map(|h| {
            (h.clamp(0.0, TOTAL_HEIGHT_M) / TOTAL_HEIGHT_M * u16::MAX as f32).round() as u16
        }),
    )?;
    ImageBuffer::<Luma<u8>, _>::from_vec(WATER_SIZE, WATER_SIZE, mask.clone())
        .ok_or("bad mask")?
        .save(output.join("water_mask.png"))
        .map_err(|e| e.to_string())?;
    save_u16(
        &output.join("water_depth.png"),
        WATER_SIZE,
        depth
            .iter()
            .map(|d| (d / MAX_DEPTH_M * u16::MAX as f32).round() as u16),
    )?;
    save_u16(
        &output.join("shore_sdf.png"),
        WATER_SIZE,
        inside
            .iter()
            .zip(&outside)
            .enumerate()
            .map(|(i, (din, dout))| {
                let signed = if mask[i] != 0 { *din } else { -*dout };
                (((signed / 128.0).clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32).round() as u16
            }),
    )?;

    let mut macro_pixels = Vec::with_capacity((MACRO_SIZE * MACRO_SIZE * 4) as usize);
    for y in 0..MACRO_SIZE {
        for x in 0..MACRO_SIZE {
            let mut c = [0.0; 3];
            for oy in 0..4 {
                for ox in 0..4 {
                    let p = diffuse[((y * 4 + oy) * SOURCE_SIZE + x * 4 + ox) as usize];
                    for k in 0..3 {
                        c[k] += p[k] / 16.0;
                    }
                }
            }
            let mx = x * WATER_SIZE / MACRO_SIZE;
            let my = y * WATER_SIZE / MACRO_SIZE;
            let water = mask[(my * WATER_SIZE + mx) as usize] != 0;
            macro_pixels.extend_from_slice(&if water {
                [128, 128, 128, 0]
            } else {
                [srgb(c[0]), srgb(c[1]), srgb(c[2]), 220]
            });
        }
    }
    ImageBuffer::<Rgba<u8>, _>::from_vec(MACRO_SIZE, MACRO_SIZE, macro_pixels)
        .ok_or("bad macro image")?
        .save(output.join("macro_color.png"))
        .map_err(|e| e.to_string())?;

    let wet = mask.iter().filter(|&&v| v != 0).count();
    let max_baked_depth = depth.iter().copied().fold(0.0, f32::max);
    fs::write(output.join("recipe.json"), format!(
        "{{\n  \"recipe_version\": 1,\n  \"source\": \"Motion Forge Pictures / Great Lakes\",\n  \"height_sha256\": \"d608ec2e62a40e38ff3a65180c6e017b14422496920a1f517d9aa691e2f252b9\",\n  \"diffuse_sha256\": \"45cc8c1e4a2698ff01de2a441e8ad2cf822bf4bae29dfb95dc9a7a20a38dce17\",\n  \"source_range\": [{source_min:.9}, {source_max:.9}],\n  \"plateau\": {plateau:.9},\n  \"plateau_epsilon\": {epsilon},\n  \"resample\": \"centred area box\",\n  \"terrain_size\": {height_size},\n  \"water_size\": {water_size},\n  \"water_level_metres\": {water_level},\n  \"max_depth_metres\": {max_depth},\n  \"wet_cells\": {wet},\n  \"wet_fraction\": {fraction:.8},\n  \"max_baked_depth_metres\": {max_baked_depth:.6}\n}}\n",
        source_min = SOURCE_MIN,
        source_max = SOURCE_MAX,
        plateau = PLATEAU,
        epsilon = PLATEAU_EPSILON,
        height_size = HEIGHT_SIZE,
        water_size = WATER_SIZE,
        water_level = WATER_LEVEL_M,
        max_depth = MAX_DEPTH_M,
        fraction = wet as f32 / mask.len() as f32,
    )).map_err(|e| e.to_string())?;
    println!("Great Lakes bake complete: {wet} wet cells, max depth {max_baked_depth:.3} m");
    Ok(())
}
