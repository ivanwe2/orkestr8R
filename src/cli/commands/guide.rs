//! `ork guide`: the guide for AI agents, built into the binary so it always matches the commands
//! it describes (a test keeps it, the skill and docs/cli.md in step).

use serde_json::json;

use crate::cli::{CliContext, CommandResult};

pub const GUIDE: &str = include_str!("../guide.md");

pub async fn run(ctx: &CliContext) -> anyhow::Result<CommandResult> {
    if !ctx.json {
        ctx.stdout.write_str(GUIDE);
    }
    Ok(CommandResult::ok_with(json!({ "guide": GUIDE })))
}
