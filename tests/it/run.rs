use orkestr8r::cli::exit;
use serde_json::{Value, json};

use crate::support::cli::run_cli;
use crate::support::fake_server::{FakeModelServer, Script};
use crate::support::workspace::{Workspace, json_lines};

const CALC: &str = "pub fn one() -> i32 {\n    1\n}\n";
const CHECK: &str = "git grep --untracked -q Sum -- src/calc.rs";

fn add_sum() -> Value {
    json!({ "path": "src/calc.rs", "old": "    1\n}", "new": "    1\n}\n\npub fn Sum(a: i32, b: i32) -> i32 { a + b }" })
}

/// A workspace whose repo has one commit holding src/calc.rs.
fn workspace(server: &FakeModelServer) -> Workspace {
    let ws = Workspace::new(&server.base_url(), &["git grep"]);
    ws.write("src/calc.rs", CALC);
    ws.commit("initial");
    ws
}

fn run_args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "run",
        "--goal",
        "Add Sum to src/calc.rs",
        "--scope",
        "src/**",
        "--check",
        CHECK,
    ];
    args.extend_from_slice(extra);
    args
}

#[tokio::test]
async fn a_passing_run_streams_json_from_start_to_result() {
    let server = FakeModelServer::start(
        Script::new()
            .tool_call("edit_file", add_sum())
            .tool_call("finish", json!({ "summary": "Added Sum" })),
    )
    .await;
    let ws = workspace(&server);

    let out = run_cli(&run_args(&["--json"]), &ws.repo, ws.env.clone()).await;

    assert_eq!(out.code, exit::OK, "{}\n{}", out.stdout, out.stderr);
    let events = json_lines(&out.stdout);
    assert_eq!(events[0]["event"], "start");
    let job = events[0]["job"].as_str().unwrap().to_string();
    assert_eq!(events[0]["branch"], format!("ork/{job}"));
    assert_eq!(events[0]["worktree"], format!(".ork/worktrees/{job}"));
    assert_eq!(events[0]["model"], "fake-model");
    assert_eq!(events[0]["server"], "box");
    let result = events.last().unwrap();
    assert_eq!(result["event"], "result");
    assert_eq!(result["status"], "passed");
    assert_eq!(result["exit_code"], 0);
    assert_eq!(result["job"], job);
    assert_eq!(result["report"]["summary"], "Added Sum");
    assert_eq!(
        result["report"]["files_changed"],
        json!([{ "path": "src/calc.rs", "added": 2, "removed": 0 }])
    );
    assert_eq!(result["report"]["checks"][0]["exit"], 0);

    // the job's files, and its events log from start to result
    let job_dir = ws.repo.join(".ork/jobs").join(&job);
    assert!(
        ws.repo
            .join(".ork/worktrees")
            .join(&job)
            .join("src/calc.rs")
            .is_file(),
        "the worktree is kept"
    );
    let logged = json_lines(&std::fs::read_to_string(job_dir.join("events.jsonl")).unwrap());
    assert_eq!(logged.first().unwrap()["event"], "start");
    assert_eq!(logged.last().unwrap()["event"], "result");
    assert!(job_dir.join("report.json").is_file() && job_dir.join("transcript.jsonl").is_file());
    // .ork/ stays out of git status
    let exclude = std::fs::read_to_string(ws.repo.join(".git/info/exclude")).unwrap();
    assert!(exclude.lines().any(|l| l == ".ork/"));
}

#[tokio::test]
async fn failing_checks_exit_4() {
    let server =
        FakeModelServer::start(Script::new().tool_call("finish", json!({ "summary": "Done" })))
            .await;
    let ws = workspace(&server);

    let out = run_cli(
        &run_args(&["--max-repairs", "0", "--json"]),
        &ws.repo,
        ws.env.clone(),
    )
    .await;

    assert_eq!(out.code, exit::CHECKS_FAILED, "{}", out.stdout);
    let result = json_lines(&out.stdout).pop().unwrap();
    assert_eq!(result["status"], "failed_checks");
    assert_eq!(result["report"]["reason"], format!("{CHECK} exited 1"));
}

