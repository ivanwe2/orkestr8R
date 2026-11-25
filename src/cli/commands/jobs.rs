//! The job commands: `jobs`, `status`, `diff`, `logs`, `apply`, `discard`. What the chief does
//! with a job after `ork run`: read it, look at it, land it or drop it.

use std::path::PathBuf;

use anyhow::{Context, bail};
use chrono::Utc;
use clap::Args;
use serde_json::{Value, json};

use crate::cli::{CliContext, CommandResult, exit};
use crate::git::Git;
use crate::grunt::short_args;
use crate::jobs::{JobState, JobStore};
use crate::orders::JobStatus;
use crate::process::ProcessRunner;

#[derive(Debug, Args)]
pub struct JobArg {
    /// The job id (as `ork jobs` lists it; `ork/<id>` works too)
    pub job: String,
}

#[derive(Debug, Args)]
pub struct DiffArgs {
    /// The job id
    pub job: String,
    /// Only the summary: files and line counts
    #[arg(long)]
    pub stat: bool,
}

#[derive(Debug, Args)]
pub struct ApplyArgs {
    /// The job id
    pub job: String,
    /// One commit instead of a merge commit
    #[arg(long)]
    pub squash: bool,
    /// Apply a job whose checks failed or that gave up
    #[arg(long)]
    pub force: bool,
}

/// The repo's git and job store, from wherever in the repo `ork` runs.
async fn open(ctx: &CliContext) -> anyhow::Result<(Git, PathBuf, JobStore)> {
    let git = Git::new(ProcessRunner::new(ctx.env.clone()));
    let root = git.repo_root(&ctx.cwd).await?;
    let store = JobStore::new(&root);
    Ok((git, root, store))
}

fn job_id(arg: &str) -> &str {
    arg.strip_prefix("ork/").unwrap_or(arg)
}

/// `ork jobs`: every job, newest first.
pub async fn jobs(ctx: &CliContext) -> anyhow::Result<CommandResult> {
    let (_, _, store) = open(ctx).await?;
    let jobs = store.list()?;
    if jobs.is_empty() {
        ctx.say("no jobs yet (ork run starts one)");
    }
    for job in &jobs {
        let files = match store.load_report(&job.id) {
            Ok(Some(report)) => match report.files_changed.len() {
                1 => "1 file".to_string(),
                n => format!("{n} files"),
            },
            _ => "-".to_string(),
        };
        ctx.say(&format!(
            "{}  {:<13}  {:>4}  {:>8}  {}",
            job.id,
            job.status.as_str(),
            age(job),
            files,
            cut(job.goal.lines().next().unwrap_or_default(), 60)
        ));
    }
    Ok(CommandResult::ok_with(json!({ "jobs": jobs })))
}

/// `ork status JOB`: the state, the report when there is one, the last event while it runs. The
/// result's status is the job's, with `ork run`'s exit codes, so polling a running job ends with
/// exactly what the run would have returned.
pub async fn status(ctx: &CliContext, args: JobArg) -> anyhow::Result<CommandResult> {
    let (_, _, store) = open(ctx).await?;
    let id = job_id(&args.job);
    let state = store.load(id)?;
    let report = store.load_report(id)?;

    let mut data = json!({ "state": state });
    if state.status == JobStatus::Running
        && let Some(last) = last_event(&store, id)
    {
        data["last_event"] = last;
    }

    ctx.say(&format!(
        "{} · {} · {} · {}",
        state.id,
        state.status.as_str(),
        state.branch,
        age(&state)
    ));
    ctx.say(&state.goal);
    if let Some(reason) = &state.reason {
        ctx.say(&format!("reason: {reason}"));
    }
    if let Some(report) = &report {
        if !report.summary.is_empty() {
            ctx.say(&report.summary);
        }
        ctx.say(&format!(
            "{} turns · {} repairs · {} in / {} out",
            report.turns, report.repairs_used, report.tokens.input, report.tokens.output
        ));
    }

    let code = match state.status {
        JobStatus::FailedChecks | JobStatus::GaveUp => exit::CHECKS_FAILED,
        JobStatus::Error => exit::ERROR,
        _ => exit::OK,
    };
    let mut result = CommandResult::new(state.status.as_str(), code).with_data(data);
    result.job = Some(state.id.clone());
    result.report = report;
    if state.status == JobStatus::Error {
        result.error = Some(
            state
                .reason
                .clone()
                .unwrap_or_else(|| "the job failed".into()),
        );
    }
    Ok(result)
}

/// The last line of the job's events.jsonl.
fn last_event(store: &JobStore, id: &str) -> Option<Value> {
    let text = std::fs::read_to_string(store.paths(id).events).ok()?;
    serde_json::from_str(text.lines().last()?).ok()
}

/// `ork diff JOB [--stat]`: what the job changed, from its base to its branch.
pub async fn diff(ctx: &CliContext, args: DiffArgs) -> anyhow::Result<CommandResult> {
    let (git, root, store) = open(ctx).await?;
    let state = store.load(job_id(&args.job))?;
    if matches!(state.status, JobStatus::Applied | JobStatus::Discarded) {
        bail!(
            "job {} was {}; its branch is gone",
            state.id,
            state.status.as_str()
        );
    }
    let diff = git
        .diff(&root, &state.base_sha, &state.branch, args.stat)
        .await?;
    if !ctx.json {
        ctx.stdout.write_str(&diff);
    }
    Ok(CommandResult::ok_with(json!({ "diff": diff })))
}

