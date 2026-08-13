//! Phase XV-B — channel-pack the Poly Haven terrain sources the engine loads.
//!
//! Run after `cargo run -p somnium_asset --example fetch_terrain`:
//!
//! ```text
//! cargo run --release -p somnium_asset --example pack_terrain -- 2k
//! cargo run --release -p somnium_asset --example pack_terrain -- 2k --force
//! cargo run -p somnium_asset --example pack_terrain -- --validate-only
//! ```
//!
//! Existing `*_albedo.png` + `*_surface.png` pairs are skipped unless `--force`
//! is passed, so a 2K pack of layers 8–15 will not replace shipping 4K 0–7.
//!
//! Photographed materials from `assets/terrain/materials.json` become **two**
//! textures each. Procedural palette slots (currently 16 and 24) are omitted
//! from `layers` and are not packed:
//!
//! | output          | R        | G        | B         | A      |
//! |-----------------|----------|----------|-----------|--------|
//! | `*_albedo.png`  | albedo R | albedo G | albedo B  | height |
//! | `*_surface.png` | normal X | normal Y | roughness | AO     |
//!
//! Normal Z is reconstructed in the shader. Metalness is dropped: terrain
//! layers are dielectric. Semantic mips (linear albedo, renormalized normals,
//! Toksvig roughness) are generated at load time, not stored in these PNGs.
//!
//! BC7 packs are encoded separately (semantic mips, then ISPC BC7):
//!
//! ```text
//! cargo run --release -p somnium_renderer --example encode_terrain_bc7
//! ```
//!
//! The runtime detects `TEXTURE_COMPRESSION_BC` and loads
//! `assets/terrain/bc7/*.bc7` when every layer is present; otherwise it stays
//! on RGBA8 and never keeps both. `SOMNIUM_TERRAIN_FORCE_RGBA8=1` forces the
//! fallback for A/B.

use image::{GenericImageView, ImageBuffer, Rgba};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const MANIFEST: &str = "assets/terrain/materials.json";

fn load(dir: &Path, material: &str, map: &str, res: &str) -> Result<image::DynamicImage, String> {
    let path = dir.join(format!("{material}_{map}_{res}.jpg"));
    image::open(&path).map_err(|e| format!("{}: {e}", path.display()))
}

fn layer_ids(manifest: &Value) -> Result<Vec<String>, String> {
    let layers = manifest
        .get("layers")
        .and_then(Value::as_array)
        .ok_or("manifest missing layers")?;
    let mut ids = Vec::with_capacity(layers.len());
    for layer in layers {
        let id = layer
            .get("id")
            .and_then(Value::as_str)
            .ok_or("layer missing id")?;
        ids.push(id.to_string());
    }
    let palette = manifest
        .get("layer_count")
        .and_then(Value::as_u64)
        .unwrap_or(32);
    if ids.is_empty() {
        return Err("manifest layers empty".into());
    }
    if ids.len() as u64 > palette {
        return Err(format!(
            "layers has {} ids but layer_count is {palette}",
            ids.len()
        ));
    }
    Ok(ids)
}

