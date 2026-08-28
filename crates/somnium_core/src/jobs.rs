//! Editor-side job helpers over [`somnium_jobs`].
//!
//! **The scheduler used to live here.** Phase CONTROL wrote a `JobRegistry` in
//! this file with a deliberately narrow public surface, and said so in its own
//! first doc line, *"so Phase MORROWIND can move this module into
//! `somnium_jobs` without changing call sites"*. MORROWIND-B did exactly that:
//! the queue, the workers, the cancellation tokens and all five of CONTROL's
//! tests now live in `crates/somnium_jobs/`, extended with declared deadlines,
//! a budgeted main-thread drain and profiler telemetry.
//!
//! What is left here is the part that was never generic: two submissions that
//! shell out to `cargo` inside *this workspace*. `somnium_jobs` has no
//! dependencies and knows nothing about content roots, terrain packs or BC7
//! encoders, and it should not — a scheduler that knows what it is scheduling
//! has stopped being a scheduler.
//!
//! Everything else is re-exported, so `crate::jobs::JobPriority` and friends
//! still resolve and the migration touched imports rather than call sites.

pub use somnium_jobs::{
    DrainStats, JobContext, JobDesc, JobError, JobHandle, JobPriority, JobSnapshot, JobStatus,
    JobSystem, JobZone,
};

/// Long editor operations that run an external `cargo` command.
///
/// An extension trait rather than inherent methods, because the alternative is
/// `somnium_jobs` depending on the workspace layout.
///
/// **Neither has a caller yet.** They were written by Phase CONTROL against the
/// terrain and BC7 tools and the editor never grew the buttons; MORROWIND-A's
/// census names dead code as a category worth seeing rather than deleting on
/// sight, and this is a two-method API waiting for a menu item, not rot. If
/// Track 4's cook lands first and replaces both, delete them then.
pub trait EditorJobs {
    /// Run the existing terrain PNG bake through the shared scheduler.
    fn submit_terrain_bake(
        &mut self,
        workspace: std::path::PathBuf,
        arguments: Vec<String>,
    ) -> Result<JobHandle<()>, JobError>;

    /// Run the existing BC7 encoder through the shared scheduler.
    fn submit_bc7_encode(
        &mut self,
        workspace: std::path::PathBuf,
        fast: bool,
    ) -> Result<JobHandle<()>, JobError>;
}

impl EditorJobs for JobSystem {
    fn submit_terrain_bake(
        &mut self,
        workspace: std::path::PathBuf,
        arguments: Vec<String>,
    ) -> Result<JobHandle<()>, JobError> {
        let mut command = std::process::Command::new("cargo");
        command
            .current_dir(workspace)
            .args([
                "run",
                "--release",
                "-p",
                "somnium_asset",
                "--example",
                "pack_terrain",
                "--",
            ])
            .args(arguments);
        submit_process(self, "Terrain bake", JobPriority::User, command)
    }

    fn submit_bc7_encode(
        &mut self,
        workspace: std::path::PathBuf,
        fast: bool,
    ) -> Result<JobHandle<()>, JobError> {
        let mut command = std::process::Command::new("cargo");
        command.current_dir(workspace).args([
            "run",
            "--release",
            "-p",
            "somnium_renderer",
            "--example",
            "encode_terrain_bc7",
        ]);
        if fast {
            command.arg("--").arg("--fast");
        }
        submit_process(self, "BC7 encode", JobPriority::User, command)
    }
}

