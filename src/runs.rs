//! One order, start to finish: the grunt loop in the job's worktree, the checks on every
//! `finish`, a commit of whatever came of it, and the report.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use chrono::Utc;
use tokio_util::sync::CancellationToken;

use crate::checks::{CheckRun, CheckRunner};
use crate::config::{OrkConfig, RoleConfig};
use crate::env::Env;
use crate::events::{EventLine, EventSink, OrkEvent};
use crate::grunt::{
    FinishCall, FinishHandler, FinishVerdict, GruntLoop, LoopEnd, LoopSettings, Transcript,
};
use crate::jobs::{JobState, JobStore};
use crate::model::ModelClient;
use crate::orders::{JobStatus, Order, OrderMode, Report, TokenCount};
use crate::stats::JobStats;
use crate::text;
use crate::tools::{ToolContext, ToolRegistry};

pub struct RunService<'a> {
    pub model: &'a ModelClient,
    pub config: &'a OrkConfig,
    pub role: &'a RoleConfig,
    pub env: &'a Env,
    pub events: &'a dyn EventSink,
}

impl RunService<'_> {
    /// Runs `order` as `job`, whose worktree must exist. Writes the report and the final status
    /// to the job store and returns the report. An error means the job couldn't run at all.
    pub async fn run(
        &self,
        order: &Order,
        job: &JobState,
        store: &JobStore,
        cancel: CancellationToken,
    ) -> anyhow::Result<Report> {
        let worktree = store.worktree_path(&job.id);
        // The report's counts (tool calls, files read) come from the events as they pass.
        let counted = Counted {
            inner: self.events,
            stats: Mutex::new(JobStats::default()),
        };
        let events: &dyn EventSink = &counted;
        let ctx = ToolContext::new(&worktree, order, self.config, self.env, cancel.clone())?;
        let checks = CheckRunner::new(
            ctx.policy.clone(),
            ctx.runner.clone(),
            Duration::from_secs(self.config.defaults.check_timeout_seconds),
        );
        let mut judge = Judge {
            checks: &order.checks,
            runner: checks,
            root: ctx.root.clone(),
            events,
            cancel: cancel.clone(),
            max_repairs: order
                .max_repairs
                .unwrap_or(self.config.defaults.max_repairs),
            repairs_used: 0,
            last: None,
        };

        let settings = LoopSettings::new(order, self.config, self.role);
        let transcript = Transcript::new(store.paths(&job.id).transcript);
        let tools = ToolRegistry::for_mode(order.mode);
        let mut grunt = GruntLoop::new(self.model, tools, events, transcript, settings);
        let outcome = grunt.run(order, &ctx, &mut judge).await;

        // Whatever came of it, keep it on the job branch, so failed work can be inspected too.
        let git = ctx.git();
        let message = format!("ork({}): {}", job.id, first_line(&order.goal));
        if let Err(err) = git.commit_all(&ctx.root, &message).await {
            self.events.emit(&OrkEvent::Warning {
                code: "commit_failed".into(),
                message: format!("{err:#}"),
            });
        }
        let files_changed = git
            .num_stat(store.root(), &job.base_sha, &job.branch)
            .await
            .unwrap_or_default();

        let failure = judge.last.as_ref().and_then(|run| run.failure.as_ref());
        let (status, reason) = match outcome.end {
            LoopEnd::Finished => match failure {
                Some(failure) => (
                    JobStatus::FailedChecks,
                    Some(format!("{} exited {}", failure.cmd, failure.exit)),
                ),
                None => (JobStatus::Passed, None),
            },
            LoopEnd::GaveUp => (JobStatus::GaveUp, outcome.reason.clone()),
            LoopEnd::Error => (JobStatus::Error, outcome.reason.clone()),
        };

        let stats = counted.stats.lock().map(|s| s.clone()).unwrap_or_default();
        let readonly = order.mode == OrderMode::Readonly;
        let seconds = (Utc::now() - job.created_at).num_milliseconds().max(0) as f64 / 1000.0;

        let report = Report {
            job: job.id.clone(),
            status,
            summary: outcome.summary.clone().unwrap_or_default(),
            files_changed,
            checks: judge.last.map(|run| run.results).unwrap_or_default(),
            repairs_used: judge.repairs_used,
            turns: outcome.turns,
            duration_s: (seconds * 10.0).round() / 10.0,
            tool_calls: stats.tools,
            failed_tool_calls: stats.tool_errors,
            files_read: if readonly {
                stats.files_read
            } else {
                Vec::new()
            },
            tokens: TokenCount {
                input: outcome.tokens_in,
                output: outcome.tokens_out,
            },
            model: self.model.model().to_string(),
            branch: job.branch.clone(),
            open_questions: outcome.open_questions.clone(),
            reason: reason.clone(),
        };
        store.write_report(&job.id, &report)?;
        store.set_status(&job.id, status, reason)?;

        // A read-only job has nothing to apply: its answer is the report. The worktree and the
        // branch go now; the job's files (the report, the logs) stay.
        if readonly {
            let cleaned = async {
                git.worktree_remove(store.root(), &worktree).await?;
                git.branch_delete(store.root(), &job.branch, true).await
            };
            if let Err(err) = cleaned.await {
                self.events.emit(&OrkEvent::Warning {
                    code: "cleanup_failed".into(),
                    message: format!("the read-only job's worktree stays: {err:#}"),
                });
            }
        }
        Ok(report)
    }
}

