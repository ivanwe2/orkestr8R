//! `ork run`: one order, start to end. Validate it, make the job and its worktree, run the grunt
//! loop with the checks, and report. Blocks until the job ends (background jobs are v0.2).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, bail};
use clap::Args;

use crate::cli::{CliContext, CommandResult, exit};
use crate::config;
use crate::events::{CompositeSink, EventSink, FileSink, OrkEvent};
use crate::git::Git;
use crate::jobs::JobStore;
use crate::model::ModelClient;
use crate::orders::{self, JobStatus, Order, OrderMode, Report};
use crate::process::{CommandPolicy, ProcessRunner};
use crate::runs::RunService;

#[derive(Debug, Args)]
pub struct RunArgs {
    /// An order file (JSON, see docs/cli.md), or - to read it from stdin; the flags override its
    /// fields
    pub order: Option<PathBuf>,
    /// What to do
    #[arg(long)]
    pub goal: Option<String>,
    /// Constraints and conventions, in plain words
    #[arg(long)]
    pub context: Option<String>,
    /// A file to show the grunt up front, relative to where you are or to the repo root (repeat
    /// for more)
    #[arg(long = "path")]
    pub paths: Vec<String>,
    /// A glob of files the grunt may change, e.g. 'src/**' (repeat for more)
    #[arg(long = "scope")]
    pub scope: Vec<String>,
    /// A command that proves the work is done, e.g. "cargo test" (repeat for more)
    #[arg(long = "check")]
    pub checks: Vec<String>,
    /// An investigation: no writing tools; the answer comes back in the summary
    #[arg(long)]
    pub readonly: bool,
    #[arg(long)]
    pub max_turns: Option<u32>,
    #[arg(long)]
    pub max_repairs: Option<u32>,
    #[arg(long)]
    pub max_minutes: Option<u32>,
}

pub async fn run(ctx: &CliContext, args: RunArgs) -> anyhow::Result<CommandResult> {
    // 1. The repo, the config, the model.
    let git = Git::new(ProcessRunner::new(ctx.env.clone()));
    let root = git.repo_root(&ctx.cwd).await?;
    let loaded = config::load(&ctx.cwd, &ctx.env)?;
    let config = &loaded.config;
    let problems = config.validate();
    if !problems.is_empty() {
        bail!("the config has problems:\n  {}", problems.join("\n  "));
    }
    let (client, role) = ModelClient::for_role(config, "grunt", &ctx.env)?;

    // 2. The order: every problem at once.
    let order = build_order(ctx, &args, &root)?;
    let problems = orders::validate(&order, &root, &CommandPolicy::new(&config.allow_commands));
    if !problems.is_empty() {
        bail!("the order has problems:\n  {}", problems.join("\n  "));
    }

    // 3. The job and its worktree, from HEAD.
    let mut warnings: Vec<(&str, String)> = loaded
        .warnings
        .iter()
        .map(|w| ("key_ignored", w.clone()))
        .collect();
    if !git.is_clean(&root).await? {
        warnings.push((
            "dirty_checkout",
            "your checkout has uncommitted changes; the job starts from HEAD, so they're not in its worktree".into(),
        ));
    }
    git.add_exclude(&root, ".ork/").await?;
    let base = git
        .head_sha(&root)
        .await
        .context("the repository has no commits yet")?;
    let store = JobStore::new(&root);
    let job = store.create(&order, &base)?;
    if let Err(err) = git
        .worktree_add(&root, &store.worktree_path(&job.id), &job.branch, &base)
        .await
    {
        store.set_status(&job.id, JobStatus::Error, Some(format!("{err:#}")))?;
        return Err(err);
    }

    // 4. Events go to the caller and to the job's events.jsonl. `start` comes first.
    let file: Arc<dyn EventSink> = Arc::new(FileSink::new(store.paths(&job.id).events));
    let events = CompositeSink(vec![ctx.events.clone(), file.clone()]);
    events.emit(&OrkEvent::Start {
        command: "run".into(),
        job: Some(job.id.clone()),
        branch: Some(job.branch.clone()),
        worktree: Some(job.worktree.clone()),
        role: Some("grunt".into()),
        model: Some(client.model().into()),
        server: Some(client.server_name().into()),
    });
    for (code, message) in warnings {
        events.emit(&OrkEvent::Warning {
            code: code.into(),
            message,
        });
    }

    // 5. The run.
    let service = RunService {
        model: &client,
        config,
        role,
        env: &ctx.env,
        events: &events,
    };
    let report = match service.run(&order, &job, &store, ctx.cancel.clone()).await {
        Ok(report) => report,
        Err(err) => {
            let _ = store.set_status(&job.id, JobStatus::Error, Some(format!("{err:#}")));
            return Err(err);
        }
    };

    print_summary(ctx, &report, order.mode == OrderMode::Readonly);
    let result = result_for(&report);
    file.emit(&result.clone().into_event());
    Ok(result)
}

