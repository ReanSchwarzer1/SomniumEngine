//! The `.somnium` container — CONTROL-J.
//!
//! A scene file is a one-line text header followed by the JSON body:
//!
//! ```text
//! SOMNIUM-SCENE 1 <header-byte-length>
//! {"thumbnail_png_base64": "...", "saved_unix_secs": 1755993600, ...}
//! {"version": 3, "entities": [ ... ]}
//! ```
//!
//! ## Why a header rather than another JSON key
//!
//! §6.2.3 asks for a scene thumbnail the Content Drawer can read *without*
//! parsing the scene, and a key inside the document cannot deliver that: the
//! drawer would have to read and parse the whole file to reach it, which for a
//! large scene is exactly the stall CONTROL-C spent itself removing. A framed
//! header is a `seek` and a few hundred bytes.
//!
//! It is also never stale, which the alternative — a sidecar `.png` — is not.
//! The thumbnail is written by the same operation that writes the data, so
//! there is no state in which they disagree.
//!
//! ## Where this lives
//!
//! In `somnium_asset` beside `material.rs`, which owns `.sommat`'s header for
//! the same reason: this crate owns *file containers*, and the crate above
//! owns what the contents mean. It also has to be here rather than in
//! `somnium_core`, because the Content Drawer's preview generator reads the
//! thumbnail and the dependency edge runs core → asset.
//!
//! ## Compatibility
//!
//! A file that does not begin with the magic is read as a bare JSON document,
//! which is what every scene written before this phase is. Nothing needs
//! migrating and no existing file becomes unreadable.

use serde::{Deserialize, Serialize};

/// The first token of a framed scene file.
pub const SCENE_MAGIC: &str = "SOMNIUM-SCENE";

/// Container version. Distinct from the *document* version inside the body —
/// this numbers the framing, that numbers the schema.
pub const CONTAINER_VERSION: u32 = 1;

/// The most header bytes a reader will accept before giving up.
///
/// A 64×64 PNG in base64 is comfortably under 8 KiB, and refusing to read more
/// is what stops a corrupt length field turning a header read into a
/// whole-file read.
pub const MAX_HEADER_BYTES: usize = 64 * 1024;

/// What a scene file says about itself before the data starts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneHeader {
    /// Container version.
    #[serde(default)]
    pub container: u32,
    /// The document version inside the body, duplicated here so the drawer can
    /// tell a map recipe from a schema scene without parsing either.
    #[serde(default)]
    pub document_version: u64,
    /// Seconds since the Unix epoch at save time.
    #[serde(default)]
    pub saved_unix_secs: u64,
    /// How many entities the body holds, for the drawer's tooltip.
    #[serde(default)]
    pub entity_count: usize,
    /// A PNG of the viewport at save time, base64-encoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_png_base64: Option<String>,
}

impl SceneHeader {
    /// Decode the thumbnail, if there is a valid one.
    #[must_use]
    pub fn thumbnail_png(&self) -> Option<Vec<u8>> {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(self.thumbnail_png_base64.as_deref()?)
            .ok()
    }

    /// Attach a PNG thumbnail.
    pub fn set_thumbnail_png(&mut self, png: &[u8]) {
        use base64::Engine as _;
        self.thumbnail_png_base64 = Some(base64::engine::general_purpose::STANDARD.encode(png));
    }
}

/// Which of the three `.somnium` formats a document is.
///
/// The `version` field has always discriminated between *formats* rather than
/// revisions — see `scene_schema`'s module docs — and this is that distinction
/// made into a type, so the loader routes on a name instead of on a number
/// somebody has to remember.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneKind {
    /// Version 1: the hand-written entity dump.
    LegacyDump,
    /// Version 2: a map recipe — a factory `kind`, no entities.
    MapRecipe,
    /// Version 3: the schema-driven entity dump.
    Schema,
    /// A version this build does not read.
    Unsupported(u64),
}

impl SceneKind {
    /// Classify a parsed document.
    #[must_use]
    pub fn of(document: &serde_json::Value) -> Self {
        match document.get("version").and_then(serde_json::Value::as_u64) {
            Some(1) => Self::LegacyDump,
            Some(2) => Self::MapRecipe,
            Some(3) => Self::Schema,
            Some(other) => Self::Unsupported(other),
            None => Self::Unsupported(0),
        }
    }
}

