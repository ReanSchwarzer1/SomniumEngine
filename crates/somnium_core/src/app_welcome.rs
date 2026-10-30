//! The editor's side of the welcome screen: the Recent Projects list on disk, what the screen shows of it, and what
//! its buttons do.
//!
//! The list is one small JSON file beside the editor preferences (`recent_projects.json` next to `editor.toml`). An
//! editor reads it, changes it and writes it back at the moment it records or forgets a project, and reads it afresh
//! each time the screen opens, so two editors open at once never write a stale copy over each other's entries.
use super::*;
use serde::{Deserialize, Serialize};
use somnium_ui::editor::welcome::WelcomeProject;
use std::path::{Path, PathBuf};

/// How many projects the list keeps.
const KEEP: usize = 12;

/// A project's picture on the welcome screen, in its folder. File > Save Project Thumbnail writes it.
const THUMBNAIL: &str = "thumbnail.png";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct RecentProject {
    /// The project folder, the one holding `game.project.json`.
    path: PathBuf,
    /// Its name when it was last opened.
    name: String,
    /// When it was last opened, in seconds since 1970 (UTC).
    opened: u64,
}

#[derive(Serialize, Deserialize)]
struct RecentFile {
    version: u32,
    projects: Vec<RecentProject>,
}

fn store_path() -> PathBuf {
    crate::settings::default_global_path().with_file_name("recent_projects.json")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The list, newest first. No file is an empty list. A file that cannot be read is moved aside, not lost, and the
/// list starts again.
fn load(file: &Path) -> Vec<RecentProject> {
    let Ok(bytes) = std::fs::read(file) else {
        return Vec::new();
    };
    match serde_json::from_slice::<RecentFile>(&bytes) {
        Ok(list) => list.projects,
        Err(error) => {
            let aside = file.with_extension("json.unreadable");
            warn!(%error, aside = %aside.display(), "The recent projects list could not be read; it was moved aside");
            let _ = std::fs::rename(file, aside);
            Vec::new()
        }
    }
}

fn save(file: &Path, projects: &[RecentProject]) -> Result<(), String> {
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(&RecentFile { version: 1, projects: projects.to_vec() })
        .map_err(|e| e.to_string())?;
    // Written beside it and renamed over it, so a crash part-way never leaves half a list.
    let partial = file.with_extension("json.partial");
    std::fs::write(&partial, text).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, file).map_err(|e| e.to_string())
}

/// Put this project first, replacing any older entry for the same folder, and keep the newest [`KEEP`].
fn record(file: &Path, project: RecentProject) -> Result<(), String> {
    let mut projects = load(file);
    projects.retain(|p| !same_folder(&p.path, &project.path));
    projects.insert(0, project);
    projects.truncate(KEEP);
    save(file, &projects)
}

/// Take a folder off the list. Returns whether it was on it.
fn forget(file: &Path, folder: &Path) -> Result<bool, String> {
    let mut projects = load(file);
    let before = projects.len();
    projects.retain(|p| !same_folder(&p.path, folder));
    if projects.len() == before {
        return Ok(false);
    }
    save(file, &projects).map(|()| true)
}

/// The folder as a person writes it: canonicalising on Windows adds a `\\?\` prefix nobody wants to read.
fn plain(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\UNC\") {
        Some(rest) => PathBuf::from(format!(r"\\{rest}")),
        None => PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text)),
    }
}

