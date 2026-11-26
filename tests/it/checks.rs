use orkestr8r::config::{OrkConfig, RoleConfig};
use orkestr8r::env::Env;
use orkestr8r::events::OrkEvent;
use orkestr8r::git::Git;
use orkestr8r::jobs::{JobState, JobStore};
use orkestr8r::orders::{FileChange, JobStatus, Order, OrderMode, Report};
use orkestr8r::process::ProcessRunner;
use orkestr8r::runs::RunService;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::support::events::CollectSink;
use crate::support::fake_server::{FakeModelServer, Script};
use crate::support::temp_repo::TempRepo;

const CALC: &str = "pub fn one() -> i32 {\n    1\n}\n";
const CHECK: &str = "git grep --untracked -q Sum -- src/calc.rs";

fn order() -> Order {
    Order {
        goal: "Add Sum to src/calc.rs\nWith two i32 arguments.".into(),
        write_scope: vec!["src/**".into()],
        checks: vec![CHECK.into()],
        max_turns: Some(10),
        max_repairs: Some(2),
        ..Default::default()
    }
}

/// The edit that makes the check pass.
fn add_sum() -> Value {
    json!({ "path": "src/calc.rs", "old": "    1\n}", "new": "    1\n}\n\npub fn Sum(a: i32, b: i32) -> i32 { a + b }" })
}

struct JobRun {
    report: Report,
    job: JobState,
    store: JobStore,
    repo: TempRepo,
    server: FakeModelServer,
    events: Vec<OrkEvent>,
}

/// A repo, a job with its worktree on `ork/<id>`, and one run of `order` against `script`.
async fn run_job(script: Script, order: Order) -> JobRun {
    let repo = TempRepo::with_files(&[("src/calc.rs", CALC)]);
    let env = Env::from_process();
    let git = Git::new(ProcessRunner::new(env.clone()));
    git.add_exclude(&repo.path, ".ork/").await.unwrap();
    let base = git.head_sha(&repo.path).await.unwrap();
    let store = JobStore::new(&repo.path);
    let job = store.create(&order, &base).unwrap();
    git.worktree_add(
        &repo.path,
        &store.worktree_path(&job.id),
        &job.branch,
        &base,
    )
    .await
    .unwrap();

    let server = FakeModelServer::start(script).await;
    let client = server.client();
    let config = OrkConfig {
        allow_commands: vec!["git grep".into()],
        ..Default::default()
    };
    let role = RoleConfig {
        server: "box".into(),
        model: "fake-model".into(),
        ..Default::default()
    };
    let sink = CollectSink::default();
    let service = RunService {
        model: &client,
        config: &config,
        role: &role,
        env: &env,
        events: &sink,
    };

    let report = service
        .run(&order, &job, &store, CancellationToken::new())
        .await
        .unwrap();
    JobRun {
        report,
        job,
        store,
        repo,
        server,
        events: sink.events(),
    }
}

