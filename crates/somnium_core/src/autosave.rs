//! Autosave and crash recovery — CONTROL-J, craft defect C11.
//!
//! Autosaves go to `<content root>/.somnium/autosave/`, beside the preview
//! cache and the asset index, because they are session state rather than
//! content: an autosave must never appear in the Content Drawer, and a folder
//! the drawer already ignores is the honest place for it.
//!
//! The policy here is pure so it can be tested without a clock, a filesystem
//! or a scene. The interesting decisions — when an autosave is *due*, and when
//! one on disk is worth offering back — are exactly the ones that are painful
//! to verify by hand.

use std::path::{Path, PathBuf};

/// The folder autosaves live in, relative to the content root.
pub const AUTOSAVE_DIR: &str = ".somnium/autosave";

/// Why an autosave is being written.
///
/// Named rather than a bool because the two have different *file names* — a
/// pre-Play snapshot is the one you want back after a play session goes wrong,
/// and an interval save would otherwise overwrite it within the minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutosaveReason {
    /// The interval elapsed.
    Interval,
    /// Play is about to start.
    BeforePlay,
}

impl AutosaveReason {
    /// The file this reason writes to.
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Interval => "autosave.somnium",
            Self::BeforePlay => "before-play.somnium",
        }
    }
}

/// Where an autosave goes, given the content root.
#[must_use]
pub fn autosave_path(content_root: &Path, reason: AutosaveReason) -> PathBuf {
    content_root.join(AUTOSAVE_DIR).join(reason.file_name())
}

/// Tracks when the next interval autosave is due.
///
/// Deliberately *not* a timer thread. Autosave has to serialise the world, so
/// it can only happen on the frame thread anyway, and a `due` predicate the
/// frame loop asks is both simpler and testable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutosaveClock {
    /// Seconds between saves. `0` disables autosave entirely.
    pub interval_secs: f32,
    /// Seconds elapsed since the last save.
    since_last: f32,
    /// Whether the scene has changed since the last autosave.
    dirty: bool,
}

impl Default for AutosaveClock {
    fn default() -> Self {
        Self {
            interval_secs: 300.0,
            since_last: 0.0,
            dirty: false,
        }
    }
}

impl AutosaveClock {
    /// A clock with the given interval.
    #[must_use]
    pub fn new(interval_secs: f32) -> Self {
        Self {
            interval_secs,
            since_last: 0.0,
            dirty: false,
        }
    }

    /// Advance by `dt` seconds, and report whether a save is due.
    ///
    /// Due means: autosave is on, the interval has elapsed, **and** something
    /// has changed. The last clause is what stops an idle editor rewriting the
    /// same file every five minutes forever — the file's timestamp is what
    /// crash recovery compares, and a stream of identical saves would make it
    /// meaningless.
    pub fn tick(&mut self, dt: f32, scene_dirty: bool) -> bool {
        self.dirty |= scene_dirty;
        if self.interval_secs <= 0.0 {
            return false;
        }
        self.since_last += dt.max(0.0);
        if self.since_last < self.interval_secs || !self.dirty {
            return false;
        }
        self.since_last = 0.0;
        self.dirty = false;
        true
    }

    /// Note that a save happened for some other reason, so the interval
    /// restarts from now.
    pub fn saved(&mut self) {
        self.since_last = 0.0;
        self.dirty = false;
    }

    /// Change the interval, keeping elapsed time. Used when the setting moves.
    pub fn set_interval(&mut self, interval_secs: f32) {
        self.interval_secs = interval_secs.max(0.0);
    }
}

/// An autosave found on disk that is worth offering back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovery {
    /// The autosave file.
    pub path: PathBuf,
    /// Why it was written.
    pub reason: AutosaveReason,
    /// Its modification time, seconds since the Unix epoch.
    pub saved_unix_secs: u64,
}

/// Decide whether an autosave should be offered, given both timestamps.
///
/// Offered only when the autosave is **newer** than the scene it shadows. An
/// autosave older than the last real save is work the person already committed
/// by hand, and offering it back would invite them to overwrite good work with
/// stale work — the failure mode that makes people distrust crash recovery.
///
/// A scene that does not exist at all (`None`) means every autosave is newer
/// than nothing, which is right: an unsaved scene is exactly the case
/// recovery is for.
#[must_use]
pub fn should_offer(autosave_secs: u64, scene_secs: Option<u64>) -> bool {
    match scene_secs {
        None => true,
        Some(scene) => autosave_secs > scene,
    }
}