/// `ork logs JOB`: the transcript, every message to and from the model.
pub async fn logs(ctx: &CliContext, args: JobArg) -> anyhow::Result<CommandResult> {
    let (_, _, store) = open(ctx).await?;
    let state = store.load(job_id(&args.job))?;
    let path = store.paths(&state.id).transcript;
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("job {} has no transcript yet", state.id))?;
    let lines: Vec<Value> = text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    if !ctx.json {
        for line in &lines {
            for human in readable(line) {
                ctx.say(&human);
            }
        }
    }
    Ok(CommandResult::ok_with(json!({ "lines": lines })))
}

/// One transcript line, for people: `#3 assistant → edit_file {"path": …}`, `#3 tool ← edited …`.
fn readable(line: &Value) -> Vec<String> {
    let turn = line["turn"].as_u64().unwrap_or(0);
    let content = line["content"]
        .as_str()
        .map(|c| cut(c.lines().next().unwrap_or_default(), 120));
    match line["role"].as_str().unwrap_or_default() {
        "meta" => vec![format!(
            "#{turn} meta {} · {}→{} tok · {:.1}s",
            line["finish_reason"].as_str().unwrap_or("?"),
            line["tokens_in"],
            line["tokens_out"],
            line["seconds"].as_f64().unwrap_or(0.0)
        )],
        "assistant" => {
            let mut out: Vec<String> = content
                .into_iter()
                .filter(|c| !c.is_empty())
                .map(|c| format!("#{turn} assistant {c}"))
                .collect();
            for call in line["tool_calls"].as_array().into_iter().flatten() {
                let name = call["function"]["name"].as_str().unwrap_or("?");
                let args = call["function"]["arguments"]
                    .as_str()
                    .and_then(short_args)
                    .map(|a| a.to_string())
                    .unwrap_or_default();
                out.push(format!("#{turn} assistant → {name} {}", cut(&args, 120)));
            }
            out
        }
        "tool" => vec![format!("#{turn} tool ← {}", content.unwrap_or_default())],
        role => vec![format!("#{turn} {role} {}", content.unwrap_or_default())],
    }
}

/// `ork apply JOB`: merge the job's branch into the current branch, then clean up.
pub async fn apply(ctx: &CliContext, args: ApplyArgs) -> anyhow::Result<CommandResult> {
    let (git, root, store) = open(ctx).await?;
    let state = store.load(job_id(&args.job))?;
    match state.status {
        JobStatus::Passed => {}
        JobStatus::FailedChecks | JobStatus::GaveUp if args.force => {}
        JobStatus::FailedChecks | JobStatus::GaveUp => bail!(
            "job {} is {}; look at it first (ork diff {}), then apply it with --force if you want it",
            state.id,
            state.status.as_str(),
            state.id
        ),
        other => bail!(
            "job {} is {}; only a finished job can be applied",
            state.id,
            other.as_str()
        ),
    }
    if !git.is_clean(&root).await? {
        bail!("your checkout has uncommitted changes; commit or stash them first");
    }
    let current = git.current_branch(&root).await?;
    if current == state.branch {
        bail!("you're on the job's branch ({current}); switch to the branch to apply it to");
    }

    let message = format!(
        "ork({}): {}",
        state.id,
        state.goal.lines().next().unwrap_or_default().trim()
    );
    git.merge(&root, &state.branch, args.squash, &message)
        .await?;
    let commit = git.head_sha(&root).await?;

    // Landed: the worktree and the branch have done their job. The job's files stay, for the logs.
    let worktree = store.worktree_path(&state.id);
    if worktree.exists() {
        git.worktree_remove(&root, &worktree).await?;
    }
    git.branch_delete(&root, &state.branch, true).await?;
    store.set_status(&state.id, JobStatus::Applied, None)?;

    let how = if args.squash {
        "squashed into"
    } else {
        "merged into"
    };
    ctx.say(&format!(
        "{} {how} {current} ({})",
        state.id,
        &commit[..commit.len().min(10)]
    ));
    Ok(CommandResult::ok_with(
        json!({ "job": state.id, "branch": current, "commit": commit, "squash": args.squash }),
    ))
}

/// `ork discard JOB`: drop the worktree and the branch. Safe to repeat.
pub async fn discard(ctx: &CliContext, args: JobArg) -> anyhow::Result<CommandResult> {
    let (git, root, store) = open(ctx).await?;
    let state = store.load(job_id(&args.job))?;
    if state.status == JobStatus::Applied {
        bail!(
            "job {} was applied already; there's nothing to discard",
            state.id
        );
    }
    let worktree = store.worktree_path(&state.id);
    if worktree.exists() {
        git.worktree_remove(&root, &worktree).await?;
    }
    // Gone already is fine: discard is idempotent.
    let _ = git.branch_delete(&root, &state.branch, true).await;
    store.set_status(&state.id, JobStatus::Discarded, state.reason.clone())?;
    ctx.say(&format!("{} discarded", state.id));
    Ok(CommandResult::ok_with(json!({ "job": state.id })))
}

fn age(job: &JobState) -> String {
    let seconds = (Utc::now() - job.created_at).num_seconds().max(0);
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m", seconds / 60),
        3600..86_400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        let head: String = text.chars().take(max - 1).collect();
        format!("{head}…")
    } else {
        text.to_string()
    }
}
