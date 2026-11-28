//! One job's run, wherever it runs: `ork run` in the foreground, or the background worker
//! (`ork __work JOB`, v0.2). The job already exists (`queued`, its worktree made, its `start`
//! event written); this takes it from there to its `result` line.
//!
//! - **A ticker** runs beside the job, once a second: it watches for `ork cancel`'s marker file
//!   (separate processes share files, not memory) and, every tenth tick, rewrites the heartbeat
//!   that tells readers the job is alive.
//! - **The job's own token** is a child of the caller's: Ctrl+C on a foreground run still reaches
//!   it, and a cancel reaches only this job.
//! - **The `result` line** is always written to `events.jsonl`, whatever happened, so whoever
//!   follows the file (`ork watch`, the TUI) sees the end.

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::cli::{CommandResult, exit};
use crate::config::OrkConfig;
use crate::env::Env;
use crate::events::{CompositeSink, EventSink, FileSink};
use crate::jobs::{BEAT, JobStore};
use crate::model::ModelClient;
use crate::orders::{JobStatus, Report};
use crate::runs::RunService;

/// How often the ticker looks for a cancel request.
const TICK: Duration = Duration::from_secs(1);

/// Runs job `id` to its end and returns its result (also appended to `events.jsonl`). `also`
/// gets the events too (the caller's stdout in the foreground; nothing in the worker).
pub async fn run_job(
    store: &JobStore,
    id: &str,
    config: &OrkConfig,
    env: &Env,
    also: Option<Arc<dyn EventSink>>,
    cancel: CancellationToken,
) -> CommandResult {
    let token = cancel.child_token();
    let _ticker = ticker(store.clone(), id.to_string(), token.clone());

    let file: Arc<dyn EventSink> = Arc::new(FileSink::new(store.paths(id).events));
    let mut sinks = vec![file.clone()];
    sinks.extend(also);
    let events = CompositeSink(sinks);

    let result = match run(store, id, config, env, &events, token).await {
        Ok(report) => result_for(&report),
        Err(err) => {
            // It couldn't run at all (its order unreadable, its role gone from the config…).
            let message = format!("{err:#}");
            let _ = store.set_status(id, JobStatus::Error, Some(message.clone()));
            let mut result = CommandResult::error(message);
            result.job = Some(id.to_string());
            result
        }
    };
    file.emit(&result.clone().into_event());
    store.clear_cancel(id);
    result
}

async fn run(
    store: &JobStore,
    id: &str,
    config: &OrkConfig,
    env: &Env,
    events: &dyn EventSink,
    token: CancellationToken,
) -> anyhow::Result<Report> {
    let order = store.load_order(id)?;
    let role_name = order.role_name().to_string();
    let (client, role) = ModelClient::for_role(config, &role_name, env)?;
    let job = store.start(id)?;
    let service = RunService {
        model: &client,
        config,
        role,
        role_name: &role_name,
        env,
        events,
    };
    service.run(&order, &job, store, token).await
}

/// Once a second: a cancel request cancels the job's token; every tenth tick, the heartbeat.
fn ticker(store: JobStore, id: String, token: CancellationToken) -> AbortOnDrop {
    let beat_every = (BEAT.as_secs() / TICK.as_secs()).max(1);
    AbortOnDrop(tokio::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        for n in 0u64.. {
            tick.tick().await;
            if store.cancel_requested(&id) {
                token.cancel();
            }
            if n % beat_every == 0 {
                let _ = store.beat(&id);
            }
        }
    }))
}

/// passed → 0; failed_checks or gave_up → 4; error → 1.
pub fn result_for(report: &Report) -> CommandResult {
    let code = match report.status {
        JobStatus::FailedChecks | JobStatus::GaveUp => exit::CHECKS_FAILED,
        JobStatus::Error => exit::ERROR,
        _ => exit::OK,
    };
    let mut result = CommandResult::new(report.status.as_str(), code);
    result.job = Some(report.job.clone());
    result.report = Some(report.clone());
    if report.status == JobStatus::Error {
        result.error = Some(
            report
                .reason
                .clone()
                .unwrap_or_else(|| "the run failed".into()),
        );
    }
    result
}

/// Stops a background task when it goes out of scope, however the scope ends.
pub struct AbortOnDrop(pub tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