/// Serialize a framed scene file.
///
/// Returned as bytes rather than written, so the framing is testable without
/// touching a filesystem.
#[must_use]
pub fn encode(header: &SceneHeader, body: &serde_json::Value) -> Vec<u8> {
    let header_json = serde_json::to_string(header).unwrap_or_else(|_| "{}".into());
    let body_text = serde_json::to_string_pretty(body).unwrap_or_else(|_| "{}".into());
    let mut out = Vec::with_capacity(header_json.len() + body_text.len() + 64);
    out.extend_from_slice(
        format!("{SCENE_MAGIC} {CONTAINER_VERSION} {}\n", header_json.len()).as_bytes(),
    );
    out.extend_from_slice(header_json.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(body_text.as_bytes());
    out
}

/// Split a framed file into its header and the body text.
///
/// A file with no magic is treated as a bare document — every scene written
/// before CONTROL-J is one — and reports `None` for the header rather than
/// failing.
///
/// # Errors
///
/// A message when the magic is present but the framing is not intact. That
/// case is a *truncated or corrupt* file, and reading the body out of it would
/// be a guess.
pub fn split(bytes: &[u8]) -> Result<(Option<SceneHeader>, &[u8]), String> {
    let Some(rest) = bytes.strip_prefix(SCENE_MAGIC.as_bytes()) else {
        return Ok((None, bytes));
    };
    let newline = rest
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or_else(|| "scene header line is not terminated".to_string())?;
    let line = std::str::from_utf8(&rest[..newline])
        .map_err(|_| "scene header line is not UTF-8".to_string())?;
    let mut parts = line.split_whitespace();
    let container: u32 = parts
        .next()
        .and_then(|token| token.parse().ok())
        .ok_or_else(|| "scene header has no container version".to_string())?;
    if container > CONTAINER_VERSION {
        return Err(format!(
            "scene container version {container} is newer than this build"
        ));
    }
    let length: usize = parts
        .next()
        .and_then(|token| token.parse().ok())
        .ok_or_else(|| "scene header has no length".to_string())?;
    if length > MAX_HEADER_BYTES {
        return Err(format!("scene header claims {length} bytes"));
    }
    let after_line = newline + 1;
    let header_bytes = rest
        .get(after_line..after_line + length)
        .ok_or_else(|| "scene header is truncated".to_string())?;
    let header: SceneHeader = serde_json::from_slice(header_bytes)
        .map_err(|error| format!("scene header is malformed: {error}"))?;
    // The body starts after the header and its terminating newline.
    let body = rest.get(after_line + length + 1..).unwrap_or(&[]);
    Ok((Some(header), body))
}

/// Read only the header. The drawer's route: a `seek` and a few hundred bytes,
/// never the whole scene.
///
/// # Errors
///
/// A message when the file cannot be read or its framing is corrupt. A file
/// with no header at all is `Ok(None)`, because that is an older scene rather
/// than a broken one.
pub fn read_header(path: &std::path::Path) -> Result<Option<SceneHeader>, String> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut buffer = vec![0u8; MAX_HEADER_BYTES.min(128 * 1024)];
    let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
    buffer.truncate(read);
    if !buffer.starts_with(SCENE_MAGIC.as_bytes()) {
        return Ok(None);
    }
    split(&buffer).map(|(header, _)| header)
}

/// Read a scene file whole: its header, if any, and its parsed body.
///
/// # Errors
///
/// A message when the file cannot be read, its framing is corrupt, or its body
/// is not JSON.
pub fn read(path: &std::path::Path) -> Result<(Option<SceneHeader>, serde_json::Value), String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let (header, body) = split(&bytes)?;
    let document: serde_json::Value =
        serde_json::from_slice(body).map_err(|error| error.to_string())?;
    Ok((header, document))
}

/// Write a framed scene file.
///
/// # Errors
///
/// A message when the file cannot be written.
pub fn write(
    path: &std::path::Path,
    header: &SceneHeader,
    body: &serde_json::Value,
) -> Result<(), String> {
    // `somnium_asset` is still on edition 2021, so this is the plain nested
    // form rather than a let-chain.
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
    }
    std::fs::write(path, encode(header, body)).map_err(|error| error.to_string())
}

