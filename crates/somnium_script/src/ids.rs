//! Identity types for script assets, attachments and live instances.
//!
//! Three separate ids, because they answer three different questions and
//! collapsing any two of them costs correctness later:
//!
//! * [`ScriptAssetId`] — *which file*. Survives everything.
//! * [`InstanceUuid`] — *which attachment on which entity*. Authored data,
//!   written into the scene, and the key that lets a hot reload put state
//!   back where it came from.
//! * [`ScriptInstanceId`] — *which live VM object*. Runtime only, never
//!   serialized, invalidated by every reload.
//!
//! None of these is exposed to script code as a number. Script-family
//! runtimes do not all have an exact 64-bit integer type, so an id that
//! round-tripped through a script's number type could come back changed.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use somnium_ecs::PersistentId;

/// Which language a script asset is written in.
///
/// The backend is chosen from this tag at load time, not from a compile
/// feature, so adding a second language is an addition rather than an
/// edit to everything that touches scripts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LanguageTag(&'static str);

impl LanguageTag {
    /// Luau, the Phase 16 primary language.
    pub const LUAU: Self = Self("luau");

    /// Declare a tag for a language.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// The tag as written in an asset header.
    #[inline]
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for LanguageTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// A content-stable identifier for a script asset.
///
/// Reuses the same 128-bit shape as [`PersistentId`] so that one id
/// generator serves both, and so a scene file's script references look
/// like its entity references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ScriptAssetId(u128);

impl ScriptAssetId {
    /// The id that means "no asset".
    pub const NONE: Self = Self(0);

    /// Mint a fresh asset id, for a newly created script.
    #[must_use]
    pub fn mint() -> Self {
        Self(PersistentId::mint().raw())
    }

    /// Rebuild an id read from a file.
    #[inline]
    #[must_use]
    pub const fn from_raw(raw: u128) -> Self {
        Self(raw)
    }

    /// The raw value, for serialization only.
    #[inline]
    #[must_use]
    pub const fn raw(self) -> u128 {
        self.0
    }

    /// Whether this references nothing.
    #[inline]
    #[must_use]
    pub const fn is_none(self) -> bool {
        self.0 == 0
    }

    /// Parse the 32-character hexadecimal form.
    #[must_use]
    pub fn parse_hex(text: &str) -> Option<Self> {
        u128::from_str_radix(text.trim(), 16).ok().map(Self)
    }

    /// The id of the script at a project-relative path.
    ///
    /// # Why the path and not the content
    ///
    /// "Content-hash-stable" is the obvious reading of the plan's phrase
    /// and the wrong one for *this* id: an attachment written into a scene
    /// names an asset, and hashing the file's bytes would give it a new
    /// name every time the author saved the script. Every attachment in
    /// every scene would break on the first edit.
    ///
    /// The path is what is stable across an edit, and it is what a
    /// human-readable scene diff should show a rename of. A content hash
    /// belongs on the *cook* side, where the question is "is this bytecode
    /// still valid", and 16-F is where that lives.
    ///
    /// Case and separators are normalised, so the same file resolves to
    /// the same id on Windows and Linux.
    #[must_use]
    pub fn from_path(path: &str) -> Self {
        // FNV-1a, widened to 128 bits. Not cryptographic and does not need
        // to be: this maps a project's own file names to ids, and a
        // collision would be a name collision.
        const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
        const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
        let mut hash = OFFSET;
        for byte in path
            .chars()
            .map(|c| if c == '\\' { '/' } else { c.to_ascii_lowercase() })
            .flat_map(|c| {
                let mut buffer = [0_u8; 4];
                let encoded = c.encode_utf8(&mut buffer).as_bytes().to_vec();
                encoded.into_iter()
            })
        {
            hash ^= u128::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
        // Zero is reserved for "no asset"; nudge the one input that would
        // produce it rather than letting a real file be unnameable.
        Self(if hash == 0 { 1 } else { hash })
    }
}

impl fmt::Display for ScriptAssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

/// The durable identity of one script attachment on one entity.
///
/// Minted when the attachment is created in the editor and written into
/// the scene. A hot reload destroys and rebuilds the VM object but keeps
/// this, which is how migrated state finds its way home.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct InstanceUuid(u128);

impl InstanceUuid {
    /// The id that means "no attachment".
    pub const NONE: Self = Self(0);

    /// Mint a fresh attachment id.
    #[must_use]
    pub fn mint() -> Self {
        Self(PersistentId::mint().raw())
    }

    /// Rebuild an id read from a file.
    #[inline]
    #[must_use]
    pub const fn from_raw(raw: u128) -> Self {
        Self(raw)
    }

    /// The raw value, for serialization only.
    #[inline]
    #[must_use]
    pub const fn raw(self) -> u128 {
        self.0
    }

    /// Whether this references nothing.
    #[inline]
    #[must_use]
    pub const fn is_none(self) -> bool {
        self.0 == 0
    }

    /// Parse the 32-character hexadecimal form.
    #[must_use]
    pub fn parse_hex(text: &str) -> Option<Self> {
        u128::from_str_radix(text.trim(), 16).ok().map(Self)
    }
}

impl fmt::Display for InstanceUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

/// A handle to a **live** script object inside a backend.
///
/// Runtime only. Never written to a file, never shown to script code, and
/// invalidated whenever the module it belongs to is reloaded. The backend
/// uses it to find its own bookkeeping; the engine uses it to say which
/// instance a call is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScriptInstanceId(u64);

impl ScriptInstanceId {
    /// Allocate the next id. Monotonic for the life of the process, so a
    /// stale id can never be confused with a live one.
    #[must_use]
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    /// Raw value, for backend-internal maps only.
    #[inline]
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ScriptInstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "instance#{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_through_their_text_form() {
        let asset = ScriptAssetId::mint();
        assert_eq!(ScriptAssetId::parse_hex(&asset.to_string()), Some(asset));
        let instance = InstanceUuid::mint();
        assert_eq!(InstanceUuid::parse_hex(&instance.to_string()), Some(instance));
    }

    #[test]
    fn none_ids_are_distinguishable() {
        assert!(ScriptAssetId::NONE.is_none());
        assert!(InstanceUuid::NONE.is_none());
        assert!(!ScriptAssetId::mint().is_none());
        assert!(!InstanceUuid::mint().is_none());
    }

    #[test]
    fn a_scripts_id_follows_its_path_and_survives_an_edit() {
        let a = ScriptAssetId::from_path("scripts/rotator.luau");
        assert_eq!(
            a,
            ScriptAssetId::from_path("scripts/rotator.luau"),
            "the same path must always give the same id, or every saved \
             attachment breaks on restart"
        );
        assert_eq!(
            a,
            ScriptAssetId::from_path("Scripts\\Rotator.luau"),
            "case and separators are normalised across platforms"
        );
        assert_ne!(a, ScriptAssetId::from_path("scripts/walker.luau"));
        assert!(!a.is_none());
    }

    #[test]
    fn live_instance_ids_are_monotonic() {
        let a = ScriptInstanceId::next();
        let b = ScriptInstanceId::next();
        assert!(a.raw() < b.raw());
    }
}
