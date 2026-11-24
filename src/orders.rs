//! Orders (what the chief asks for) and reports (what comes back). Contracts: docs/cli.md.

use serde::{Deserialize, Serialize};

/// Where a job is. `running` until the run ends; `applied` / `discarded` after the chief decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Passed,
    FailedChecks,
    GaveUp,
    Error,
    Applied,
    Discarded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub cmd: String,
    pub exit: i32,
    pub seconds: f64,
    pub tail: String,
}

/// `in` and `out` are keywords-ish in most languages, so the Rust fields are spelled out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenCount {
    #[serde(rename = "in")]
    pub input: u64,
    #[serde(rename = "out")]
    pub output: u64,
}

/// What the chief reads instead of the work. Aim for under ~400 tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub job: String,
    pub status: JobStatus,
    pub summary: String,
    pub files_changed: Vec<FileChange>,
    pub checks: Vec<CheckResult>,
    pub repairs_used: u32,
    pub turns: u32,
    pub tokens: TokenCount,
    pub model: String,
    pub branch: String,
    pub open_questions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