#[tokio::test]
async fn an_invalid_order_exits_1_with_every_problem() {
    let server = FakeModelServer::start(Script::new()).await;
    let ws = workspace(&server);

    let out = run_cli(
        &[
            "run",
            "--path",
            "missing.rs",
            "--check",
            "rm -rf src",
            "--json",
        ],
        &ws.repo,
        ws.env.clone(),
    )
    .await;

    assert_eq!(out.code, exit::ERROR);
    let result = json_lines(&out.stdout).pop().unwrap();
    let error = result["error"].as_str().unwrap();
    for problem in [
        "goal: required",
        "write_scope: required",
        "paths: missing.rs not found",
        "checks: rm -rf src: refused",
    ] {
        assert!(error.contains(problem), "{problem:?} not in {error}");
    }
    assert!(server.requests().is_empty(), "nothing ran");
    assert!(!ws.repo.join(".ork/jobs").exists(), "no job was made");
}

#[tokio::test]
async fn flags_override_the_order_file() {
    let server = FakeModelServer::start(
        Script::new().tool_call("read_file", json!({ "path": "src/calc.rs" })),
    )
    .await;
    let ws = workspace(&server);
    let file = ws.dir().join("order.json");
    let order = json!({
        "goal": "From the file",
        "write_scope": ["src/**"],
        "checks": [CHECK],
        "max_turns": 30
    });
    std::fs::write(&file, order.to_string()).unwrap();
    let file = file.display().to_string();

    let out = run_cli(
        &[
            "run",
            &file,
            "--goal",
            "From the flag",
            "--max-turns",
            "1",
            "--json",
        ],
        &ws.repo,
        ws.env.clone(),
    )
    .await;

    assert_eq!(out.code, exit::CHECKS_FAILED, "{}", out.stdout);
    let result = json_lines(&out.stdout).pop().unwrap();
    assert_eq!(result["status"], "gave_up");
    assert_eq!(result["report"]["reason"], "max_turns");
    assert_eq!(result["report"]["turns"], 1);
    let first_message = server.requests()[0]["messages"][1]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        first_message.starts_with("## Goal\nFrom the flag\n"),
        "{first_message}"
    );
    assert!(first_message.contains(CHECK), "the file's checks are kept");
}

#[tokio::test]
async fn a_dirty_checkout_is_a_warning_after_start() {
    let server = FakeModelServer::start(
        Script::new()
            .tool_call("edit_file", add_sum())
            .tool_call("finish", json!({ "summary": "ok" })),
    )
    .await;
    let ws = workspace(&server);
    ws.write("notes.txt", "not committed\n");

    let out = run_cli(&run_args(&["--json"]), &ws.repo, ws.env.clone()).await;

    assert_eq!(out.code, exit::OK, "{}", out.stdout);
    let events = json_lines(&out.stdout);
    assert_eq!(events[0]["event"], "start");
    assert_eq!(events[1]["event"], "warning");
    assert_eq!(events[1]["code"], "dirty_checkout");
}

#[tokio::test]
async fn people_get_progress_on_stderr_and_the_report_on_stdout() {
    let server = FakeModelServer::start(
        Script::new()
            .tool_call("edit_file", add_sum())
            .tool_call("finish", json!({ "summary": "Added Sum" })),
    )
    .await;
    let ws = workspace(&server);

    let out = run_cli(&run_args(&[]), &ws.repo, ws.env.clone()).await;

    assert_eq!(out.code, exit::OK, "{}", out.stderr);
    let lines: Vec<&str> = out.stdout.lines().collect();
    let job = lines[0].split(" · ").nth(1).unwrap();
    assert_eq!(
        lines,
        [
            format!("passed · {job} · ork/{job}").as_str(),
            "Added Sum",
            "src/calc.rs  +2 -0",
            format!("✓ {CHECK}").as_str(),
            format!("next: ork diff {job} · ork apply {job} · ork discard {job}").as_str(),
        ]
    );
    assert!(
        out.stderr.starts_with(&format!(
            "ork run {job} · grunt fake-model on box · .ork/worktrees/{job}\n"
        )),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("  t1  edit_file src/calc.rs   ok"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains(&format!("  ✓ {CHECK}")),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr
            .contains("passed · 2 turns · 0 repairs · 200 in / 40 out · 1 tool call"),
        "{}",
        out.stderr
    );
}

#[tokio::test]
async fn outside_a_repository_it_is_an_error() {
    let server = FakeModelServer::start(Script::new()).await;
    let ws = workspace(&server);
    let outside = ws.dir().join("not-a-repo");
    std::fs::create_dir_all(&outside).unwrap();

    let out = run_cli(&run_args(&[]), &outside, ws.env.clone()).await;

    assert_eq!(out.code, exit::ERROR);
    assert!(
        out.stderr.contains("not inside a git repository"),
        "{}",
        out.stderr
    );
}