/// The order file (if any), with the flags on top.
fn build_order(ctx: &CliContext, args: &RunArgs, root: &Path) -> anyhow::Result<Order> {
    let mut order = match &args.order {
        // PowerShell and long goals: an order on stdin needs no quoting and no temp file.
        Some(path) if path.as_os_str() == "-" => {
            let text = ctx
                .stdin
                .read_all()
                .context("reading the order from stdin")?;
            serde_json::from_str(&text).context("stdin is not a valid order")?
        }
        Some(path) => {
            let path = ctx.cwd.join(path);
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            serde_json::from_str(&text)
                .with_context(|| format!("{} is not a valid order", path.display()))?
        }
        None => Order::default(),
    };
    if let Some(goal) = &args.goal {
        order.goal = goal.clone();
    }
    if let Some(context) = &args.context {
        order.context = Some(context.clone());
    }
    if !args.paths.is_empty() {
        order.paths = args
            .paths
            .iter()
            .map(|path| super::repo_relative(path, &ctx.cwd, root).unwrap_or_else(|| path.clone()))
            .collect();
    }
    if !args.scope.is_empty() {
        order.write_scope = args.scope.clone();
    }
    if !args.checks.is_empty() {
        order.checks = args.checks.clone();
    }
    if args.readonly {
        order.mode = OrderMode::Readonly;
    }
    order.max_turns = args.max_turns.or(order.max_turns);
    order.max_repairs = args.max_repairs.or(order.max_repairs);
    order.max_minutes = args.max_minutes.or(order.max_minutes);
    Ok(order)
}

/// passed → 0; failed_checks or gave_up → 4; error → 1.
fn result_for(report: &Report) -> CommandResult {
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

/// The report for people, on stdout (progress went to stderr as it happened).
fn print_summary(ctx: &CliContext, report: &Report, readonly: bool) {
    let id = &report.job;
    ctx.say(&format!(
        "{} · {id} · {}",
        report.status.as_str(),
        report.branch
    ));
    if !report.summary.is_empty() {
        ctx.say(&report.summary);
    }
    if let Some(reason) = &report.reason {
        ctx.say(&format!("reason: {reason}"));
    }
    for change in &report.files_changed {
        ctx.say(&format!(
            "{}  +{} -{}",
            change.path, change.added, change.removed
        ));
    }
    for check in &report.checks {
        if check.exit == 0 {
            ctx.say(&format!("✓ {}", check.cmd));
        } else {
            ctx.say(&format!("✗ {}  exit {}", check.cmd, check.exit));
        }
    }
    for question in &report.open_questions {
        ctx.say(&format!("? {question}"));
    }
    if readonly {
        ctx.say(&format!(
            "read-only: nothing to apply; its worktree is gone (ork logs {id} for the transcript)"
        ));
    } else if report.status == JobStatus::Passed {
        ctx.say(&format!(
            "next: ork diff {id} · ork apply {id} · ork discard {id}"
        ));
    } else {
        ctx.say(&format!(
            "next: ork diff {id} · ork logs {id} · ork discard {id}"
        ));
    }
}