/// Spawn `command` on a worker and poll it, so cancellation kills the child.
///
/// The 10 ms poll is a compromise and worth naming: `wait()` would block the
/// worker uninterruptibly, and a bake that cannot be cancelled is worse than
/// one that takes an extra 10 ms to notice it was.
fn submit_process(
    jobs: &mut JobSystem,
    name: &'static str,
    priority: JobPriority,
    mut command: std::process::Command,
) -> Result<JobHandle<()>, JobError> {
    jobs.submit(name, priority, move |context| {
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        loop {
            if context.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                return Err("cancelled".into());
            }
            match child.try_wait().map_err(|error| error.to_string())? {
                Some(status) if status.success() => return Ok(()),
                Some(status) => return Err(format!("process exited with {status}")),
                None => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
    })
}

/// One job name's contribution to a frame, for the profiler panel.
///
/// `phase_MORROWIND.md` §8: *"every job reports to the Phase 29 profiler as a
/// CPU zone with its priority and its queue wait. A job system without
/// visibility is a source of mystery hitches."*
///
/// Aggregated by name rather than listed per job, because a folder open
/// produces sixty identical decodes and sixty identical rows is not a report.
#[derive(Clone, Debug, PartialEq)]
pub struct JobProfileRow {
    /// The `&'static str` the submitter had to provide.
    pub name: &'static str,
    /// The class it was submitted with.
    pub priority: JobPriority,
    /// How many finished under this name.
    pub count: u32,
    /// Total time spent running.
    pub ran_ms: f32,
    /// **The longest single queue wait**, which is the number that explains a
    /// stall. Totals hide it: sixty jobs waiting 1 ms each and one waiting
    /// 60 ms sum identically and mean completely different things.
    pub worst_queued_ms: f32,
    /// How many expired in the queue without running.
    ///
    /// Streaming that quietly drops half its requests and a scheduler that is
    /// keeping up look identical without this column.
    pub expired: u32,
}

/// Fold a frame's zones into one row per job name, worst first.
///
/// Sorted by `worst_queued_ms` because the reason to open this panel is a
/// hitch, and the row that explains a hitch is the one that waited longest.
#[must_use]
pub fn profile_rows(zones: &[JobZone]) -> Vec<JobProfileRow> {
    let mut rows: Vec<JobProfileRow> = Vec::new();
    for zone in zones {
        let ran_ms = zone.ran_for.as_secs_f32() * 1000.0;
        let queued_ms = zone.queued_for.as_secs_f32() * 1000.0;
        let expired = u32::from(zone.outcome == JobStatus::Expired);
        match rows.iter_mut().find(|row| row.name == zone.name) {
            Some(row) => {
                row.count += 1;
                row.ran_ms += ran_ms;
                row.worst_queued_ms = row.worst_queued_ms.max(queued_ms);
                row.expired += expired;
                // A name submitted at two classes reports the more urgent one:
                // the panel's question is "what was the most important thing
                // waiting", not "what was the average thing waiting".
                row.priority = row.priority.max(zone.priority);
            }
            None => rows.push(JobProfileRow {
                name: zone.name,
                priority: zone.priority,
                count: 1,
                ran_ms,
                worst_queued_ms: queued_ms,
                expired,
            }),
        }
    }
    rows.sort_by(|a, b| {
        b.worst_queued_ms
            .total_cmp(&a.worst_queued_ms)
            .then_with(|| a.name.cmp(b.name))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn zone(name: &'static str, priority: JobPriority, queued: u64, ran: u64) -> JobZone {
        JobZone {
            name,
            priority,
            queued_for: Duration::from_millis(queued),
            ran_for: Duration::from_millis(ran),
            outcome: JobStatus::Completed,
        }
    }

    #[test]
    fn rows_aggregate_by_name_and_keep_the_worst_wait() {
        let rows = profile_rows(&[
            zone("thumbnail.decode", JobPriority::Background, 1, 200),
            zone("thumbnail.decode", JobPriority::Visible, 60, 240),
            zone("gltf.import", JobPriority::User, 2, 900),
        ]);
        assert_eq!(rows.len(), 2);

        // Worst wait first: that is the row that explains the hitch.
        assert_eq!(rows[0].name, "thumbnail.decode");
        assert_eq!(rows[0].count, 2);
        assert!((rows[0].worst_queued_ms - 60.0).abs() < 0.5);
        assert!((rows[0].ran_ms - 440.0).abs() < 1.0);
        assert_eq!(
            rows[0].priority,
            JobPriority::Visible,
            "a name submitted at two classes reports the more urgent one"
        );
        assert_eq!(rows[1].name, "gltf.import");
    }

    #[test]
    fn expired_jobs_are_counted_rather_than_hidden() {
        let mut dropped = zone("cell.stream", JobPriority::Visible, 40, 0);
        dropped.outcome = JobStatus::Expired;
        let rows = profile_rows(&[zone("cell.stream", JobPriority::Visible, 3, 12), dropped]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 2);
        assert_eq!(
            rows[0].expired, 1,
            "streaming that drops half its requests must not look like \
             streaming that is keeping up"
        );
    }

    #[test]
    fn no_zones_means_no_rows() {
        assert!(profile_rows(&[]).is_empty());
    }
}
