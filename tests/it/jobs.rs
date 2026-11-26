use orkestr8r::cli::exit;
use orkestr8r::jobs::JobStore;
use orkestr8r::orders::{JobStatus, Order};
use serde_json::{Value, json};

use crate::support::cli::run_cli;
use crate::support::fake_server::{FakeModelServer, Script};
use crate::support::temp_repo::git;
use crate::support::workspace::{Workspace, json_lines};

const CALC: &str = "pub fn one() -> i32 {\n    1\n}\n";
const WITH_SUM: &str =
    "pub fn one() -> i32 {\n    1\n}\n\npub fn Sum(a: i32, b: i32) -> i32 { a + b }\n";
const CHECK: &str = "git grep --untracked -q Sum -- src/calc.rs";

fn add_sum() -> Value {
    json!({ "path": "src/calc.rs", "old": "    1\n}", "new": "    1\n}\n\npub fn Sum(a: i32, b: i32) -> i32 { a + b }" })
}

/// A repo with one commit, and one `ork run` of `script` in it. Returns the job id.
async fn one_job(script: Script, extra: &[&str]) -> (Workspace, FakeModelServer, String) {
    let server = FakeModelServer::start(script).await;
    let ws = Workspace::new(&server.base_url(), &["git grep"]);
    ws.write("src/calc.rs", CALC);
    ws.commit("initial");
    let mut args = vec![
        "run",
        "--goal",
        "Add Sum to src/calc.rs",
        "--scope",
        "src/**",
        "--check",
        CHECK,
        "--json",
    ];
    args.extend_from_slice(extra);
    let out = run_cli(&args, &ws.repo, ws.env.clone()).await;
    let job = json_lines(&out.stdout)[0]["job"]
        .as_str()
        .unwrap()
        .to_string();
    (ws, server, job)
}

pub(crate) async fn passing_job() -> (Workspace, FakeModelServer, String) {
    one_job(
        Script::new()
            .tool_call("edit_file", add_sum())
            .tool_call("finish", json!({ "summary": "Added Sum" })),
        &[],
    )
    .await
}

async fn ork(ws: &Workspace, args: &[&str]) -> (i32, Vec<Value>, String) {
    let mut args = args.to_vec();
    args.push("--json");
    let out = run_cli(&args, &ws.repo, ws.env.clone()).await;
    (out.code, json_lines(&out.stdout), out.stderr)
}

#[tokio::test]
async fn jobs_lists_newest_first() {
    let (ws, _server, job) = passing_job().await;

    let (code, events, _) = ork(&ws, &["jobs"]).await;
    assert_eq!(code, exit::OK);
    let jobs = events.last().unwrap()["data"]["jobs"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        (jobs[0]["id"].as_str(), jobs[0]["status"].as_str()),
        (Some(job.as_str()), Some("passed"))
    );

    let table = run_cli(&["jobs"], &ws.repo, ws.env.clone()).await.stdout;
    assert!(table.starts_with(&format!("{job}  passed")), "{table}");
    assert!(
        table.contains("1 file") && table.contains("Add Sum to src/calc.rs"),
        "{table}"
    );
}

#[tokio::test]
async fn status_gives_back_what_the_run_returned() {
    let (ws, _server, job) = passing_job().await;

    let (code, events, _) = ork(&ws, &["status", &job]).await;

    assert_eq!(code, exit::OK);
    let result = events.last().unwrap();
    assert_eq!(result["status"], "passed");
    assert_eq!(result["job"], job.as_str());
    assert_eq!(result["report"]["summary"], "Added Sum");
    assert_eq!(result["data"]["state"]["branch"], format!("ork/{job}"));
}

#[tokio::test]
async fn status_of_a_running_job_shows_its_last_event() {
    let (ws, _server, _) = passing_job().await;
    let store = JobStore::new(&ws.repo);
    let running = store
        .create(
            &Order {
                goal: "Still going".into(),
                ..Default::default()
            },
            "abc",
        )
        .unwrap();
    std::fs::write(store.paths(&running.id).events, "{\"event\":\"call\",\"turn\":7}\n{\"event\":\"tool\",\"turn\":7,\"name\":\"run\",\"ok\":true,\"ms\":900}\n").unwrap();

    let (code, events, _) = ork(&ws, &["status", &format!("ork/{}", running.id)]).await;

    assert_eq!(code, exit::OK);
    let result = events.last().unwrap();
    assert_eq!(result["status"], "running");
    assert_eq!(result["data"]["last_event"]["name"], "run");
    assert!(result.get("report").is_none());
}

