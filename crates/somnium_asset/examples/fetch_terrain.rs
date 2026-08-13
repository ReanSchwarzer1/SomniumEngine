//! Phase XV-B — manifest-driven Poly Haven fetch.
//!
//! ```text
//! cargo run --release -p somnium_asset --example fetch_terrain -- 4k
//! ```
//!
//! Reads `assets/terrain/materials.json`. Fail-closed on license, missing map,
//! byte-size, or MD5 mismatch. Writes SHA-256 into
//! `assets/terrain/_source/FETCH_REPORT.json`. Does not rewrite the hand-authored
//! manifest (avoids scrambling key order). User-Agent:
//! `SomniumEngine-terrain-fetch/XV`.
//!
//! Rejected IDs (`terrain_red_01`, `dry_riverbed_rock`, `grass_path_2`,
//! `grass_path_3`) are never downloaded.

use md5::{Digest, Md5};
use serde_json::{Map, Value};
use sha2::Sha256;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

const UA: &str = "SomniumEngine-terrain-fetch/XV";
const MANIFEST: &str = "assets/terrain/materials.json";
const STAGING: &str = "assets/terrain/_source";
const MAX_RETRIES: u32 = 3;

fn hex_md5(bytes: &[u8]) -> String {
    let mut h = Md5::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

fn download(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    let mut last = String::new();
    for attempt in 1..=MAX_RETRIES {
        match agent.get(url).call() {
            Ok(resp) => {
                let mut buf = Vec::new();
                resp.into_reader()
                    .read_to_end(&mut buf)
                    .map_err(|e| format!("read {url}: {e}"))?;
                return Ok(buf);
            }
            Err(e) => {
                last = e.to_string();
                eprintln!("retry {attempt}/{MAX_RETRIES} {url}: {last}");
                thread::sleep(Duration::from_millis(400 * u64::from(attempt)));
            }
        }
    }
    Err(format!("{url}: {last}"))
}

fn file_entry<'a>(layer: &'a Value, res: &str, map: &str) -> Result<&'a Value, String> {
    layer
        .get("files")
        .and_then(|f| f.get(res))
        .and_then(|r| r.get(map))
        .ok_or_else(|| format!("manifest missing files.{res}.{map}"))
}

fn main() -> Result<(), String> {
    let res = std::env::args().nth(1).unwrap_or_else(|| "4k".to_string());
    if res != "2k" && res != "4k" {
        return Err("resolution must be 2k or 4k".into());
    }

    let text = fs::read_to_string(MANIFEST).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let manifest: Value = serde_json::from_str(&text).map_err(|e| format!("{MANIFEST}: {e}"))?;

    let ua = manifest
        .get("user_agent")
        .and_then(Value::as_str)
        .unwrap_or(UA);
    if ua != UA {
        return Err(format!("unexpected user_agent {ua:?}; expected {UA:?}"));
    }

    let rejected: Vec<String> = manifest
        .get("rejected_for_role")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|r| r.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let maps: Vec<String> = manifest
        .get("packer_maps")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .ok_or("manifest missing packer_maps")?;

    let staging = PathBuf::from(STAGING);
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;

    let agent = ureq::AgentBuilder::new()
        .user_agent(UA)
        .timeout(Duration::from_secs(300))
        .build();

    let mut report = Vec::new();
    let layers = manifest
        .get("layers")
        .and_then(Value::as_array)
        .ok_or("manifest missing layers")?;

    for layer in layers {
        let id = layer
            .get("id")
            .and_then(Value::as_str)
            .ok_or("layer missing id")?
            .to_string();
        if rejected.iter().any(|r| r == &id) {
            return Err(format!(
                "rejected id {id} must not appear in layers (use its substitute)"
            ));
        }
        let license = layer.get("license").and_then(Value::as_str).unwrap_or("");
        if license != "CC0-1.0" {
            return Err(format!(
                "{id}: license {license:?} is not CC0-1.0 (fail closed)"
            ));
        }

        for map in &maps {
            let meta = file_entry(layer, &res, map)?.clone();
            let url = meta
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{id}.{map}: missing url"))?;
            let expect_bytes = meta.get("bytes").and_then(Value::as_u64);
            let expect_md5 = meta
                .get("md5")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{id}.{map}: missing md5"))?;

            let name = format!("{id}_{map}_{res}.jpg");
            let out = staging.join(&name);
            let bytes = if out.is_file() {
                fs::read(&out).map_err(|e| format!("{}: {e}", out.display()))?
            } else {
                eprintln!("get   {name}");
                let bytes = download(&agent, url)?;
                fs::write(&out, &bytes).map_err(|e| format!("{}: {e}", out.display()))?;
                bytes
            };

            if let Some(n) = expect_bytes {
                if n != bytes.len() as u64 {
                    let _ = fs::remove_file(&out);
                    return Err(format!(
                        "{name}: size {} != manifest {n} (fail closed)",
                        bytes.len()
                    ));
                }
            }
            let got_md5 = hex_md5(&bytes);
            if !got_md5.eq_ignore_ascii_case(expect_md5) {
                let _ = fs::remove_file(&out);
                return Err(format!(
                    "{name}: md5 {got_md5} != manifest {expect_md5} (fail closed)"
                ));
            }
            let sha = hex_sha256(&bytes);
            let mut row = Map::new();
            row.insert("id".into(), Value::String(id.clone()));
            row.insert("map".into(), Value::String(map.clone()));
            row.insert("res".into(), Value::String(res.clone()));
            row.insert("bytes".into(), Value::from(bytes.len() as u64));
            row.insert("md5".into(), Value::String(got_md5));
            row.insert("sha256".into(), Value::String(sha));
            row.insert("path".into(), Value::String(out.display().to_string()));
            report.push(Value::Object(row));
            eprintln!("ok    {name}");
        }
    }

    let mut summary = Map::new();
    summary.insert("user_agent".into(), Value::String(UA.into()));
    summary.insert("resolution".into(), Value::String(res.clone()));
    summary.insert("staging".into(), Value::String(STAGING.into()));
    summary.insert("files".into(), Value::Array(report));
    let report_path = Path::new(STAGING).join("FETCH_REPORT.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&Value::Object(summary)).unwrap() + "\n",
    )
    .map_err(|e| format!("{}: {e}", report_path.display()))?;

    eprintln!("wrote {}", report_path.display());
    eprintln!("next: cargo run --release -p somnium_asset --example pack_terrain -- {res}");
    Ok(())
}
