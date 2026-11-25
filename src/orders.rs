//! Orders (what the chief asks for) and reports (what comes back). Contracts: docs/cli.md.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::process::CommandPolicy;

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

impl JobStatus {
    /// The name on the wire: `passed`, `failed_checks`, …
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Running => "running",
            JobStatus::Passed => "passed",
            JobStatus::FailedChecks => "failed_checks",
            JobStatus::GaveUp => "gave_up",
            JobStatus::Error => "error",
            JobStatus::Applied => "applied",
            JobStatus::Discarded => "discarded",
        }
    }
}

/// Every problem with `order`, at once (the chief fixes them all in one go). `root` is the repo
/// root its paths are relative to; `policy` is the command allow-list its checks must pass.
pub fn validate(order: &Order, root: &Path, policy: &CommandPolicy) -> Vec<String> {
    let mut problems = Vec::new();
    if order.goal.trim().is_empty() {
        problems.push("goal: required".to_string());
    }
    if order.mode == OrderMode::Write {
        if order.checks.is_empty() {
            problems.push(
                "checks: at least one is required (or use --readonly for an investigation)"
                    .to_string(),
            );
        }
        if order.write_scope.is_empty() {
            problems.push(
                "write_scope: required: the files the grunt may change (--scope 'src/**')"
                    .to_string(),
            );
        }
    }
    for pattern in &order.write_scope {
        if globset::Glob::new(pattern).is_err() {
            problems.push(format!("write_scope: {pattern} is not a valid glob"));
        }
    }
    for path in &order.paths {
        if !root.join(path).is_file() {
            problems.push(format!(
                "paths: {path} not found (paths are relative to the repo root)"
            ));
        }
    }
    for check in &order.checks {
        if let Err(refused) = policy.check(check) {
            problems.push(format!("checks: {check}: {refused}"));
        }
    }
    problems
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
