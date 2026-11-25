//! One module per command. Each has an `Args` struct (clap) and
//! `async fn run(ctx: &CliContext, args) -> anyhow::Result<CommandResult>`.

pub mod doctor;
pub mod init;
pub mod models;

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
