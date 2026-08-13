//! Encode packed terrain PNGs (and procedural slots) to BC7 mip chains.
//!
//! Semantic mips are built first (`terrain::mips`), then each level is
//! compressed with Intel ISPC Texture Compressor via `intel_tex_2`. Height and
//! AO live in alpha, so the encoder uses the alpha-aware BC7 settings.
//!
//! ```text
//! cargo run --release -p somnium_renderer --example encode_terrain_bc7
//! cargo run --release -p somnium_renderer --example encode_terrain_bc7 -- --force
//! cargo run --release -p somnium_renderer --example encode_terrain_bc7 -- --fast
//! ```
//!
//! Output is `assets/terrain/bc7/{material}_{albedo|surface}.bc7` (gitignored).
//! Hero layers 0–15 encode at 2048; extra 16–31 at 1024. Existing files are
//! skipped unless `--force`. The runtime loads these when
//! `TEXTURE_COMPRESSION_BC` is present; `SOMNIUM_TERRAIN_FORCE_RGBA8=1` keeps
//! the RGBA8 path for A/B.

use intel_tex_2::bc7;
use somnium_renderer::terrain::mips::{PackedKind, build_mip_chain};
use somnium_renderer::terrain::textures::{
    LAYER_MATERIALS, TERRAIN_HERO_LAYERS, layer_packed_rgba,
};
use std::fs;
use std::path::Path;
use std::time::Instant;

const OUT_DIR: &str = "assets/terrain/bc7";

fn is_flag(arg: &str) -> bool {
    arg == "--force" || arg == "--fast"
}

fn pad_bc7_rgba(data: &[u8], w: u32, h: u32) -> (u32, u32, Vec<u8>) {
    let nw = w.max(4).div_ceil(4) * 4;
    let nh = h.max(4).div_ceil(4) * 4;
    if nw == w && nh == h {
        return (w, h, data.to_vec());
    }
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        let sy = y.min(h - 1);
        for x in 0..nw {
            let sx = x.min(w - 1);
            let src = ((sy * w + sx) * 4) as usize;
            let dst = ((y * nw + x) * 4) as usize;
            out[dst..dst + 4].copy_from_slice(&data[src..src + 4]);
        }
    }
    (nw, nh, out)
}

fn compress_mip(rgba: &[u8], w: u32, h: u32, settings: &bc7::EncodeSettings) -> Vec<u8> {
    let (pw, ph, padded) = pad_bc7_rgba(rgba, w, h);
    let surface = intel_tex_2::RgbaSurface {
        data: &padded,
        width: pw,
        height: ph,
        stride: pw * 4,
    };
    bc7::compress_blocks(settings, &surface)
}

fn encode_map(rgba: &[u8], size: u32, kind: PackedKind, settings: &bc7::EncodeSettings) -> Vec<u8> {
    let mut out = Vec::new();
    for (w, h, mip) in build_mip_chain(rgba, size, size, kind) {
        out.extend(compress_mip(&mip, w, h, settings));
    }
    out
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let force = args.iter().any(|a| a == "--force");
    let fast = args.iter().any(|a| a == "--fast");
    for arg in args.iter().skip(1) {
        if !is_flag(arg) {
            return Err(format!(
                "unknown argument {arg} (expected --force / --fast)"
            ));
        }
    }

    let settings = if fast {
        println!("BC7 settings: alpha_fast");
        bc7::alpha_fast_settings()
    } else {
        println!("BC7 settings: alpha_basic (height/AO in alpha)");
        bc7::alpha_basic_settings()
    };

    fs::create_dir_all(OUT_DIR).map_err(|e| format!("{OUT_DIR}: {e}"))?;
    let started = Instant::now();
    let mut encoded = 0u32;
    let mut skipped = 0u32;
    let mut photographed = 0u32;

    for (i, material) in LAYER_MATERIALS.iter().enumerate() {
        let size = if (i as u32) < TERRAIN_HERO_LAYERS {
            2048
        } else {
            1024
        };
        let albedo_path = format!("{OUT_DIR}/{material}_albedo.bc7");
        let surface_path = format!("{OUT_DIR}/{material}_surface.bc7");
        if !force && Path::new(&albedo_path).is_file() && Path::new(&surface_path).is_file() {
            println!("skip {i:02} {material} ({size}) — exists");
            skipped += 1;
            continue;
        }

        let layer_t = Instant::now();
        let (albedo, surface, from_png) = layer_packed_rgba(i, size);
        if from_png {
            photographed += 1;
        }
        let albedo_bc7 = encode_map(&albedo, size, PackedKind::AlbedoHeight, &settings);
        let surface_bc7 = encode_map(&surface, size, PackedKind::Surface, &settings);
        fs::write(&albedo_path, &albedo_bc7).map_err(|e| format!("{albedo_path}: {e}"))?;
        fs::write(&surface_path, &surface_bc7).map_err(|e| format!("{surface_path}: {e}"))?;
        encoded += 1;
        println!(
            "enc  {i:02} {material} {size} albedo={} KiB surface={} KiB {} ({:.1}s)",
            albedo_bc7.len() / 1024,
            surface_bc7.len() / 1024,
            if from_png { "png" } else { "procedural" },
            layer_t.elapsed().as_secs_f32()
        );
    }

    println!(
        "done: encoded {encoded}, skipped {skipped}, photographed-this-run {photographed}/32 in {:.1}s → {OUT_DIR}",
        started.elapsed().as_secs_f32()
    );
    Ok(())
}
