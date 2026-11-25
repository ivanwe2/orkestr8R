//! The `ork` command line. [`run`] is the whole CLI, callable in-process: `main` only forwards to
//! it, so tests run commands with their own writers, directory and environment.

pub mod commands;
pub mod context;
pub mod exit;

use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, Subcommand};
use futures::FutureExt;
use tokio_util::sync::CancellationToken;

pub use context::{CliContext, CommandResult};

use crate::env::Env;
use crate::io::{Captured, SharedWriter};

#[derive(Debug, Parser)]
#[command(
    name = "ork",
    version,
    about = "orkestr8R: run coding orders on a local model, in git worktrees, checked with real commands.",
    arg_required_else_help = true,
    subcommand_required = true,
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Print events as JSON Lines on stdout; the last line is always `result`.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Write ork.json (the repo's, or the user's with --user); add .ork/ to .git/info/exclude
    Init(commands::init::InitArgs),
    /// Is everything in place? Config, git, server, tool calling, allow-listed commands
    Doctor,
    /// Which model does each role, on which server; --test sends a tiny request
    Models(commands::models::ModelsArgs),
    /// A one-shot, read-only answer about files from the local model
    Ask(commands::ask::AskArgs),
    /// Run one order: worktree, grunt loop, checks, report (flags override the file)
    Run(commands::run::RunArgs),
    /// Every job, newest first
    Jobs,
    /// One job: status, the last event while running, the report when done
    Status(commands::jobs::JobArg),
    /// The job's changes
    Diff(commands::jobs::DiffArgs),
    /// The transcript: every message to and from the model
    Logs(commands::jobs::JobArg),
    /// Merge the job's branch into the current branch, then clean up
    Apply(commands::jobs::ApplyArgs),
    /// Drop the worktree and the branch
    Discard(commands::jobs::JobArg),
    /// The guide for AI agents: when to delegate, the order, reading the report
    Guide,
}

/// The clap command tree (T15's sync test enumerates its subcommands).
pub fn command() -> clap::Command {
    Cli::command()
}

/// Where the CLI writes: stdout for results, stderr for people.
#[derive(Clone)]
pub struct CliIo {
    pub stdout: SharedWriter,
    pub stderr: SharedWriter,
}

impl CliIo {
    /// The process's real stdout and stderr.
    pub fn stdio() -> Self {
        Self {
            stdout: SharedWriter::stdout(),
            stderr: SharedWriter::stderr(),
        }
    }

    /// Writers into memory, plus handles to read stdout and stderr back (for tests).
    pub fn capture() -> (Self, Captured, Captured) {
        let (stdout, out) = SharedWriter::capture();
        let (stderr, err) = SharedWriter::capture();
        (Self { stdout, stderr }, out, err)
    }
}

/// Runs `ork` with `args` (the first one is the program name) and returns the exit code.
pub async fn run(
    args: Vec<String>,
    io: CliIo,
    cwd: PathBuf,
    env: Env,
    cancel: CancellationToken,
) -> i32 {
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(err) => return parse_error(&io, err, args.iter().any(|a| a == "--json")),
    };
    let ctx = CliContext::new(io, cwd, env, cancel, cli.json);
    match cli.command {
        Commands::Init(args) => complete(&ctx, commands::init::run(&ctx, args)).await,
        Commands::Doctor => complete(&ctx, commands::doctor::run(&ctx)).await,
        Commands::Models(args) => complete(&ctx, commands::models::run(&ctx, args)).await,
        Commands::Ask(args) => complete(&ctx, commands::ask::run(&ctx, args)).await,
        Commands::Run(args) => complete(&ctx, commands::run::run(&ctx, args)).await,
        Commands::Jobs => complete(&ctx, commands::jobs::jobs(&ctx)).await,
        Commands::Status(args) => complete(&ctx, commands::jobs::status(&ctx, args)).await,
        Commands::Diff(args) => complete(&ctx, commands::jobs::diff(&ctx, args)).await,
        Commands::Logs(args) => complete(&ctx, commands::jobs::logs(&ctx, args)).await,
        Commands::Apply(args) => complete(&ctx, commands::jobs::apply(&ctx, args)).await,
        Commands::Discard(args) => complete(&ctx, commands::jobs::discard(&ctx, args)).await,
        Commands::Guide => complete(&ctx, commands::guide::run(&ctx)).await,
    }
}

/// Runs one command handler and ends the command: exactly one `result` event, last, whatever the
/// handler did. Returns the exit code.
///
/// - `Ok(result)` → that result.
/// - `Err(e)` → status `error`, exit 1, the error's context chain as the message (and the full
///   debug form on stderr with `ORK_DEBUG=1`).
/// - a panic → status `error`, exit 1, `internal error: …`.
pub async fn complete<F>(ctx: &CliContext, handler: F) -> i32
where
    F: Future<Output = anyhow::Result<CommandResult>>,
{
    // `catch_unwind` turns a panic inside the future into an `Err` instead of unwinding past us.
    // `AssertUnwindSafe` is our promise that nothing is left half-updated that we read afterwards.
    let result = match AssertUnwindSafe(handler).catch_unwind().await {
        Ok(Ok(result)) => result,
        Ok(Err(err)) => {
            if ctx.env.get("ORK_DEBUG") == Some("1") {
                ctx.note(&format!("{err:?}"));
            }
            CommandResult::error(format!("{err:#}"))
        }
        Err(panic) => CommandResult::error(format!("internal error: {}", panic_message(&*panic))),
    };
    let code = result.exit_code;
    ctx.emit(result.into_event());
    code
}

fn panic_message(panic: &(dyn Any + Send)) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        message.to_string()
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        "panic".to_string()
    }
}

/// clap reports `--help` and `--version` as "errors" too. Those print to stdout and exit 0. Real
/// parse errors exit 1 (clap's own default is 2; our contract says 1): with `--json` as one
/// `result` line on stdout, otherwise clap's message on stderr.
fn parse_error(io: &CliIo, err: clap::Error, json: bool) -> i32 {
    let text = err.render().to_string();
    match err.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            io.stdout.write_str(&text);
            exit::OK
        }
        _ if json => {
            let result = CommandResult::error(text.trim());
            io.stdout.write_line(&result.into_event().to_line());
            exit::ERROR
        }
        _ => {
            io.stderr.write_str(&text);
            exit::ERROR
        }
    }
}
