//! Audio errors (MORROWIND-AG).
//!
//! **This file used to be one line: `// Error stub`.**
//!
//! The errors themselves already lived in `engine.rs`; this module is where
//! they belong, and it re-exports them so the crate reads the way its module
//! list promised.

pub use crate::engine::AudioError;

/// Whether a failure is worth interrupting the player over.
///
/// Audio fails in two very different ways and treating them alike is how a
/// missing footstep becomes a crash:
///
/// - **A missing or corrupt file** is a content bug. The right response is a
///   log line and silence for that one sound, because a game that will not
///   start because one footstep is missing is worse than one that is quiet.
/// - **No audio device** is an environment fact, not a bug — a headless CI
///   machine, a container, a player with no sound card. Everything else must
///   keep working.
///
/// Neither is fatal. This function exists so that judgement is written once
/// rather than re-made at each call site, and so nobody reaches for `unwrap`.
#[must_use]
pub fn is_fatal(_error: &AudioError) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **No audio failure stops the game.**
    ///
    /// A headless CI machine has no audio device, and a missing sound file is a
    /// content bug. Neither is a reason to refuse to run.
    #[test]
    fn no_audio_error_is_fatal() {
        let error = AudioError::PlayError("no device".into());
        assert!(!is_fatal(&error));
        let missing = AudioError::MissingFile("footstep.ogg".into());
        assert!(!is_fatal(&missing));
    }

    #[test]
    fn errors_say_what_failed() {
        let error = AudioError::MissingFile("assets/footstep.ogg".into());
        assert!(error.to_string().contains("assets/footstep.ogg"));
    }
}
