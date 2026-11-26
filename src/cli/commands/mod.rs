//! One module per command. Each has an `Args` struct (clap) and
//! `async fn run(ctx: &CliContext, args) -> anyhow::Result<CommandResult>`.

pub mod ask;
pub mod doctor;
pub mod guide;
pub mod init;
pub mod jobs;
pub mod models;
pub mod run;
pub mod skill;
pub mod tui;

use crate::cli::CliContext;
use crate::config::{self, LoadedConfig};
use crate::events::OrkEvent;

/// Loads the config for this command and reports its warnings (e.g. an API key ignored) as
/// `warning` events.
pub fn load_config(ctx: &CliContext) -> anyhow::Result<LoadedConfig> {
    let loaded = config::load(&ctx.cwd, &ctx.env)?;
    for warning in &loaded.warnings {
        ctx.emit(OrkEvent::Warning {
            code: "key_ignored".into(),
            message: warning.clone(),
        });
    }
    Ok(loaded)
}

/// A file path someone typed, relative to where they are (`cwd`), as a path relative to the repo
/// root with `/` (what orders store). `None` when it isn't a file under `cwd`, or isn't inside the
/// repo: the caller then takes it as relative to the root already.
pub fn repo_relative(path: &str, cwd: &std::path::Path, root: &std::path::Path) -> Option<String> {
    let file = dunce::canonicalize(cwd.join(path)).ok()?;
    if !file.is_file() {
        return None;
    }
    let root = dunce::canonicalize(root).ok()?;
    let rel = file.strip_prefix(&root).ok()?;
    Some(
        rel.components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
    )
}
