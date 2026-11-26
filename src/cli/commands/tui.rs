//! `ork tui` (and `ork` alone, in a terminal): the live view of this repo's jobs and models, for
//! people. The first time, when no agent has the skill yet, it asks which agents you use.

use anyhow::{Context, bail};
use tokio::runtime::Handle;

use crate::cli::commands::load_config;
use crate::cli::{CliContext, CommandResult};
use crate::git::Git;
use crate::jobs::JobStore;
use crate::process::ProcessRunner;
use crate::skill::Installed;
use crate::tui::app::{App, Setup};
use crate::tui::theme::ThemeName;
use crate::tui::{self, Ork};

pub async fn run(ctx: &CliContext) -> anyhow::Result<CommandResult> {
    if ctx.json {
        bail!(
            "ork tui is for people; agents read the same with `ork jobs --json` and `ork status JOB --json`"
        );
    }
    if !ctx.terminal {
        bail!("ork tui needs a terminal");
    }
    let config = load_config(ctx)?.config;
    let git = Git::new(ProcessRunner::new(ctx.env.clone()));
    let root = git.repo_root(&ctx.cwd).await.context(
        "the dashboard shows a repo's jobs: run ork in a git repository (your project's)",
    )?;

    let ork = Ork {
        handle: Handle::current(),
        cwd: ctx.cwd.clone(),
        env: ctx.env.clone(),
        cancel: ctx.cancel.clone(),
    };
    let mut app = App::new(
        JobStore::new(&root),
        Setup::from_config(&config),
        theme(ctx),
    );
    app.repo = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    app.skills = ork.skills();
    // First run: no agent knows to delegate yet. Ask which ones are used.
    if !app.skills.is_empty()
        && app
            .skills
            .iter()
            .all(|target| target.installed == Installed::No)
    {
        app.open_picker();
    }

    // The terminal loop blocks, so it gets a thread of its own; its actions call back into the
    // runtime through the handle.
    tokio::task::spawn_blocking(move || tui::run(app, ork))
        .await
        .context("the TUI stopped")??;
    Ok(CommandResult::ok())
}

/// https://no-color.org: set and not empty means no colours.
pub fn theme(ctx: &CliContext) -> ThemeName {
    match ctx.env.get("NO_COLOR") {
        Some(value) if !value.is_empty() => ThemeName::Plain,
        _ => ThemeName::Dark,
    }
}