fn same_folder(a: &Path, b: &Path) -> bool {
    let (a, b) = (plain(a), plain(b));
    if cfg!(windows) {
        a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}

/// How long ago `then` was, at `now` (both seconds since 1970): "just now", "3 hours ago", "on 14 Sep 2026".
fn opened_ago(now: u64, then: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    let ago = |count: u64, unit: &str| format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" });
    let seconds = now.saturating_sub(then);
    if seconds < MINUTE {
        "just now".into()
    } else if seconds < HOUR {
        ago(seconds / MINUTE, "minute")
    } else if seconds < DAY {
        ago(seconds / HOUR, "hour")
    } else if seconds < 2 * DAY {
        "yesterday".into()
    } else if seconds < 30 * DAY {
        ago(seconds / DAY, "day")
    } else {
        const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        let (year, month, day) = civil_date(then);
        format!("on {day} {} {year}", MONTHS[month as usize - 1])
    }
}

/// The UTC calendar date of a moment, from Howard Hinnant's `civil_from_days`.
fn civil_date(seconds: u64) -> (i64, u32, u32) {
    let z = (seconds / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// What the screen shows of the list, as the disk is now: whether each folder is still there, whether it has a
/// picture, and which one this editor has open.
fn welcome_projects(projects: Vec<RecentProject>, current: Option<&Path>, now: u64) -> Vec<WelcomeProject> {
    projects
        .into_iter()
        .map(|project| {
            let missing = !project.path.join("game.project.json").is_file();
            let thumbnail = Some(project.path.join(THUMBNAIL)).filter(|picture| !missing && picture.is_file());
            WelcomeProject {
                current: current.is_some_and(|open| same_folder(open, &project.path)),
                opened: opened_ago(now, project.opened),
                name: project.name,
                path: project.path,
                thumbnail,
                missing,
            }
        })
        .collect()
}

impl<G: GameApp> Engine<G> {
    /// Once the editor is up: with a project open, put it at the top of the Recent Projects list; with none, show
    /// the welcome screen.
    pub(super) fn welcome_at_startup(&mut self) {
        let Some(root) = self.config.project_root.clone() else {
            self.show_welcome();
            return;
        };
        let name = crate::authoring::project::ProjectPaths::open(&root).map_or_else(
            |_| root.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            |project| project.manifest.name,
        );
        if let Err(error) = record(&store_path(), RecentProject { path: plain(&root), name, opened: now() }) {
            warn!(%error, "The recent projects list could not be updated");
        }
    }

    /// Read the list afresh (another editor may have changed it), and show it. Each project picture is decoded off
    /// the frame by the preview jobs and arrives through the thumbnail atlas.
    pub(super) fn show_welcome(&mut self) {
        let projects = welcome_projects(load(&store_path()), self.config.project_root.as_deref(), now());
        for picture in projects.iter().filter_map(|p| p.thumbnail.clone()) {
            if self.preview_jobs.contains_key(&picture) {
                continue;
            }
            let source = picture.clone();
            match self.jobs.submit("Project thumbnail", JobPriority::Visible, move |_| {
                Ok(somnium_asset::preview::decode_texture(&source)
                    .map(|rgba| somnium_asset::preview::PreparedPreview { rgba, from_disk_cache: false }))
            }) {
                Ok(job) => {
                    self.preview_jobs.insert(picture, job);
                }
                Err(error) => warn!(?error, "preview queue is full"),
            }
        }
        if let Some(ui) = self.ui_manager.as_mut() {
            ui.show_welcome(&projects);
        }
    }

    /// Open a project from the list in its own editor. The one already open just closes the screen.
    pub(super) fn open_recent_project(&mut self, folder: &Path) {
        let result = self.open_editor_project(folder);
        let Some(ui) = self.ui_manager.as_mut() else {
            return;
        };
        match result {
            Ok(message) => {
                ui.close_welcome();
                ui.push_toast(&message);
            }
            Err(error) => ui.push_toast(&format!("Project could not open: {error}")),
        }
    }

    pub(super) fn forget_recent_project(&mut self, folder: &Path) {
        if let Err(error) = forget(&store_path(), folder)
            && let Some(ui) = self.ui_manager.as_mut()
        {
            ui.push_toast(&format!("The recent projects list could not be changed: {error}"));
        }
        self.show_welcome();
    }

    /// Save what the viewport shows, without the editor around it, as the open project's thumbnail. The renderer
    /// writes it with the next frame.
    pub(super) fn save_project_thumbnail(&mut self) {
        let result = (|| -> Result<String, String> {
            let root = self.config.project_root.as_deref().ok_or("No project is open")?;
            let picture = plain(&root.join(THUMBNAIL));
            self.renderer
                .as_mut()
                .ok_or("The renderer is not running")?
                .request_authoring_capture(&picture, false)?;
            Ok(format!("This view is now the project's thumbnail ({})", picture.display()))
        })();
        if let Some(ui) = self.ui_manager.as_mut() {
            ui.push_toast(&result.unwrap_or_else(|error| format!("Thumbnail not saved: {error}")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh scratch folder for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("somnium-recent-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn project(path: &str, opened: u64) -> RecentProject {
        RecentProject { path: PathBuf::from(path), name: path.rsplit(['/', '\\']).next().unwrap().into(), opened }
    }

    #[test]
    fn the_newest_open_goes_first_and_replaces_its_old_entry() {
        let file = scratch("order").join("recent_projects.json");
        assert!(load(&file).is_empty(), "no file is an empty list");
        record(&file, project("C:/Games/Alpha", 10)).unwrap();
        record(&file, project("C:/Games/Beta", 20)).unwrap();
        record(&file, RecentProject { name: "Alpha renamed".into(), ..project("C:/Games/Alpha", 30) }).unwrap();
        let list = load(&file);
        assert_eq!(list.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Alpha renamed", "Beta"]);
        assert_eq!(list[0].opened, 30);
        assert!(!file.with_extension("json.partial").exists(), "the partial write is renamed into place");
    }

    #[test]
    fn the_list_keeps_the_newest_twelve() {
        let file = scratch("keep").join("recent_projects.json");
        for n in 0..15 {
            record(&file, project(&format!("C:/Games/P{n}"), n)).unwrap();
        }
        let list = load(&file);
        assert_eq!(list.len(), KEEP);
        assert_eq!(list[0].name, "P14");
        assert_eq!(list[KEEP - 1].name, "P3");
    }

    #[test]
    fn forgetting_takes_off_only_that_folder() {
        let file = scratch("forget").join("recent_projects.json");
        record(&file, project("C:/Games/Alpha", 1)).unwrap();
        record(&file, project("C:/Games/Beta", 2)).unwrap();
        assert!(forget(&file, Path::new("C:/Games/Alpha")).unwrap());
        assert!(!forget(&file, Path::new("C:/Games/Alpha")).unwrap(), "already gone");
        assert_eq!(load(&file), vec![project("C:/Games/Beta", 2)]);
    }

    #[test]
    fn an_unreadable_list_is_moved_aside_not_lost() {
        let dir = scratch("unreadable");
        let file = dir.join("recent_projects.json");
        std::fs::write(&file, "{ not json").unwrap();
        assert!(load(&file).is_empty());
        assert_eq!(std::fs::read_to_string(dir.join("recent_projects.json.unreadable")).unwrap(), "{ not json");
        record(&file, project("C:/Games/Alpha", 1)).unwrap();
        assert_eq!(load(&file).len(), 1);
    }

    #[test]
    fn a_list_from_a_newer_editor_still_loads() {
        let file = scratch("newer").join("recent_projects.json");
        std::fs::write(
            &file,
            r#"{"version":2,"pinned":[],"projects":[{"path":"C:/Games/Alpha","name":"Alpha","opened":5,"colour":"red"}]}"#,
        )
        .unwrap();
        assert_eq!(load(&file), vec![project("C:/Games/Alpha", 5)]);
    }

    #[cfg(windows)]
    #[test]
    fn one_folder_is_one_entry_whatever_its_case_or_prefix() {
        assert!(same_folder(Path::new(r"\\?\C:\Games\Alpha"), Path::new(r"c:\games\alpha")));
        assert!(!same_folder(Path::new(r"C:\Games\Alpha"), Path::new(r"C:\Games\Alphabet")));
        assert_eq!(plain(Path::new(r"\\?\C:\Games\Alpha")), PathBuf::from(r"C:\Games\Alpha"));
        assert_eq!(plain(Path::new(r"\\?\UNC\server\share\Alpha")), PathBuf::from(r"\\server\share\Alpha"));
        let file = scratch("case").join("recent_projects.json");
        record(&file, project(r"\\?\C:\Games\Alpha", 1)).unwrap();
        record(&file, project(r"C:\GAMES\ALPHA", 2)).unwrap();
        assert_eq!(load(&file).len(), 1);
    }

    #[test]
    fn when_reads_like_a_person_would_say_it() {
        let now = 1_760_000_000; // 9 Oct 2025, 08:53 UTC
        assert_eq!(opened_ago(now, now), "just now");
        assert_eq!(opened_ago(now, now + 500), "just now", "a clock that went back is not the future");
        assert_eq!(opened_ago(now, now - 59), "just now");
        assert_eq!(opened_ago(now, now - 60), "1 minute ago");
        assert_eq!(opened_ago(now, now - 125), "2 minutes ago");
        assert_eq!(opened_ago(now, now - 3_600), "1 hour ago");
        assert_eq!(opened_ago(now, now - 5 * 3_600), "5 hours ago");
        assert_eq!(opened_ago(now, now - 30 * 3_600), "yesterday");
        assert_eq!(opened_ago(now, now - 3 * 86_400), "3 days ago");
        assert_eq!(opened_ago(now, now - 29 * 86_400), "29 days ago");
        assert_eq!(opened_ago(now, 951_782_400), "on 29 Feb 2000");
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(now), (2025, 10, 9));
    }

    #[test]
    fn the_screen_sees_missing_folders_pictures_and_the_open_project() {
        let dir = scratch("view");
        let (alpha, beta, gone) = (dir.join("Alpha"), dir.join("Beta"), dir.join("Gone"));
        for folder in [&alpha, &beta] {
            std::fs::create_dir_all(folder).unwrap();
            std::fs::write(folder.join("game.project.json"), "{}").unwrap();
        }
        std::fs::write(beta.join(THUMBNAIL), b"png").unwrap();
        let entry = |path: &Path| RecentProject { path: path.to_path_buf(), name: "x".into(), opened: 0 };
        let view = welcome_projects(vec![entry(&alpha), entry(&beta), entry(&gone)], Some(&alpha), 7_200);
        assert!(view[0].current && !view[0].missing && view[0].thumbnail.is_none());
        assert_eq!(view[0].opened, "2 hours ago");
        assert!(!view[1].current && view[1].thumbnail.as_deref() == Some(beta.join(THUMBNAIL).as_path()));
        assert!(view[2].missing && view[2].thumbnail.is_none() && !view[2].current);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