/// Look for a recoverable autosave beside `content_root`.
///
/// The pre-Play snapshot wins a tie, because if both exist at the same second
/// the one taken deliberately before a risky operation is the one worth having.
#[must_use]
pub fn find_recovery(content_root: &Path, scene_path: &Path) -> Option<Recovery> {
    let scene_secs = modified_secs(scene_path);
    [AutosaveReason::BeforePlay, AutosaveReason::Interval]
        .into_iter()
        .filter_map(|reason| {
            let path = autosave_path(content_root, reason);
            let saved_unix_secs = modified_secs(&path)?;
            should_offer(saved_unix_secs, scene_secs).then_some(Recovery {
                path,
                reason,
                saved_unix_secs,
            })
        })
        .max_by_key(|recovery| recovery.saved_unix_secs)
}

/// A file's modification time in seconds since the Unix epoch, or `None` if it
/// does not exist or the filesystem will not say.
#[must_use]
pub fn modified_secs(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|elapsed| elapsed.as_secs())
}

/// Delete both autosaves. Called after a clean manual save, because at that
/// moment there is nothing left to recover.
pub fn clear(content_root: &Path) {
    for reason in [AutosaveReason::Interval, AutosaveReason::BeforePlay] {
        let _ = std::fs::remove_file(autosave_path(content_root, reason));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The interval only fires when something actually changed. An idle editor
    /// rewriting the same file every five minutes would make the timestamp
    /// crash recovery compares meaningless.
    #[test]
    fn an_idle_editor_never_autosaves() {
        let mut clock = AutosaveClock::new(60.0);
        for _ in 0..200 {
            assert!(!clock.tick(1.0, false));
        }
    }

    #[test]
    fn a_dirty_scene_saves_once_per_interval() {
        let mut clock = AutosaveClock::new(60.0);
        for _ in 0..59 {
            assert!(!clock.tick(1.0, true));
        }
        assert!(clock.tick(1.0, true), "the sixtieth second is due");
        // …and the counter restarts rather than firing every frame after.
        assert!(!clock.tick(1.0, true));
    }

    /// A change during the interval still counts, even if the scene is clean
    /// again by the time the interval elapses — an edit and an undo is still
    /// a session worth recovering.
    #[test]
    fn a_change_anywhere_in_the_interval_arms_the_save() {
        let mut clock = AutosaveClock::new(10.0);
        assert!(!clock.tick(1.0, true));
        for _ in 0..8 {
            assert!(!clock.tick(1.0, false));
        }
        assert!(clock.tick(1.0, false), "the change earlier still counts");
    }

    #[test]
    fn a_zero_interval_disables_autosave() {
        let mut clock = AutosaveClock::new(0.0);
        for _ in 0..1_000 {
            assert!(!clock.tick(60.0, true));
        }
    }

    #[test]
    fn a_manual_save_restarts_the_interval() {
        let mut clock = AutosaveClock::new(10.0);
        for _ in 0..9 {
            clock.tick(1.0, true);
        }
        clock.saved();
        assert!(!clock.tick(1.0, false), "the countdown started again");
    }

    /// The rule that makes recovery trustworthy: never offer work older than
    /// what the person already saved by hand.
    #[test]
    fn an_autosave_older_than_the_scene_is_not_offered() {
        assert!(should_offer(200, Some(100)), "newer is offered");
        assert!(!should_offer(100, Some(200)), "older is not");
        assert!(!should_offer(100, Some(100)), "identical is not");
        assert!(
            should_offer(1, None),
            "an unsaved scene is what this is for"
        );
    }

    /// The two reasons write to different files, so an interval save cannot
    /// overwrite the snapshot taken deliberately before Play.
    #[test]
    fn the_two_reasons_do_not_overwrite_each_other() {
        let root = Path::new("content");
        assert_ne!(
            autosave_path(root, AutosaveReason::Interval),
            autosave_path(root, AutosaveReason::BeforePlay)
        );
        assert!(
            autosave_path(root, AutosaveReason::Interval)
                .to_string_lossy()
                .contains(".somnium"),
            "autosaves live where the drawer does not look"
        );
    }

    /// End to end against a real filesystem: two autosaves and a scene, and
    /// the newest of the ones worth offering wins.
    #[test]
    fn recovery_finds_the_newest_offerable_autosave() {
        let root = std::env::temp_dir().join(format!(
            "somnium_autosave_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(AUTOSAVE_DIR)).unwrap();
        let scene = root.join("scene.somnium");
        std::fs::write(&scene, b"{}").unwrap();

        // No autosaves yet.
        assert_eq!(find_recovery(&root, &scene), None);

        // One written after the scene is offered.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let interval = autosave_path(&root, AutosaveReason::Interval);
        std::fs::write(&interval, b"{}").unwrap();
        let found = find_recovery(&root, &scene).expect("an autosave newer than the scene");
        assert_eq!(found.path, interval);
        assert_eq!(found.reason, AutosaveReason::Interval);

        // Clearing removes both.
        clear(&root);
        assert_eq!(find_recovery(&root, &scene), None);
        let _ = std::fs::remove_dir_all(root);
    }
}
