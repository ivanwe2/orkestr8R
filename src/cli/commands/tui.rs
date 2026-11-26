//! `ork tui`: the live view of this repo's jobs and models, for people.

use std::io::IsTerminal;

use anyhow::{Context, bail};
use tokio::runtime::Handle;

use crate::cli::commands::load_config;
use crate::cli::{CliContext, CommandResult};
use crate::git::Git;
use crate::jobs::JobStore;
use crate::process::ProcessRunner;
use crate::tui::app::{App, Setup};
use crate::tui::theme::ThemeName;
use crate::tui::{self, Ork};

pub async fn run(ctx: &CliContext) -> anyhow::Result<CommandResult> {
    if ctx.json {
        bail!(
            "ork tui is for people; agents read the same with `ork jobs --json` and `ork status JOB --json`"
        );
    }
    if !std::io::stdout().is_terminal() {
        bail!("ork tui needs a terminal");
    }
    let config = load_config(ctx)?.config;
    let git = Git::new(ProcessRunner::new(ctx.env.clone()));
    let root = git.repo_root(&ctx.cwd).await?;

    // https://no-color.org: set and not empty means no colours.
    let theme = match ctx.env.get("NO_COLOR") {
        Some(value) if !value.is_empty() => ThemeName::Plain,
        _ => ThemeName::Dark,
    };
    let app = App::new(JobStore::new(&root), Setup::from_config(&config), theme);
    let ork = Ork {
        handle: Handle::current(),
        cwd: ctx.cwd.clone(),
        env: ctx.env.clone(),
        cancel: ctx.cancel.clone(),
    };
    // The terminal loop blocks, so it gets a thread of its own; its actions call back into the
    // runtime through the handle.
    tokio::task::spawn_blocking(move || tui::run(app, ork))
        .await
        .context("the TUI stopped")??;
    Ok(CommandResult::ok())
}