#[tokio::test]
async fn diff_shows_the_changes() {
    let (ws, _server, job) = passing_job().await;

    let (code, events, _) = ork(&ws, &["diff", &job, "--stat"]).await;
    assert_eq!(code, exit::OK);
    let stat = events.last().unwrap()["data"]["diff"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        stat.contains("src/calc.rs") && stat.contains("1 file changed"),
        "{stat}"
    );

    let full = run_cli(&["diff", &job], &ws.repo, ws.env.clone())
        .await
        .stdout;
    assert!(
        full.contains("+pub fn Sum(a: i32, b: i32) -> i32 { a + b }"),
        "{full}"
    );
}

#[tokio::test]
async fn logs_read_like_a_conversation() {
    let (ws, _server, job) = passing_job().await;

    let out = run_cli(&["logs", &job], &ws.repo, ws.env.clone()).await;

    assert_eq!(out.code, exit::OK);
    assert!(
        out.stdout.contains("#0 system You are a coding agent"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("#1 assistant → edit_file {\"new\":"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("#1 tool ← edited src/calc.rs (-2 +4 lines)"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("#2 meta tool_calls · 100→20 tok"),
        "{}",
        out.stdout
    );

    let (_, events, _) = ork(&ws, &["logs", &job]).await;
    let lines = events.last().unwrap()["data"]["lines"]
        .as_array()
        .unwrap()
        .len();
    assert!(lines >= 8, "{lines}");
}

#[tokio::test]
async fn the_models_text_cannot_drive_the_terminal() {
    let script = Script::new().tool_call("edit_file", add_sum()).tool_call(
        "finish",
        json!({ "summary": "Added Sum\u{1b}]0;pwned\u{7}\u{1b}[2J" }),
    );
    let (ws, _server, job) = one_job(script, &[]).await;

    let status = run_cli(&["status", &job], &ws.repo, ws.env.clone())
        .await
        .stdout;
    let logs = run_cli(&["logs", &job], &ws.repo, ws.env.clone())
        .await
        .stdout;

    for out in [&status, &logs] {
        assert!(!out.contains('\u{1b}') && !out.contains('\u{7}'), "{out:?}");
    }
    // the summary line: escaped so it shows; in the logs it's inside the call's JSON arguments,
    // which serde escaped already
    assert!(
        status.contains(r"Added Sum\u{1b}]0;pwned\u{7}\u{1b}[2J"),
        "{status:?}"
    );
    assert!(logs.contains(r"Added Sum\u001b]0;pwned\u0007"), "{logs:?}");
}

#[tokio::test]
async fn apply_merges_and_cleans_up() {
    let (ws, _server, job) = passing_job().await;

    let (code, events, stderr) = ork(&ws, &["apply", &job]).await;

    assert_eq!(code, exit::OK, "{stderr}");
    assert_eq!(
        std::fs::read_to_string(ws.repo.join("src/calc.rs")).unwrap(),
        WITH_SUM
    );
    let parents = git(&ws.repo, &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(parents.split_whitespace().count(), 3, "a merge commit");
    assert_eq!(
        git(&ws.repo, &["log", "-1", "--format=%s"]).trim(),
        format!("ork({job}): Add Sum to src/calc.rs")
    );
    assert!(!ws.repo.join(".ork/worktrees").join(&job).exists());
    assert!(
        git(&ws.repo, &["branch", "--list", &format!("ork/{job}")])
            .trim()
            .is_empty()
    );
    assert_eq!(
        JobStore::new(&ws.repo).load(&job).unwrap().status,
        JobStatus::Applied
    );
    assert_eq!(events.last().unwrap()["data"]["squash"], false);

    // only once
    let (again, events, _) = ork(&ws, &["apply", &job]).await;
    assert_eq!(again, exit::ERROR);
    assert!(
        events.last().unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("is applied")
    );
}

#[tokio::test]
async fn apply_cuts_a_long_goal_in_the_merge_subject() {
    let server = FakeModelServer::start(
        Script::new()
            .tool_call("edit_file", add_sum())
            .tool_call("finish", json!({ "summary": "Added Sum" })),
    )
    .await;
    let ws = Workspace::new(&server.base_url(), &["git grep"]);
    ws.write("src/calc.rs", CALC);
    ws.commit("initial");
    let goal = format!("Add Sum to src/calc.rs {}", "and keep going ".repeat(10));
    let args = [
        "run", "--goal", &goal, "--scope", "src/**", "--check", CHECK, "--json",
    ];
    let out = run_cli(&args, &ws.repo, ws.env.clone()).await;
    let job = json_lines(&out.stdout)[0]["job"]
        .as_str()
        .unwrap()
        .to_string();

    let (code, _, stderr) = ork(&ws, &["apply", &job]).await;

    assert_eq!(code, exit::OK, "{stderr}");
    let subject = git(&ws.repo, &["log", "-1", "--format=%s"]);
    let subject = subject.trim();
    let prefix = format!("ork({job}): ");
    assert!(
        subject.starts_with(&format!("{prefix}Add Sum")),
        "{subject}"
    );
    assert!(subject.ends_with('…'), "{subject}");
    assert_eq!(
        subject.chars().count(),
        prefix.chars().count() + 72,
        "{subject}"
    );
}

#[tokio::test]
async fn apply_squash_makes_one_plain_commit() {
    let (ws, _server, job) = passing_job().await;

    let (code, _, stderr) = ork(&ws, &["apply", &job, "--squash"]).await;

    assert_eq!(code, exit::OK, "{stderr}");
    assert_eq!(
        std::fs::read_to_string(ws.repo.join("src/calc.rs")).unwrap(),
        WITH_SUM
    );
    let parents = git(&ws.repo, &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(parents.split_whitespace().count(), 2, "one parent");
    assert_eq!(
        git(&ws.repo, &["log", "-1", "--format=%s"]).trim(),
        format!("ork({job}): Add Sum to src/calc.rs")
    );
    assert!(
        git(&ws.repo, &["branch", "--list", &format!("ork/{job}")])
            .trim()
            .is_empty()
    );
}

#[tokio::test]
async fn apply_refuses_a_dirty_checkout() {
    let (ws, _server, job) = passing_job().await;
    ws.write("src/calc.rs", "// my own edit\n");

    let (code, events, _) = ork(&ws, &["apply", &job]).await;

    assert_eq!(code, exit::ERROR);
    assert!(
        events.last().unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("uncommitted changes")
    );
    assert_eq!(
        std::fs::read_to_string(ws.repo.join("src/calc.rs")).unwrap(),
        "// my own edit\n"
    );
    assert_eq!(
        JobStore::new(&ws.repo).load(&job).unwrap().status,
        JobStatus::Passed
    );
}

#[tokio::test]
async fn a_failed_job_needs_force() {
    let script = Script::new()
        .tool_call(
            "write_file",
            json!({ "path": "src/other.rs", "content": "x\n" }),
        )
        .tool_call("finish", json!({ "summary": "Done" }));
    let (ws, _server, job) = one_job(script, &["--max-repairs", "0"]).await;
    assert_eq!(
        JobStore::new(&ws.repo).load(&job).unwrap().status,
        JobStatus::FailedChecks
    );

    let (code, events, _) = ork(&ws, &["apply", &job]).await;
    assert_eq!(code, exit::ERROR);
    assert!(
        events.last().unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("--force")
    );

    let (forced, _, stderr) = ork(&ws, &["apply", &job, "--force"]).await;
    assert_eq!(forced, exit::OK, "{stderr}");
    assert!(ws.repo.join("src/other.rs").is_file());
}

#[tokio::test]
async fn a_conflict_aborts_and_changes_nothing() {
    let (ws, _server, job) = passing_job().await;
    ws.write(
        "src/calc.rs",
        "pub fn one() -> i32 {\n    1\n}\n\npub fn Sum() {}\n",
    );
    ws.commit("my own Sum");
    let head = git(&ws.repo, &["rev-parse", "HEAD"]);

    let (code, events, _) = ork(&ws, &["apply", &job]).await;

    assert_eq!(code, exit::ERROR);
    assert!(
        events.last().unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("conflict")
    );
    assert_eq!(git(&ws.repo, &["rev-parse", "HEAD"]), head);
    assert!(
        git(&ws.repo, &["status", "--porcelain"])
            .lines()
            .all(|l| l.contains(".ork"))
    );
    assert_eq!(
        JobStore::new(&ws.repo).load(&job).unwrap().status,
        JobStatus::Passed,
        "still appliable later"
    );
}

#[tokio::test]
async fn discard_is_safe_to_repeat() {
    let (ws, _server, job) = passing_job().await;

    let (first, _, stderr) = ork(&ws, &["discard", &job]).await;
    let (second, _, _) = ork(&ws, &["discard", &job]).await;

    assert_eq!((first, second), (exit::OK, exit::OK), "{stderr}");
    assert!(!ws.repo.join(".ork/worktrees").join(&job).exists());
    assert!(
        git(&ws.repo, &["branch", "--list", &format!("ork/{job}")])
            .trim()
            .is_empty()
    );
    let store = JobStore::new(&ws.repo);
    assert_eq!(store.load(&job).unwrap().status, JobStatus::Discarded);
    assert!(
        store.paths(&job).transcript.is_file(),
        "the job's files stay, for the logs"
    );
    assert_eq!(
        std::fs::read_to_string(ws.repo.join("src/calc.rs")).unwrap(),
        CALC
    );
}

#[tokio::test]
async fn an_unknown_job_is_an_error() {
    let (ws, _server, _) = passing_job().await;

    let (code, events, _) = ork(&ws, &["status", "zzzz"]).await;

    assert_eq!(code, exit::ERROR);
    assert!(
        events.last().unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("no job zzzz")
    );
}

#[tokio::test]
async fn a_read_only_job_cleans_up_after_itself_and_has_nothing_to_apply() {
    let server = FakeModelServer::start(
        Script::new()
            .tool_call("read_file", json!({ "path": "src/calc.rs" }))
            .tool_call(
                "finish",
                json!({ "summary": "One function: one(), src/calc.rs:1" }),
            ),
    )
    .await;
    let ws = Workspace::new(&server.base_url(), &["git grep"]);
    ws.write("src/calc.rs", CALC);
    ws.commit("initial");

    let out = run_cli(
        &[
            "run",
            "--goal",
            "Which functions does src/calc.rs have?",
            "--readonly",
            "--json",
        ],
        &ws.repo,
        ws.env.clone(),
    )
    .await;

    assert_eq!(out.code, exit::OK, "{}", out.stderr);
    let events = json_lines(&out.stdout);
    let job = events[0]["job"].as_str().unwrap().to_string();
    let report = &events.last().unwrap()["report"];
    assert_eq!(report["files_read"], json!(["src/calc.rs"]));
    assert_eq!(report["tool_calls"], 1);
    assert!(!ws.repo.join(".ork/worktrees").join(&job).exists());
    let branches = git(&ws.repo, &["branch", "--list", &format!("ork/{job}")]);
    assert!(branches.trim().is_empty(), "{branches}");

    let (code, events, _) = ork(&ws, &["apply", &job]).await;
    assert_eq!(code, exit::ERROR);
    let error = events.last().unwrap()["error"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        error.contains("read-only") && error.contains("nothing to apply"),
        "{error}"
    );
    let (code, events, _) = ork(&ws, &["diff", &job]).await;
    assert_eq!(code, exit::ERROR);
    assert!(
        events.last().unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("changed nothing")
    );
    // discard still works: it marks the job done
    let (code, _, _) = ork(&ws, &["discard", &job]).await;
    assert_eq!(code, exit::OK);
}

#[tokio::test]
async fn job_commands_work_from_inside_a_jobs_worktree() {
    let (ws, _server, job) = passing_job().await;
    let inside = ws.repo.join(".ork/worktrees").join(&job);
    assert!(inside.is_dir());

    let out = run_cli(&["status", &job, "--json"], &inside, ws.env.clone()).await;
    assert_eq!(out.code, exit::OK, "{}", out.stderr);
    assert_eq!(json_lines(&out.stdout).last().unwrap()["status"], "passed");

    let out = run_cli(&["jobs", "--json"], &inside, ws.env.clone()).await;
    let jobs = json_lines(&out.stdout).last().unwrap()["data"]["jobs"].clone();
    assert_eq!(jobs.as_array().unwrap().len(), 1, "{jobs}");
    assert!(
        !inside.join(".ork").exists(),
        "no nested .ork/ in the worktree"
    );
}
