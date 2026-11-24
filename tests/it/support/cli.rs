//! Runs the CLI in-process and captures what it wrote.

use std::path::Path;

use orkestr8r::cli::{self, CliIo};
use orkestr8r::env::Env;
use tokio_util::sync::CancellationToken;

pub struct CliOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// `ork ARGS…` in `cwd` with `env`.
pub async fn run_cli(args: &[&str], cwd: &Path, env: Env) -> CliOutput {
    let (io, stdout, stderr) = CliIo::capture();
    let argv = std::iter::once("ork")
        .chain(args.iter().copied())
        .map(String::from)
        .collect();
    let code = cli::run(argv, io, cwd.to_path_buf(), env, CancellationToken::new()).await;
    CliOutput {
        code,
        stdout: stdout.text(),
        stderr: stderr.text(),
    }
}
