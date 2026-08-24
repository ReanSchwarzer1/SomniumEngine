//! Source watching for hot reload.
//!
//! Seam 3: *"In debug builds a file watcher invalidates by module and
//! recompiles dependent variants. Hot shader reload is the highest-value
//! developer feature in the entire phase per line of code, and it falls out of
//! this seam for free."*
//!
//! # Why polling, and why that is not a compromise
//!
//! This is a modification-time poll over the registered file set, not an OS
//! notification API. `somnium_shader` depends on wgpu and nothing else
//! (plan §7.9), and the alternative is an inotify / `ReadDirectoryChangesW` /
//! FSEvents dependency tree for a feature that exists only in debug builds.
//!
//! The cost is fifty-odd `stat` calls. Run on a background job at a few hertz
//! that is unmeasurable, and it comes with a property the notification APIs do
//! not have for free: it cannot miss an edit, because it compares state rather
//! than consuming events.
//!
//! # The rule the plan is emphatic about
//!
//! > *"atomic pipeline swap, and a visible toast on failure with the naga
//! > diagnostic — **never a silent revert to the old pipeline**."*
//!
//! This module reports *what changed*; [`crate::ShaderSystem::apply_reload`]
//! owns the swap and the failure path. The important half of that rule lives
//! there, and Appendix A.7 makes it the specific check for this sub-phase:
//! introduce a deliberate WGSL syntax error, and the diagnostic must be shown
//! while **the old pipeline stays bound** — not a black screen, not a silent
//! revert with no message.

use std::{collections::HashMap, path::PathBuf, time::SystemTime};

use crate::compose::ModuleId;

/// A file a module was loaded from, and the stamp last seen on it.
struct Watched {
    path: PathBuf,
    stamp: Option<SystemTime>,
}

/// Modification-time poll over the registered module files.
#[derive(Default)]
pub struct SourceWatcher {
    watched: HashMap<ModuleId, Watched>,
}

impl SourceWatcher {
    /// Watch `path` as the source of `module`.
    ///
    /// A path that does not exist is still watched: a module whose file appears
    /// later — a new shader dropped in beside the others — should be picked up
    /// rather than needing a restart.
    pub fn watch(&mut self, module: ModuleId, path: PathBuf) {
        let stamp = stamp_of(&path);
        self.watched.insert(module, Watched { path, stamp });
    }

    /// How many files are watched.
    #[must_use]
    pub fn len(&self) -> usize {
        self.watched.len()
    }

    /// Whether nothing is watched.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.watched.is_empty()
    }

    /// The path a module is watched at.
    #[must_use]
    pub fn path(&self, module: ModuleId) -> Option<&std::path::Path> {
        self.watched.get(&module).map(|w| w.path.as_path())
    }

    /// Modules whose file changed since the last poll, with their new source.
    ///
    /// A file that vanished is **not** reported as a change. Editors write by
    /// rename often enough that a momentary absence is normal, and treating it
    /// as an edit would invalidate every dependent variant and recompile them
    /// from a file that is about to be replaced. The next poll sees the new
    /// stamp and reports it properly.
    ///
    /// A file that fails to *read* is likewise skipped rather than reported: on
    /// Windows a save can briefly hold the file open, and reading a partial
    /// write produces a naga error about code nobody wrote.
    pub fn poll(&mut self) -> Vec<(ModuleId, String)> {
        let mut changed = Vec::new();
        for (&module, watched) in &mut self.watched {
            let stamp = stamp_of(&watched.path);
            if stamp.is_none() || stamp == watched.stamp {
                continue;
            }
            match std::fs::read_to_string(&watched.path) {
                Ok(source) => {
                    watched.stamp = stamp;
                    changed.push((module, source));
                }
                // Leave the old stamp in place so the next poll retries.
                Err(_) => continue,
            }
        }
        // Deterministic order, so a reload of several files at once produces
        // the same recompilation sequence every time.
        changed.sort_by_key(|(module, _)| *module);
        changed
    }
}

fn stamp_of(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(path: &std::path::Path, text: &str) {
        let mut file = std::fs::File::create(path).unwrap();
        file.write_all(text.as_bytes()).unwrap();
        file.sync_all().unwrap();
    }

    /// Filesystem timestamp granularity is coarse enough on some platforms that
    /// two writes in the same millisecond are indistinguishable. Tests nudge the
    /// stamp explicitly rather than sleeping, which would make them slow and
    /// still not guarantee anything.
    fn bump(path: &std::path::Path, text: &str) {
        write(path, text);
        let future = SystemTime::now() + std::time::Duration::from_secs(2);
        let file = std::fs::File::options().write(true).open(path).unwrap();
        let _ = file.set_modified(future);
    }

    #[test]
    fn an_unchanged_file_is_not_reported() {
        let dir = std::env::temp_dir().join(format!("som_watch_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.wgsl");
        write(&path, "fn a() {}\n");

        let mut watcher = SourceWatcher::default();
        watcher.watch(ModuleId(0), path.clone());
        assert!(
            watcher.poll().is_empty(),
            "the first poll must not fire on its own baseline"
        );
        assert!(watcher.poll().is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_changed_file_is_reported_once_with_its_new_source() {
        let dir = std::env::temp_dir().join(format!("som_watch_ch_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.wgsl");
        write(&path, "fn a() {}\n");

        let mut watcher = SourceWatcher::default();
        watcher.watch(ModuleId(0), path.clone());
        assert!(watcher.poll().is_empty());

        bump(&path, "fn a() { let x = 1; }\n");
        let changed = watcher.poll();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].0, ModuleId(0));
        assert!(changed[0].1.contains("let x = 1"));

        assert!(
            watcher.poll().is_empty(),
            "one edit must produce one reload, not one per poll"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A vanished file is not an edit.
    ///
    /// Editors that save by rename make the file briefly absent. Reporting that
    /// as a change would invalidate every dependent variant and recompile them
    /// against a file that is about to be replaced.
    #[test]
    fn a_missing_file_is_not_a_change() {
        let dir = std::env::temp_dir().join(format!("som_watch_rm_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.wgsl");
        write(&path, "fn a() {}\n");

        let mut watcher = SourceWatcher::default();
        watcher.watch(ModuleId(0), path.clone());
        assert!(watcher.poll().is_empty());

        std::fs::remove_file(&path).unwrap();
        assert!(
            watcher.poll().is_empty(),
            "a momentary absence is not an edit"
        );

        bump(&path, "fn a() { let x = 2; }\n");
        let changed = watcher.poll();
        assert_eq!(changed.len(), 1, "the rename's arrival is the edit");
        assert!(changed[0].1.contains("let x = 2"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_that_appears_later_is_picked_up() {
        let dir = std::env::temp_dir().join(format!("som_watch_new_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("later.wgsl");

        let mut watcher = SourceWatcher::default();
        watcher.watch(ModuleId(3), path.clone());
        assert!(watcher.poll().is_empty());

        bump(&path, "fn later() {}\n");
        assert_eq!(
            watcher.poll().len(),
            1,
            "a new shader must not need a restart"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
