//! Phase 16-F: the bytecode cache.
//!
//! # Bytecode is a cache, never storage
//!
//! Luau's own `Bytecode.h` states that indefinite backward compatibility
//! is not provided, and the VM does **not** validate its input — it
//! assumes the bytes came from its own compiler. Handing it bytecode from
//! a different Luau is undefined behaviour, not a load error.
//!
//! So every cached artifact carries two things beside the bytes: the
//! runtime's fingerprint, and a hash of the source it came from. A
//! mismatch in either is not an error — it is a cache miss, and the answer
//! is to compile from source, which is always available. That is what
//! makes this safe to have at all.
//!
//! # What it buys, honestly
//!
//! Compilation was measured at **0.79 ms for a thousand lines**, 250×
//! inside its budget (`context.md` §17.18.3). This is
//! therefore not a frame-time optimisation and is not sold as one. It is
//! here because a shipped game should not have to carry a compiler pass on
//! every startup for every script, and because the plan asks for the
//! artifact to exist and to be fingerprinted before anything downstream
//! starts depending on it.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Magic at the head of every cooked file, so a truncated or unrelated
/// file is rejected before it is interpreted as bytecode.
const MAGIC: &[u8; 8] = b"SOMNLUAC";

/// One cooked script on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookedScript {
    /// The runtime that produced the bytes.
    pub fingerprint: String,
    /// A hash of the source text.
    pub source_hash: u64,
    /// The bytes.
    pub bytecode: Vec<u8>,
}

impl CookedScript {
    /// Serialise to the on-disk form.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let fingerprint = self.fingerprint.as_bytes();
        let mut out = Vec::with_capacity(32 + fingerprint.len() + self.bytecode.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.source_hash.to_le_bytes());
        out.extend_from_slice(&u32::try_from(fingerprint.len()).unwrap_or(0).to_le_bytes());
        out.extend_from_slice(fingerprint);
        out.extend_from_slice(&self.bytecode);
        out
    }

    /// Read the on-disk form.
    ///
    /// Returns `None` for anything that is not exactly what this build
    /// wrote — wrong magic, truncated, a length that does not fit. There
    /// is no partial recovery here on purpose: a half-understood header
    /// followed by bytes fed to a VM that does not validate them is the
    /// one failure mode this whole module exists to prevent.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 20 || &bytes[..8] != MAGIC {
            return None;
        }
        let source_hash = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
        let name_len = u32::from_le_bytes(bytes[16..20].try_into().ok()?) as usize;
        let start = 20_usize.checked_add(name_len)?;
        if bytes.len() < start {
            return None;
        }
        let fingerprint = std::str::from_utf8(&bytes[20..start]).ok()?.to_string();
        Some(Self {
            fingerprint,
            source_hash,
            bytecode: bytes[start..].to_vec(),
        })
    }

    /// Whether this artifact may be used for `source` on this runtime.
    #[must_use]
    pub fn is_valid_for(&self, source: &str, fingerprint: &str) -> bool {
        !self.bytecode.is_empty()
            && self.fingerprint == fingerprint
            && self.source_hash == hash_source(source)
    }
}

/// A stable hash of a script's text.
///
/// Line endings are normalised first: the same file checked out on
/// Windows and Linux is the same script, and a cache that recooked
/// everything on a platform change would be worse than no cache.
#[must_use]
pub fn hash_source(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes().filter(|b| *b != b'\r') {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Where cooked artifacts live.
///
/// Under `target/` because they are build output: they are derived from
/// files that are in version control, they are invalid on another
/// runtime, and nothing should ever be shipped from here without a
/// deliberate cook step that re-validates them.
#[must_use]
pub fn cache_dir() -> PathBuf {
    std::env::var_os("SOMNIUM_SCRIPT_CACHE").map_or_else(
        || {
            std::env::current_dir()
                .unwrap_or_default()
                .join("target")
                .join("script-cache")
        },
        PathBuf::from,
    )
}

/// The cache file for one script.
#[must_use]
pub fn cache_path(dir: &Path, asset: somnium_script::ids::ScriptAssetId) -> PathBuf {
    dir.join(format!("{asset}.luauc"))
}

/// Write an artifact, atomically enough that a killed process cannot
/// leave a half-file the next run would read.
///
/// # Errors
///
/// Whatever the filesystem said.
pub fn write_cooked(path: &Path, cooked: &CookedScript) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("luauc.tmp");
    {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&cooked.encode())?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)
}

/// Read an artifact, if there is a usable one.
#[must_use]
pub fn read_cooked(path: &Path) -> Option<CookedScript> {
    CookedScript::decode(&std::fs::read(path).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_script::ids::ScriptAssetId;

    fn cooked() -> CookedScript {
        CookedScript {
            fingerprint: "somnium-luau-1/0123456789abcdef".into(),
            source_hash: hash_source("return 1"),
            bytecode: vec![1, 2, 3, 4, 5],
        }
    }

    #[test]
    fn an_artifact_round_trips() {
        let original = cooked();
        assert_eq!(CookedScript::decode(&original.encode()), Some(original));
    }

    #[test]
    fn an_artifact_is_only_valid_for_the_runtime_that_made_it() {
        let artifact = cooked();
        assert!(artifact.is_valid_for("return 1", &artifact.fingerprint));
        assert!(
            !artifact.is_valid_for("return 1", "somnium-luau-1/ffffffffffffffff"),
            "a different runtime must be a cache miss, not a load"
        );
    }

    #[test]
    fn an_edited_source_invalidates_its_artifact() {
        let artifact = cooked();
        assert!(!artifact.is_valid_for("return 2", &artifact.fingerprint));
    }

    #[test]
    fn line_endings_do_not_invalidate_a_cache() {
        assert_eq!(
            hash_source("a\r\nb\r\n"),
            hash_source("a\nb\n"),
            "the same file checked out on two platforms is the same script"
        );
    }

    #[test]
    fn garbage_is_refused_rather_than_partly_believed() {
        assert_eq!(CookedScript::decode(b""), None);
        assert_eq!(CookedScript::decode(b"not a somnium artifact"), None);
        // Right magic, truncated body.
        let mut truncated = cooked().encode();
        truncated.truncate(12);
        assert_eq!(CookedScript::decode(&truncated), None);
        // Right magic, a length field that runs off the end.
        let mut lying = cooked().encode();
        lying[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(CookedScript::decode(&lying), None);
    }

    #[test]
    fn an_empty_artifact_is_never_valid() {
        let empty = CookedScript {
            bytecode: Vec::new(),
            ..cooked()
        };
        assert!(!empty.is_valid_for("return 1", &empty.fingerprint));
    }

    #[test]
    fn writing_and_reading_go_through_the_filesystem() {
        let dir = std::env::temp_dir().join(format!("somnium_cook_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = cache_path(&dir, ScriptAssetId::from_path("scripts/a.luau"));
        let artifact = cooked();
        write_cooked(&path, &artifact).unwrap();
        assert_eq!(read_cooked(&path), Some(artifact));
        assert_eq!(read_cooked(&dir.join("missing.luauc")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
