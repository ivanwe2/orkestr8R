use std::time::{Duration, Instant};

use orkestr8r::env::Env;
use orkestr8r::process::{CommandPolicy, ProcessRunner, ProcessSpec, head_tail, split};
use tokio_util::sync::CancellationToken;

const TESTTOOL: &str = env!("CARGO_BIN_EXE_ork-testtool");

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn split_handles_quotes_and_the_escape() {
    assert_eq!(
        split("cargo test --test it").unwrap(),
        strings(&["cargo", "test", "--test", "it"])
    );
    assert_eq!(split("  a   b  ").unwrap(), strings(&["a", "b"]));
    assert_eq!(
        split(r#"echo "a b" 'c d'"#).unwrap(),
        strings(&["echo", "a b", "c d"])
    );
    assert_eq!(split(r#"--name="a b""#).unwrap(), strings(&["--name=a b"]));
    assert_eq!(
        split(r#"say "he said \"hi\"""#).unwrap(),
        strings(&["say", r#"he said "hi""#])
    );
    assert_eq!(split(r#"x "" y"#).unwrap(), strings(&["x", "", "y"]));
    assert_eq!(
        split(r"dir C:\work\src").unwrap(),
        strings(&["dir", r"C:\work\src"])
    );
    assert_eq!(split("").unwrap(), Vec::<String>::new());
}

#[test]
fn an_unclosed_quote_is_an_error() {
    assert!(split(r#"echo "a b"#).is_err());
    assert!(split("echo 'a b").is_err());
}

#[test]
fn the_policy_allows_by_token_prefix() {
    let policy = CommandPolicy::new(&strings(&["cargo test", "git grep"]));

    assert_eq!(
        policy.check("cargo test").unwrap(),
        strings(&["cargo", "test"])
    );
    assert_eq!(
        policy.check("cargo test --test it tools::").unwrap(),
        strings(&["cargo", "test", "--test", "it", "tools::"])
    );
    assert_eq!(
        policy.check(r#"git grep -q "fn sum""#).unwrap(),
        strings(&["git", "grep", "-q", "fn sum"])
    );

    let refused = policy.check("cargo run").unwrap_err();
    assert!(
        refused.contains("not allowed") && refused.contains("cargo test, git grep"),
        "{refused}"
    );
    // a token prefix, not a string prefix
    assert!(policy.check("cargo testx").is_err());
    assert!(policy.check("cargo").is_err());
}

#[test]
fn the_policy_refuses_every_metacharacter() {
    let policy = CommandPolicy::new(&strings(&["cargo test"]));

    for cmd in [
        "cargo test; rm -rf /",
        "cargo test && curl x",
        "cargo test | tee out",
        "cargo test < in",
        "cargo test > out",
        "cargo test $HOME",
        "cargo test `id`",
        "cargo test\nrm x",
        "cargo test\rrm x",
    ] {
        let reason = policy.check(cmd).unwrap_err();
        assert!(reason.contains("metacharacter"), "{cmd:?}: {reason}");
    }
}

#[test]
fn an_empty_allow_list_says_so() {
    let reason = CommandPolicy::new(&[]).check("cargo test").unwrap_err();
    assert!(reason.contains("allow_commands"), "{reason}");
}

fn runner() -> ProcessRunner {
    ProcessRunner::new(Env::from_process().with("SECRET_X", "leaked"))
}

fn spec(args: &[&str], timeout: Duration) -> ProcessSpec {
    let mut argv = vec![TESTTOOL.to_string()];
    argv.extend(strings(args));
    ProcessSpec {
        argv,
        cwd: std::env::temp_dir(),
        timeout,
    }
}

#[tokio::test]
async fn the_exit_code_comes_back() {
    let result = runner()
        .run(
            spec(&["exit", "3"], Duration::from_secs(30)),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(result.exit_code, 3);
    assert!(!result.timed_out);
}

#[tokio::test]
async fn the_output_comes_back() {
    let result = runner()
        .run(
            spec(&["print", "5"], Duration::from_secs(30)),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.output.lines().collect::<Vec<_>>(),
        ["line 1", "line 2", "line 3", "line 4", "line 5"]
    );
}

#[tokio::test]
async fn huge_output_keeps_the_head_and_the_tail() {
    let result = runner()
        .run(
            spec(&["print", "200000"], Duration::from_secs(60)),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

    assert!(
        result.output.len() < 1024 * 1024 + 100,
        "{}",
        result.output.len()
    );
    assert!(result.output.starts_with("line 1\nline 2\n"));
    assert!(result.output.ends_with("line 199999\nline 200000\n"));
    assert!(result.output.contains("bytes cut]…"));
}

#[tokio::test]
async fn a_timeout_kills_it() {
    let started = Instant::now();

    let result = runner()
        .run(
            spec(&["sleep", "30"], Duration::from_secs(1)),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

    assert!(result.timed_out);
    assert_eq!(result.exit_code, -1);
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_cancel_kills_it_and_is_an_error() {
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        trigger.cancel();
    });
    let started = Instant::now();

    let err = runner()
        .run(spec(&["sleep", "30"], Duration::from_secs(60)), &cancel)
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "cancelled");
    assert!(started.elapsed() < Duration::from_secs(15));
}

#[tokio::test]
async fn secrets_are_not_passed_on() {
    let result = runner()
        .run(
            spec(&["env", "SECRET_X"], Duration::from_secs(30)),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(
        result.output, "",
        "SECRET_X is in ork's environment, not the command's"
    );
}

#[tokio::test]
async fn commands_run_in_ci_mode() {
    let result = runner()
        .run(
            spec(&["env", "CI"], Duration::from_secs(30)),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(result.output, "true");
}

#[test]
fn only_allowed_variables_are_passed() {
    let runner = ProcessRunner::new(
        Env::default()
            .with("PATH", "/bin")
            .with("DOTNET_ROOT", "/dotnet")
            .with("CARGO_TARGET_DIR", "/cache/target")
            .with("CARGO_REGISTRY_TOKEN", "secret")
            .with("NUGET_AUTH_TOKEN", "secret")
            .with("OPENAI_API_KEY", "secret"),
    );

    let names: Vec<String> = runner
        .child_env()
        .into_iter()
        .map(|(name, _)| name)
        .collect();

    for passed in ["PATH", "DOTNET_ROOT", "CARGO_TARGET_DIR"] {
        assert!(names.contains(&passed.to_string()), "{passed} not passed");
    }
    for secret in ["CARGO_REGISTRY_TOKEN", "NUGET_AUTH_TOKEN", "OPENAI_API_KEY"] {
        assert!(!names.contains(&secret.to_string()), "{secret} leaked");
    }
}

#[tokio::test]
async fn a_missing_program_is_an_error() {
    let spec = ProcessSpec {
        argv: strings(&["no-such-program-x9"]),
        cwd: std::env::temp_dir(),
        timeout: Duration::from_secs(5),
    };

    let err = runner()
        .run(spec, &CancellationToken::new())
        .await
        .unwrap_err();

    assert!(err.to_string().contains("not found on PATH"), "{err}");
}

#[test]
fn head_tail_keeps_both_ends_and_is_safe_on_multibyte_text() {
    assert_eq!(head_tail("short", 4000), "short");

    let text = "é".repeat(10_000);
    let cut = head_tail(&text, 4000);

    assert!(cut.starts_with(&"é".repeat(2000)));
    assert!(cut.ends_with(&"é".repeat(2000)));
    assert!(cut.contains("…[6000 chars cut]…"));
}
