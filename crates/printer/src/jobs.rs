//! Persistent print-job queue.
//!
//! A job survives a restart of the printer service and has no relationship
//! with whichever client (web UI, Bluetooth phone) created it -- once a job
//! is accepted here, the Pi owns it. The queue is a small JSON file written
//! atomically, in the same style as `state.rs`.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Upper bound on copies per item per request. Bounds a single request's
/// work so a typo cannot queue an unbounded print run.
pub const MAX_COPIES: u32 = 100;
/// Finished jobs retained for status display; older ones are dropped.
const MAX_FINISHED_JOBS: usize = 20;
/// Consecutive device failures after which a running job is marked failed.
pub const MAX_CONSECUTIVE_FAILURES: u32 = 5;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Item,
    All,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn is_active(self) -> bool {
        matches!(self, JobState::Queued | JobState::Running)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub kind: JobKind,
    /// Library item ids, snapshotted when the job was accepted. Each is
    /// printed `copies` times in a row.
    pub items: Vec<String>,
    pub copies: u32,
    /// Print units (one physical print each) completed or skipped so far.
    pub done: u32,
    /// Units skipped because the item was removed from the library after
    /// the job was accepted.
    pub skipped: u32,
    pub state: JobState,
    pub created_at: f64,
    pub finished_at: Option<f64>,
    pub error: Option<String>,
    #[serde(default)]
    pub consecutive_failures: u32,
}

impl Job {
    pub fn total(&self) -> u32 {
        self.items.len() as u32 * self.copies
    }

    /// The item the next print unit belongs to, if any remain.
    pub fn next_item(&self) -> Option<&str> {
        if self.copies == 0 || self.done >= self.total() {
            return None;
        }
        self.items.get((self.done / self.copies) as usize).map(String::as_str)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct JobStore {
    pub jobs: Vec<Job>,
}

impl JobStore {
    /// A missing or corrupt queue file is an empty queue; the service must
    /// always boot (see `state::load`). Jobs left `Running` by a crash are
    /// returned to `Queued` and resume from their recorded progress.
    pub fn load(path: &Path) -> JobStore {
        let mut store: JobStore = fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        for job in &mut store.jobs {
            if job.state == JobState::Running {
                job.state = JobState::Queued;
            }
        }
        store
    }

    /// Re-reads the file without the crash-recovery rewrite of `Running`.
    pub fn reload(path: &Path) -> JobStore {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&mut self, path: &Path) -> std::io::Result<()> {
        let mut finished_seen = 0;
        let mut keep = Vec::with_capacity(self.jobs.len());
        // Newest first while trimming, then restore chronological order.
        for job in self.jobs.drain(..).rev() {
            if job.state.is_active() {
                keep.push(job);
            } else if finished_seen < MAX_FINISHED_JOBS {
                finished_seen += 1;
                keep.push(job);
            }
        }
        keep.reverse();
        self.jobs = keep;

        let temp_path = path.with_extension("json.tmp");
        fs::write(&temp_path, serde_json::to_vec_pretty(self).unwrap())?;
        fs::rename(&temp_path, path)
    }

    #[cfg(test)]
    pub fn find(&self, id: &str) -> Option<&Job> {
        self.jobs.iter().find(|job| job.id == id)
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|job| job.id == id)
    }

    pub fn active_all_job(&self) -> Option<&Job> {
        self.jobs.iter().find(|job| job.kind == JobKind::All && job.state.is_active())
    }

    /// Oldest active job: jobs run strictly in the order they were accepted.
    pub fn next_active_id(&self) -> Option<String> {
        self.jobs.iter().find(|job| job.state.is_active()).map(|job| job.id.clone())
    }

    pub fn enqueue(&mut self, kind: JobKind, items: Vec<String>, copies: u32, now: f64) -> Job {
        let job = Job {
            id: format!("job-{}-{:08x}", (now * 1000.0) as u64, rand::random::<u32>()),
            kind,
            items,
            copies,
            done: 0,
            skipped: 0,
            state: JobState::Queued,
            created_at: now,
            finished_at: None,
            error: None,
            consecutive_failures: 0,
        };
        self.jobs.push(job.clone());
        job
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("jobs-test-{tag}-{}.json", std::process::id()))
    }

    #[test]
    fn next_item_walks_items_with_copies_in_a_row() {
        let mut store = JobStore::default();
        let mut job = store.enqueue(JobKind::All, vec!["a".into(), "b".into()], 2, 1.0);
        assert_eq!(job.total(), 4);
        let mut seen = Vec::new();
        while let Some(id) = job.next_item() {
            seen.push(id.to_string());
            job.done += 1;
        }
        assert_eq!(seen, vec!["a", "a", "b", "b"]);
    }

    #[test]
    fn running_jobs_return_to_queued_on_load_and_keep_progress() {
        let path = temp_path("recover");
        let mut store = JobStore::default();
        let id = store.enqueue(JobKind::Item, vec!["a".into()], 5, 1.0).id;
        store.find_mut(&id).unwrap().state = JobState::Running;
        store.find_mut(&id).unwrap().done = 3;
        store.save(&path).unwrap();

        let loaded = JobStore::load(&path);
        let job = loaded.find(&id).unwrap();
        assert_eq!(job.state, JobState::Queued);
        assert_eq!(job.done, 3);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn corrupt_file_is_an_empty_queue() {
        let path = temp_path("corrupt");
        fs::write(&path, b"{not json").unwrap();
        assert!(JobStore::load(&path).jobs.is_empty());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn save_trims_old_finished_jobs_but_never_active_ones() {
        let path = temp_path("trim");
        let mut store = JobStore::default();
        for i in 0..(MAX_FINISHED_JOBS + 5) {
            let id = store.enqueue(JobKind::Item, vec!["a".into()], 1, i as f64).id;
            store.find_mut(&id).unwrap().state = JobState::Done;
        }
        let active = store.enqueue(JobKind::Item, vec!["a".into()], 1, 999.0).id;
        store.save(&path).unwrap();
        assert_eq!(store.jobs.len(), MAX_FINISHED_JOBS + 1);
        assert!(store.find(&active).is_some());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn only_one_all_job_may_be_active() {
        let mut store = JobStore::default();
        assert!(store.active_all_job().is_none());
        let id = store.enqueue(JobKind::All, vec!["a".into()], 1, 1.0).id;
        assert!(store.active_all_job().is_some());
        store.find_mut(&id).unwrap().state = JobState::Done;
        assert!(store.active_all_job().is_none());
    }
}
