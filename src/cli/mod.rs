//! The `ork` command line. [`run`] is the whole CLI, callable in-process: `main` only forwards to
//! it, so tests run commands with their own writers, directory and environment.

pub mod exit;

use std::path::PathBuf;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};
use tokio_util::sync::CancellationToken;

use crate::env::Env;
use crate::io::{Captured, SharedWriter};

#[derive(Debug, Parser)]
#[command(
    name = "ork",
    version,
    about = "orkestr8R: run coding orders on a local model, in git worktrees, checked with real commands.",
    arg_required_else_help = true
)]
pub struct Cli {}

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
    _cwd: PathBuf,
    _env: Env,
    _cancel: CancellationToken,
) -> i32 {
    match Cli::try_parse_from(&args) {
        Ok(_cli) => exit::OK,
        Err(err) => parse_error(&io, err),
    }
}

/// clap reports `--help` and `--version` as "errors" too. Those print to stdout and exit 0; real
/// parse errors print to stderr and exit 1 (clap's own default is 2; our contract says 1).
fn parse_error(io: &CliIo, err: clap::Error) -> i32 {
    let text = err.render().to_string();
    match err.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            io.stdout.write_str(&text);
            exit::OK
        }
        _ => {
            io.stderr.write_str(&text);
            exit::ERROR
        }
    }
}
