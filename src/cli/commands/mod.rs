//! One module per command. Each has an `Args` struct (clap) and
//! `async fn run(ctx: &CliContext, args) -> anyhow::Result<CommandResult>`.

pub mod init;
