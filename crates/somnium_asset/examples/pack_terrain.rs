//! Phase 25K — channel-pack the Poly Haven terrain sources the engine loads.
//!
//! Run after `tools/fetch_terrain_textures.sh`:
//!
//! ```text
//! cargo run --release -p somnium_asset --example pack_terrain
//! ```
//!
//! Four source maps per material become **two** textures:
//!
//! | output          | R        | G        | B         | A      |
//! |-----------------|----------|----------|-----------|--------|
//! | `*_albedo.png`  | albedo R | albedo G | albedo B  | height |
//! | `*_surface.png` | normal X | normal Y | roughness | AO     |
//!
//! Two reasons, and the second is the one that matters. Memory: a 4K RGBA8
//! texture with mips is ~89 MB, so four maps per material across eight
//! materials is nearly 3 GB and two is half of that. And **sample count**: the
//! terrain shader samples every layer for every pixel, and Phase 25F triples
//! whatever it samples — halving the taps per layer is the difference between
//! hex-tiling being affordable and not.
//!
//! Normal Z is dropped and reconstructed in the shader as
//! `sqrt(1 - x² - y²)`, which is exact for a unit normal and is what BC5
//! compression would force anyway. Metalness is dropped from `arm`: terrain
//! layers are dielectric, and the engine's terrain path already hardcodes
//! `metallic = 0`.

use image::{GenericImageView, ImageBuffer, Rgba};
use std::path::{Path, PathBuf};

/// Materials to pack. Must match `tools/fetch_terrain_textures.sh`.
const MATERIALS: [&str; 8] = [
    "aerial_grass_rock",
    "leafy_grass",
    "forrest_ground_01",
    "brown_mud",
    "aerial_rocks_04",
    "snow_02",
    "coast_sand_rocks_02",
    "gravel_floor",
];

fn load(dir: &Path, material: &str, map: &str, res: &str) -> Result<image::DynamicImage, String> {
    let path = dir.join(format!("{material}_{map}_{res}.jpg"));
    image::open(&path).map_err(|e| format!("{}: {e}", path.display()))
}

fn main() -> Result<(), String> {
    let res = std::env::args().nth(1).unwrap_or_else(|| "4k".to_string());
    let source = PathBuf::from("assets/terrain/_source");
    let out_dir = PathBuf::from("assets/terrain");
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;

    for material in MATERIALS {
        let diff = load(&source, material, "diff", &res)?;
        let nor = load(&source, material, "nor_dx", &res)?;
        let arm = load(&source, material, "arm", &res)?;
        let disp = load(&source, material, "disp", &res)?;

        let (w, h) = diff.dimensions();
        // Poly Haven ships every map of a set at the same resolution, but the
        // displacement map is sometimes authored smaller. Resizing to the
        // albedo's size keeps the packing a straight per-texel operation.
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
        for y in 0..h {
            for x in 0..w {
                let d = diff.get_pixel(x, y).0;
                let n = nor.get_pixel(x, y).0;
                let a = arm.get_pixel(x, y).0;
                // Displacement is greyscale; any channel will do.
                let height = disp.get_pixel(x, y).0[0];

                albedo.put_pixel(x, y, Rgba([d[0], d[1], d[2], height]));
                // arm = ambient occlusion (R), roughness (G), metalness (B).
                surface.put_pixel(x, y, Rgba([n[0], n[1], a[1], a[0]]));
            }
        }

        let albedo_path = out_dir.join(format!("{material}_albedo.png"));
        let surface_path = out_dir.join(format!("{material}_surface.png"));
        albedo.save(&albedo_path).map_err(|e| e.to_string())?;
        surface.save(&surface_path).map_err(|e| e.to_string())?;
        println!("packed {material} ({w}x{h})");
    }

    println!("\n{} materials written to {}", MATERIALS.len(), out_dir.display());
    Ok(())
}