/// Passes the events on, and adds them up.
struct Counted<'a> {
    inner: &'a dyn EventSink,
    stats: Mutex<JobStats>,
}

impl EventSink for Counted<'_> {
    fn emit(&self, event: &OrkEvent) {
        if let Ok(mut stats) = self.stats.lock() {
            stats.push(&EventLine {
                ts: None,
                event: event.clone(),
            });
        }
        self.inner.emit(event);
    }
}

/// The `on_finish` that runs the checks. Passing checks accept; failing ones reject with their
/// output while repairs are left, which is all a repair turn is: a rejected `finish`. With no
/// repairs left it accepts, and the failure goes into the report.
struct Judge<'a> {
    checks: &'a [String],
    runner: CheckRunner,
    root: PathBuf,
    events: &'a dyn EventSink,
    cancel: CancellationToken,
    max_repairs: u32,
    repairs_used: u32,
    /// The latest round of checks.
    last: Option<CheckRun>,
}

impl FinishHandler for Judge<'_> {
    async fn on_finish(&mut self, _call: FinishCall) -> FinishVerdict {
        // A readonly investigation without checks: its answer is the summary.
        if self.checks.is_empty() {
            return FinishVerdict::Accept;
        }
        let run = match self
            .runner
            .run(self.checks, &self.root, self.events, &self.cancel)
            .await
        {
            Ok(run) => run,
            // Cancelled: the loop sees the token and ends the run itself.
            Err(_) => return FinishVerdict::Accept,
        };
        let verdict = match &run.failure {
            None => FinishVerdict::Accept,
            Some(failure) if self.repairs_used < self.max_repairs => {
                self.repairs_used += 1;
                self.events.emit(&OrkEvent::Repair {
                    attempt: self.repairs_used,
                });
                FinishVerdict::Reject(format!(
                    "Checks failed. {} exited {}:\n{}\nFix the problem, then call finish again.",
                    failure.cmd, failure.exit, failure.output
                ))
            }
            Some(_) => FinishVerdict::Accept,
        };
        self.last = Some(run);
        verdict
    }
}

/// The goal's first line, for the commit message (at most 72 characters).
fn first_line(goal: &str) -> String {
    let line = goal.lines().next().unwrap_or_default().trim();
    text::ellipsize(line, 72)
}
