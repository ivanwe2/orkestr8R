use std::process::ExitCode;

use orkestr8r::cli::{self, CliIo};
use orkestr8r::env::Env;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> ExitCode {
    // Ctrl+C cancels the token; everything long-running watches it and winds down.
    let cancel = CancellationToken::new();
    let on_ctrl_c = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            on_ctrl_c.cancel();
        }
    });

    let args = std::env::args().collect();
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let code = cli::run(args, CliIo::stdio(), cwd, Env::from_process(), cancel).await;
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