/// Seconds since the Unix epoch, or zero if the clock is before it.
#[must_use]
pub fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> serde_json::Value {
        serde_json::json!({ "version": 3, "entities": [] })
    }

    fn header() -> SceneHeader {
        let mut header = SceneHeader {
            container: CONTAINER_VERSION,
            document_version: 3,
            saved_unix_secs: 1_755_993_600,
            entity_count: 12,
            thumbnail_png_base64: None,
        };
        header.set_thumbnail_png(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        header
    }

    #[test]
    fn a_framed_file_round_trips() {
        let encoded = encode(&header(), &body());
        let (read_header, body_bytes) = split(&encoded).expect("framing is intact");
        let read_header = read_header.expect("the header is present");
        assert_eq!(read_header, header());
        assert_eq!(
            read_header.thumbnail_png().unwrap(),
            vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
        );
        let document: serde_json::Value = serde_json::from_slice(body_bytes).unwrap();
        assert_eq!(document, body());
    }

    /// The point of the framing: the header is readable without the body.
    #[test]
    fn the_header_is_readable_without_parsing_the_body() {
        let mut huge = body();
        huge["entities"] = serde_json::Value::Array(
            (0..5_000)
                .map(|index| serde_json::json!({ "persistent_id": format!("{index:032x}") }))
                .collect(),
        );
        let encoded = encode(&header(), &huge);
        assert!(encoded.len() > 100_000, "the body is genuinely large");

        // Only the first few hundred bytes are needed.
        let prefix = &encoded[..2_000.min(encoded.len())];
        let (parsed, _) = split(prefix).expect("the header alone parses");
        assert_eq!(parsed.unwrap().entity_count, 12);
    }

    /// Every scene written before CONTROL-J is a bare JSON document, and must
    /// keep loading with no migration at all.
    #[test]
    fn an_unframed_file_is_read_as_a_bare_document() {
        let bare = serde_json::to_vec_pretty(&body()).unwrap();
        let (header, body_bytes) = split(&bare).expect("a bare document is not an error");
        assert_eq!(header, None);
        let document: serde_json::Value = serde_json::from_slice(body_bytes).unwrap();
        assert_eq!(document["version"], 3);
    }

    /// A truncated file is refused rather than half-read. Guessing at the body
    /// of a corrupt scene is how a bad save becomes a bad load.
    #[test]
    fn a_truncated_header_is_refused_rather_than_guessed_at() {
        let encoded = encode(&header(), &body());
        let cut = &encoded[..encoded.len().min(40)];
        assert!(split(cut).is_err());

        let mut lying = Vec::from(format!("{SCENE_MAGIC} 1 999999999\n").as_bytes());
        lying.extend_from_slice(b"{}");
        assert!(split(&lying).is_err(), "an absurd length is refused");
    }

    /// A container from a newer build is refused by name rather than
    /// misinterpreted.
    #[test]
    fn a_newer_container_is_refused() {
        let mut future = Vec::from(format!("{SCENE_MAGIC} 99 2\n").as_bytes());
        future.extend_from_slice(b"{}\n{}");
        let error = split(&future).expect_err("a newer container is refused");
        assert!(error.contains("99"), "{error}");
    }

    /// The three formats route by name, not by a number somebody remembers.
    #[test]
    fn the_three_formats_are_told_apart() {
        assert_eq!(
            SceneKind::of(&serde_json::json!({ "version": 1 })),
            SceneKind::LegacyDump
        );
        assert_eq!(
            SceneKind::of(&serde_json::json!({ "version": 2, "kind": "coastal" })),
            SceneKind::MapRecipe
        );
        assert_eq!(
            SceneKind::of(&serde_json::json!({ "version": 3, "entities": [] })),
            SceneKind::Schema
        );
        assert_eq!(
            SceneKind::of(&serde_json::json!({ "version": 9 })),
            SceneKind::Unsupported(9)
        );
        assert_eq!(
            SceneKind::of(&serde_json::json!({})),
            SceneKind::Unsupported(0)
        );
    }

    #[test]
    fn a_file_round_trips_through_the_filesystem() {
        let dir = std::env::temp_dir().join(format!("somnium_scene_file_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("scene.somnium");
        write(&path, &header(), &body()).expect("write");

        let only_header = read_header(&path).expect("read header").expect("present");
        assert_eq!(only_header.entity_count, 12);

        let (full_header, document) = read(&path).expect("read");
        assert_eq!(full_header, Some(header()));
        assert_eq!(document, body());
        let _ = std::fs::remove_dir_all(dir);
    }
}
