use orkestr8r::cli::exit;
use orkestr8r::env::Env;

use crate::support::cli::run_cli;

#[tokio::test]
async fn version_prints_ork_0_1_0() {
    let out = run_cli(&["--version"], &std::env::temp_dir(), Env::default()).await;

    assert_eq!(out.code, exit::OK);
    assert!(
        out.stdout.starts_with("ork 0.1.0"),
        "stdout: {}",
        out.stdout
    );
}

#[tokio::test]
async fn an_unknown_flag_exits_1_with_a_message_on_stderr() {
    let out = run_cli(&["--nope"], &std::env::temp_dir(), Env::default()).await;

    assert_eq!(out.code, exit::ERROR);
    assert!(out.stdout.is_empty());
    assert!(out.stderr.contains("--nope"), "stderr: {}", out.stderr);
}

#[test]
fn env_names_are_case_insensitive_on_windows_only() {
    let env = Env::default().with("Path", "x");
    assert_eq!(env.get("PATH").is_some(), cfg!(windows));
    assert_eq!(env.get("Path"), Some("x"));
}