fn is_flag(arg: &str) -> bool {
    arg == "--force" || arg == "--validate-only"
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let res = args
        .iter()
        .skip(1)
        .find(|a| !is_flag(a))
        .cloned()
        .unwrap_or_else(|| "4k".to_string());
    let force = args.iter().any(|a| a == "--force");
    let validate_only = args.iter().any(|a| a == "--validate-only");
    let source = PathBuf::from("assets/terrain/_source");
    let out_dir = PathBuf::from("assets/terrain");
    if !validate_only {
        fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    }

    let text = fs::read_to_string(MANIFEST).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let manifest: Value = serde_json::from_str(&text).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let materials = layer_ids(&manifest)?;
    let maps = manifest
        .get("packer_maps")
        .and_then(Value::as_array)
        .ok_or("manifest missing packer_maps")?;
    for required in ["diff", "nor_dx", "arm", "disp"] {
        if !maps.iter().any(|m| m.as_str() == Some(required)) {
            return Err(format!("packer_maps missing {required}"));
        }
    }

    let mut packed = Vec::new();
    let mut missing = Vec::new();
    for material in &materials {
        let albedo_path = out_dir.join(format!("{material}_albedo.png"));
        let surface_path = out_dir.join(format!("{material}_surface.png"));
        if validate_only {
            match (albedo_path.exists(), surface_path.exists()) {
                (true, true) => {
                    let (w, h) = image::image_dimensions(&albedo_path).unwrap_or((0, 0));
                    println!("ok {material} ({w}x{h})");
                    packed.push(serde_json::json!({
                        "id": material,
                        "skipped": true,
                        "width": w,
                        "height": h,
                        "albedo": albedo_path.display().to_string(),
                        "surface": surface_path.display().to_string(),
                    }));
                }
                (a, s) => {
                    missing.push(format!(
                        "{material}: albedo={} surface={}",
                        if a { "present" } else { "missing" },
                        if s { "present" } else { "missing" }
                    ));
                }
            }
            continue;
        }
        if !force && albedo_path.exists() && surface_path.exists() {
            let (w, h) = image::image_dimensions(&albedo_path).unwrap_or((0, 0));
            println!("skip {material} (already packed, {w}x{h})");
            packed.push(serde_json::json!({
                "id": material,
                "skipped": true,
                "width": w,
                "height": h,
                "albedo": albedo_path.display().to_string(),
                "surface": surface_path.display().to_string(),
            }));
            continue;
        }
        let diff = load(&source, material, "diff", &res)?;
        let nor = load(&source, material, "nor_dx", &res)?;
        let arm = load(&source, material, "arm", &res)?;
        let disp = load(&source, material, "disp", &res)?;

        let (w, h) = diff.dimensions();
        let fit = |img: image::DynamicImage| -> image::DynamicImage {
            if img.dimensions() == (w, h) {
                img
            } else {
                img.resize_exact(w, h, image::imageops::FilterType::Lanczos3)
            }
        };
        let nor = fit(nor).to_rgba8();
        let arm = fit(arm).to_rgba8();
        let disp = fit(disp).to_rgba8();
        let diff = diff.to_rgba8();

        let mut albedo = ImageBuffer::<Rgba<u8>, Vec<u8>>::new(w, h);
        let mut surface = ImageBuffer::<Rgba<u8>, Vec<u8>>::new(w, h);
        let mut n_min = 1.0f32;
        let mut n_max = 0.0f32;
        let mut n_clamped = 0u32;
        for y in 0..h {
            for x in 0..w {
                let d = diff.get_pixel(x, y).0;
                let n = nor.get_pixel(x, y).0;
                let a = arm.get_pixel(x, y).0;
                let height = disp.get_pixel(x, y).0[0];
                albedo.put_pixel(x, y, Rgba([d[0], d[1], d[2], height]));
                let mut nx = f32::from(n[0]) / 255.0 * 2.0 - 1.0;
                let mut ny = f32::from(n[1]) / 255.0 * 2.0 - 1.0;
                let len = (nx * nx + ny * ny).sqrt();
                n_min = n_min.min(len);
                n_max = n_max.max(len);
                if len > 1.0 {
                    nx /= len;
                    ny /= len;
                    n_clamped += 1;
                }
                let nx8 = ((nx * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
                let ny8 = ((ny * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
                surface.put_pixel(x, y, Rgba([nx8, ny8, a[1], a[0]]));
            }
        }
        if n_max > 1.5 {
            return Err(format!(
                "{material}: packed XY normal length reached {n_max:.3} (source looks corrupt)"
            ));
        }
        if n_clamped > 0 {
            println!(
                "warn {material}: {n_clamped} texels had XY length > 1 (max {n_max:.3}); clamped to unit disk"
            );
        }

        albedo.save(&albedo_path).map_err(|e| e.to_string())?;
        surface.save(&surface_path).map_err(|e| e.to_string())?;
        println!("packed {material} ({w}x{h})");
        packed.push(serde_json::json!({
            "id": material,
            "width": w,
            "height": h,
            "albedo": albedo_path.display().to_string(),
            "surface": surface_path.display().to_string(),
            "xy_normal_length_min": n_min,
            "xy_normal_length_max": n_max,
            "xy_clamped_texels": n_clamped,
        }));
    }

    if validate_only {
        let palette = manifest
            .get("layer_count")
            .and_then(Value::as_u64)
            .unwrap_or(32);
        let runtime_rgba8_mib = 16.0 * 2.0 * 2048.0 * 2048.0 * 4.0 * (4.0 / 3.0)
            / (1024.0 * 1024.0)
            + 16.0 * 2.0 * 1024.0 * 1024.0 * 4.0 * (4.0 / 3.0) / (1024.0 * 1024.0);
        let dropped_mib = 32.0 * 2.0 * 1024.0 * 1024.0 * 4.0 * (4.0 / 3.0) / (1024.0 * 1024.0);
        println!(
            "validate-only: {} photographed layers, palette {palette}, {} missing",
            packed.len(),
            missing.len()
        );
        println!(
            "projected RGBA8 residency: hero2k+extra1k = {runtime_rgba8_mib:.0} MiB (over 700); runtime drop-to-1k = {dropped_mib:.0} MiB"
        );
        if !missing.is_empty() {
            return Err(format!(
                "pack_terrain --validate-only failed:\n  {}",
                missing.join("\n  ")
            ));
        }
        return Ok(());
    }

    let size = packed
        .first()
        .and_then(|v| v.get("width"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let photographed = packed.len() as f32;
    let rgba8_mib_2k = photographed * 2.0 * 2048.0 * 2048.0 * 4.0 * (4.0 / 3.0) / (1024.0 * 1024.0);
    let bc7_mib_2k = rgba8_mib_2k / 4.0;
    let runtime_rgba8_mib = 16.0 * 2.0 * 2048.0 * 2048.0 * 4.0 * (4.0 / 3.0) / (1024.0 * 1024.0)
        + 16.0 * 2.0 * 1024.0 * 1024.0 * 4.0 * (4.0 / 3.0) / (1024.0 * 1024.0);
    let report = serde_json::json!({
        "manifest": MANIFEST,
        "resolution_arg": res,
        "packed_edge": size,
        "photographed_layers": packed.len(),
        "layer_count": packed.len(),
        "layout": {
            "albedo": "sRGB RGB + linear height A",
            "surface": "DirectX normal XY + roughness + AO",
        },
        "mips": "generated at load (linear albedo, renormalized normals, Toksvig roughness)",
        "bc7": {
            "encoded": false,
            "encoder": "somnium_renderer example encode_terrain_bc7",
            "reason": "PNG packer does not emit BC7; run encode_terrain_bc7. Runtime loads assets/terrain/bc7/*.bc7 when complete, else RGBA8, never both",
            "estimated_2k_rgba8_mib": rgba8_mib_2k,
            "estimated_2k_bc7_mib": bc7_mib_2k,
            "estimated_runtime_hero2k_extra1k_rgba8_mib": runtime_rgba8_mib,
            "budgets_mib": { "bc7_2k": 200, "rgba8_2k": 700 },
        },
        "layers": packed,
    });
    let report_path = out_dir.join("pack_report.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&report).unwrap() + "\n",
    )
    .map_err(|e| e.to_string())?;

    println!(
        "\n{} materials written to {} (report {})",
        materials.len(),
        out_dir.display(),
        report_path.display()
    );
    Ok(())
}
