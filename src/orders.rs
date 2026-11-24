//! Orders (what the chief asks for) and reports (what comes back). Contracts: docs/cli.md.

use serde::{Deserialize, Serialize};

/// `write` (the default) gets every tool; `readonly` gets only the reading ones, for
/// investigations whose answer comes back in the summary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderMode {
    #[default]
    Write,
    Readonly,
}

/// One task for the grunt: what to do, what it may touch, and the commands that prove it's done.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Order {
    /// Required (the validator checks it isn't empty).
    pub goal: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Files to put into the first message, relative to the repo root.
    pub paths: Vec<String>,
    /// Globs, relative to the repo root: the only files the grunt may write.
    pub write_scope: Vec<String>,
    /// Command lines; each must pass the command policy.
    pub checks: Vec<String>,
    pub mode: OrderMode,
    /// `None` → the config's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_repairs: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_minutes: Option<u32>,
}

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