fn check_exits(events: &[OrkEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            OrkEvent::Check { exit, .. } => Some(format!("check {exit}")),
            OrkEvent::Repair { attempt } => Some(format!("repair {attempt}")),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_failing_check_becomes_a_repair_turn() {
    let script = Script::new()
        .tool_call("finish", json!({ "summary": "Done, I think" }))
        .tool_call("edit_file", add_sum())
        .tool_call("finish", json!({ "summary": "Added Sum" }));

    let run = run_job(script, order()).await;

    assert_eq!(run.report.status, JobStatus::Passed, "{:?}", run.report);
    assert_eq!(run.report.repairs_used, 1);
    assert_eq!(run.report.turns, 3);
    assert_eq!(run.report.summary, "Added Sum");
    assert_eq!(run.report.reason, None);
    assert_eq!(check_exits(&run.events), ["check 1", "repair 1", "check 0"]);
    assert_eq!(run.report.checks.len(), 1);
    assert_eq!(
        (run.report.checks[0].cmd.as_str(), run.report.checks[0].exit),
        (CHECK, 0)
    );

    // the rejection reached the model as the first finish's result
    let second = run.server.requests()[1]["messages"]
        .as_array()
        .unwrap()
        .clone();
    let feedback = second[3]["content"].as_str().unwrap();
    assert_eq!(second[3]["tool_call_id"], "call_1");
    assert!(
        feedback.starts_with(&format!("Checks failed. {CHECK} exited 1:\n")),
        "{feedback}"
    );
    assert!(
        feedback.ends_with("Fix the problem, then call finish again."),
        "{feedback}"
    );
}

#[tokio::test]
async fn the_work_is_committed_on_the_job_branch() {
    let script = Script::new()
        .tool_call("edit_file", add_sum())
        .tool_call("finish", json!({ "summary": "Added Sum" }));

    let run = run_job(script, order()).await;

    assert_eq!(run.report.status, JobStatus::Passed);
    assert_eq!(run.report.branch, format!("ork/{}", run.job.id));
    let subject = run
        .repo
        .git(&["log", "-1", "--format=%s", &run.report.branch]);
    assert_eq!(
        subject.trim(),
        format!("ork({}): Add Sum to src/calc.rs", run.job.id)
    );
    assert_eq!(
        run.report.files_changed,
        [FileChange {
            path: "src/calc.rs".into(),
            added: 2,
            removed: 0
        }]
    );
    // the counts: one tool call (finish isn't one); files_read is for read-only orders
    assert_eq!(
        (run.report.tool_calls, run.report.failed_tool_calls),
        (1, 0)
    );
    assert!(run.report.files_read.is_empty());
    assert!(run.report.duration_s >= 0.0);
    // the user's checkout is untouched
    assert_eq!(run.repo.read("src/calc.rs"), CALC);
}

#[tokio::test]
async fn when_repairs_run_out_the_failure_is_reported() {
    let script = Script::new()
        .tool_call("finish", json!({ "summary": "Done" }))
        .tool_call("finish", json!({ "summary": "Really done" }));
    let order = Order {
        max_repairs: Some(1),
        ..order()
    };

    let run = run_job(script, order).await;

    assert_eq!(run.report.status, JobStatus::FailedChecks);
    assert_eq!(run.report.repairs_used, 1);
    assert_eq!(run.report.reason, Some(format!("{CHECK} exited 1")));
    assert_eq!(check_exits(&run.events), ["check 1", "repair 1", "check 1"]);
    assert_eq!(run.report.checks[0].exit, 1);
}

#[tokio::test]
async fn no_repairs_means_the_first_failure_ends_it() {
    let script = Script::new().tool_call("finish", json!({ "summary": "Done" }));
    let order = Order {
        max_repairs: Some(0),
        ..order()
    };

    let run = run_job(script, order).await;

    assert_eq!(run.report.status, JobStatus::FailedChecks);
    assert_eq!(run.report.repairs_used, 0);
}

#[tokio::test]
async fn a_readonly_order_answers_in_its_summary() {
    let script = Script::new()
        .tool_call("grep", json!({ "pattern": "fn " }))
        .tool_call("read_file", json!({ "path": "src/calc.rs" }))
        .tool_call(
            "finish",
            json!({ "summary": "One function: one() in src/calc.rs:1." }),
        );
    let order = Order {
        goal: "Which functions does src/calc.rs have?".into(),
        mode: OrderMode::Readonly,
        ..Default::default()
    };

    let run = run_job(script, order).await;

    assert_eq!(run.report.status, JobStatus::Passed);
    assert_eq!(run.report.summary, "One function: one() in src/calc.rs:1.");
    assert!(run.report.checks.is_empty());
    assert!(run.report.files_changed.is_empty());
    // readonly orders aren't offered the writing tools
    let tools = run.server.requests()[0]["tools"].as_array().unwrap().len();
    assert_eq!(tools, 4);
    // what it read, for the chief to judge how far it looked
    assert_eq!(run.report.files_read, ["src/calc.rs"]);
    assert_eq!(run.report.tool_calls, 2);
    // nothing to apply: the worktree and the branch are gone, the report stays
    assert!(!run.store.worktree_path(&run.job.id).exists());
    assert!(
        run.repo
            .git(&["branch", "--list", &run.report.branch])
            .trim()
            .is_empty()
    );
    assert!(run.store.load_report(&run.job.id).unwrap().is_some());
}

#[tokio::test]
async fn giving_up_is_reported_with_the_reason_and_the_work_kept() {
    let script = Script::new()
        .tool_call("edit_file", add_sum())
        .tool_call("read_file", json!({ "path": "src/calc.rs" }));
    let order = Order {
        max_turns: Some(2),
        ..order()
    };

    let run = run_job(script, order).await;

    assert_eq!(run.report.status, JobStatus::GaveUp);
    assert_eq!(run.report.reason.as_deref(), Some("max_turns"));
    assert_eq!(
        run.report.files_changed.len(),
        1,
        "committed anyway, for inspection"
    );
}

#[tokio::test]
async fn the_report_and_the_status_are_stored() {
    let script = Script::new()
        .tool_call("edit_file", add_sum())
        .tool_call("finish", json!({ "summary": "Added Sum" }));

    let run = run_job(script, order()).await;

    let state = run.store.load(&run.job.id).unwrap();
    assert_eq!(state.status, JobStatus::Passed);
    assert_eq!(
        run.store.load_report(&run.job.id).unwrap(),
        Some(run.report.clone())
    );
    let transcript = std::fs::read_to_string(run.store.paths(&run.job.id).transcript).unwrap();
    assert!(transcript.lines().count() >= 6, "{transcript}");
    assert_eq!(
        (run.report.tokens.input, run.report.tokens.output),
        (200, 40)
    );
    assert_eq!(run.report.model, "fake-model");
}

#[tokio::test]
async fn a_model_error_is_an_error_status() {
    let run = run_job(Script::new().status(401), order()).await;

    assert_eq!(run.report.status, JobStatus::Error);
    assert!(run.report.reason.as_deref().unwrap().contains("401"));
}

#[tokio::test]
async fn a_check_that_failed_before_the_job_is_said_so() {
    // The grunt changes nothing; the check (`Sum` exists) fails on the base commit too.
    let script = Script::new().tool_call("finish", json!({ "summary": "Done" }));
    let order = Order {
        max_repairs: Some(0),
        ..order()
    };

    let run = run_job(script, order).await;

    assert_eq!(run.report.status, JobStatus::FailedChecks);
    assert_eq!(run.report.failing_before, [CHECK]);
    assert!(
        run.events.iter().any(|e| matches!(
            e,
            OrkEvent::Warning { code, .. } if code == "already_failing"
        )),
        "{:?}",
        run.events
    );
    // the baseline's own check run isn't on the job's track, and its worktree is gone
    assert_eq!(check_exits(&run.events), ["check 1"]);
    assert!(
        !run.repo
            .path
            .join(".ork/baseline")
            .join(&run.job.id)
            .exists()
    );
}

#[tokio::test]
async fn a_check_the_grunt_broke_isnt_blamed_on_the_base() {
    // `one` is there on the base commit; the grunt renames it away.
    let script = Script::new()
        .tool_call(
            "edit_file",
            json!({ "path": "src/calc.rs", "old": "pub fn one()", "new": "pub fn two()" }),
        )
        .tool_call("finish", json!({ "summary": "Renamed" }));
    let order = Order {
        checks: vec!["git grep --untracked -q one -- src/calc.rs".into()],
        max_repairs: Some(0),
        ..order()
    };

    let run = run_job(script, order).await;

    assert_eq!(run.report.status, JobStatus::FailedChecks);
    assert!(run.report.failing_before.is_empty(), "{:?}", run.report);
}
