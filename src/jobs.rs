//! Jobs: one per `ork run`, kept as files under `.ork/jobs/<id>/` (no database until a daemon
//! needs one). The worktree is `.ork/worktrees/<id>`, on branch `ork/<id>`.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::orders::{JobStatus, Order, Report};

/// Characters for job ids: lower case and digits, without the ones that look alike (i l o 0 1).
const ID_ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
const ID_LENGTH: usize = 4;

/// Whether `id` looks like a job id: 1–32 lower-case letters and digits.
pub fn is_job_id(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// A new 4-character job id; drawn again while `taken(id)` says it's in use.
pub fn new_job_id(taken: impl Fn(&str) -> bool) -> String {
    loop {
        let id: String = (0..ID_LENGTH)
            .map(|_| ID_ALPHABET[fastrand::usize(..ID_ALPHABET.len())] as char)
            .collect();
        if !taken(&id) {
            return id;
        }
    }
}

/// `.ork/jobs/<id>/state.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobState {
    pub id: String,
    pub status: JobStatus,
    pub goal: String,
    pub base_sha: String,
    pub branch: String,
    /// Relative to the repo root, with `/`: `.ork/worktrees/<id>`.
    pub worktree: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// When the run ended (passed, failed_checks, gave_up or error). `updated_at` moves again on
    /// apply or discard; this doesn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The files of one job.
#[derive(Debug, Clone)]
pub struct JobPaths {
    pub dir: PathBuf,
    pub order: PathBuf,
    pub state: PathBuf,
    pub events: PathBuf,
    pub transcript: PathBuf,
    pub report: PathBuf,
}

/// The jobs of one repository.
#[derive(Debug, Clone)]
pub struct JobStore {
    root: PathBuf,
}

impl JobStore {
    /// `root` is the repository's top directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn jobs_dir(&self) -> PathBuf {
        self.root.join(".ork").join("jobs")
    }

    pub fn paths(&self, id: &str) -> JobPaths {
        let dir = self.jobs_dir().join(id);
        JobPaths {
            order: dir.join("order.json"),
            state: dir.join("state.json"),
            events: dir.join("events.jsonl"),
            transcript: dir.join("transcript.jsonl"),
            report: dir.join("report.json"),
            dir,
        }
    }

    /// The job's worktree, as a full path.
    pub fn worktree_path(&self, id: &str) -> PathBuf {
        self.root.join(".ork").join("worktrees").join(id)
    }

    /// A new job: picks an id, writes `order.json` and `state.json` (status `running`).
    pub fn create(&self, order: &Order, base_sha: &str) -> anyhow::Result<JobState> {
        let id = new_job_id(|id| self.jobs_dir().join(id).exists());
        let paths = self.paths(&id);
        std::fs::create_dir_all(&paths.dir)
            .with_context(|| format!("creating {}", paths.dir.display()))?;
        write_atomic(&paths.order, &serde_json::to_vec_pretty(order)?)?;

        let now = Utc::now();
        let state = JobState {
            status: JobStatus::Running,
            goal: order.goal.clone(),
            base_sha: base_sha.to_string(),
            branch: format!("ork/{id}"),
            worktree: format!(".ork/worktrees/{id}"),
            created_at: now,
            updated_at: now,
            finished_at: None,
            reason: None,
            id,
        };
        write_atomic(&paths.state, &serde_json::to_vec_pretty(&state)?)?;
        Ok(state)
    }

    /// The job's state. Checked on the way in, because the id comes from the command line and the
    /// file from disk (a cloned repo could even carry its own `.ork/`): the id must look like an
    /// id (no `../`), and the file must describe that job, its branch `ork/<id>`, its worktree
    /// `.ork/worktrees/<id>` and a hex base sha (no `--option` slipped in for git).
    pub fn load(&self, id: &str) -> anyhow::Result<JobState> {
        if !is_job_id(id) {
            bail!("{id:?} is not a job id (ork jobs lists them)");
        }
        let path = self.paths(id).state;
        if !path.is_file() {
            bail!("no job {id} (ork jobs lists them)");
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let state: JobState = serde_json::from_str(&text)
            .with_context(|| format!("{} is damaged", path.display()))?;
        let consistent = state.id == id
            && state.branch == format!("ork/{id}")
            && state.worktree == format!(".ork/worktrees/{id}")
            && (1..=64).contains(&state.base_sha.len())
            && state.base_sha.chars().all(|c| c.is_ascii_hexdigit());
        if !consistent {
            bail!(
                "{} is damaged: it doesn't describe job {id}",
                path.display()
            );
        }
        Ok(state)
    }

    pub fn load_order(&self, id: &str) -> anyhow::Result<Order> {
        let path = self.paths(id).order;
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        Ok(serde_json::from_str(&text)?)
    }

    /// Every job, newest first. Directories without a readable `state.json` are skipped.
    pub fn list(&self) -> anyhow::Result<Vec<JobState>> {
        let Ok(entries) = std::fs::read_dir(self.jobs_dir()) else {
            return Ok(Vec::new());
        };
        let mut jobs: Vec<JobState> = entries
            .flatten()
            .filter_map(|entry| self.load(&entry.file_name().to_string_lossy()).ok())
            .collect();
        jobs.sort_by_key(|job| std::cmp::Reverse(job.created_at));
        Ok(jobs)
    }

    pub fn set_status(
        &self,
        id: &str,
        status: JobStatus,
        reason: Option<String>,
    ) -> anyhow::Result<JobState> {
        let mut state = self.load(id)?;
        let now = Utc::now();
        let finishes = matches!(
            status,
            JobStatus::Passed | JobStatus::FailedChecks | JobStatus::GaveUp | JobStatus::Error
        );
        if finishes && state.finished_at.is_none() {
            state.finished_at = Some(now);
        }
        state.status = status;
        state.reason = reason;
        state.updated_at = now;
        write_atomic(&self.paths(id).state, &serde_json::to_vec_pretty(&state)?)?;
        Ok(state)
    }

    pub fn write_report(&self, id: &str, report: &Report) -> anyhow::Result<()> {
        write_atomic(&self.paths(id).report, &serde_json::to_vec_pretty(report)?)
    }

    pub fn load_report(&self, id: &str) -> anyhow::Result<Option<Report>> {
        let path = self.paths(id).report;
        if !path.is_file() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        Ok(Some(serde_json::from_str(&text)?))
    }
}

/// Writes the whole file or nothing: into `<file>.tmp`, then a rename over the old one (atomic on
/// one filesystem). A crash mid-write leaves the old file whole and a stray `.tmp` behind.
fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}
