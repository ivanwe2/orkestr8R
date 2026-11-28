use orkestr8r::cli::exit;

use crate::support::cli::run_cli;
use crate::support::fake_server::{FakeModelServer, Script};
use crate::support::workspace::{Workspace, json_lines};

#[tokio::test]
async fn the_request_carries_the_numbered_files() {
    let server = FakeModelServer::start(Script::new().text("Sum is at src/calc.rs:2.")).await;
    let ws = Workspace::new(&server.base_url(), &[]);
    ws.write(
        "src/calc.rs",
        "pub struct Calc;\npub fn sum(a: i32, b: i32) -> i32 { a + b }\n",
    );

    let out = run_cli(
        &["ask", "Where is sum?", "--path", "src/calc.rs"],
        &ws.repo,
        ws.env.clone(),
    )
    .await;

    assert_eq!(out.code, exit::OK, "{}", out.stderr);
    let request = &server.requests()[0];
    assert!(request.get("tools").is_none(), "no tools for ask");
    assert_eq!(
        request["messages"][0]["content"],
        "Answer the question about the files below. Be concise. Cite file:line where it helps."
    );
    let user = request["messages"][1]["content"].as_str().unwrap();
    assert!(user.starts_with("Where is sum?\n"), "{user}");
    assert!(
        user.contains("## src/calc.rs\n```\n    1 │ pub struct Calc;\n    2 │ pub fn sum"),
        "{user}"
    );
}

#[tokio::test]
async fn the_answer_is_printed_without_think_blocks() {
    let server =
        FakeModelServer::start(Script::new().text("<think>it's on line 2</think>Line 2.")).await;
    let ws = Workspace::new(&server.base_url(), &[]);
    ws.write("a.txt", "x\n");

    let out = run_cli(
        &["ask", "Where?", "--path", "a.txt"],
        &ws.repo,
        ws.env.clone(),
    )
    .await;

    assert_eq!(out.code, exit::OK, "{}", out.stderr);
    assert_eq!(out.stdout, "Line 2.\n");
}

#[tokio::test]
async fn with_json_the_answer_is_in_data() {
    let server = FakeModelServer::start(Script::new().text("Line 2.")).await;
    let ws = Workspace::new(&server.base_url(), &[]);
    ws.write("a.txt", "x\n");

    let out = run_cli(
        &["ask", "Where?", "--path", "a.txt", "--json"],
        &ws.repo,
        ws.env.clone(),
    )
    .await;

    assert_eq!(out.code, exit::OK);
    let events = json_lines(&out.stdout);
    assert_eq!(events[0]["event"], "call");
    assert_eq!(events[0]["server"], "box");
    let data = &events.last().unwrap()["data"];
    assert_eq!(data["answer"], "Line 2.");
    assert_eq!(data["model"], "fake-model");
    assert_eq!(
        (data["tokens_in"].as_u64(), data["tokens_out"].as_u64()),
        (Some(100), Some(20))
    );
}

#[tokio::test]
async fn big_files_are_cut_with_a_warning() {
    let server = FakeModelServer::start(Script::new().text("ok")).await;
    let ws = Workspace::new(&server.base_url(), &[]);
    let long: String = (1..=500).map(|i| format!("line {i}\n")).collect();
    ws.write("long.txt", &long);

    let out = run_cli(
        &["ask", "?", "--path", "long.txt", "--json"],
        &ws.repo,
        ws.env.clone(),
    )
    .await;

    assert_eq!(out.code, exit::OK);
    let warning = &json_lines(&out.stdout)[0];
    assert_eq!(warning["event"], "warning");
    assert_eq!(warning["code"], "truncated");
    let user = server.requests()[0]["messages"][1]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        user.contains("  400 │ line 400\n```"),
        "capped at 400 lines"
    );
    assert!(!user.contains("line 401"));
}

#[tokio::test]
async fn a_missing_file_exits_1_before_asking() {
    let server = FakeModelServer::start(Script::new().text("never")).await;
    let ws = Workspace::new(&server.base_url(), &[]);

    let out = run_cli(&["ask", "?", "--path", "nope.rs"], &ws.repo, ws.env.clone()).await;

    assert_eq!(out.code, exit::ERROR);
    assert_eq!(out.stderr, "error: not found: nope.rs\n");
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn paths_work_from_a_subdirectory_either_way() {
    let server = FakeModelServer::start(Script::new().text("ok")).await;
    let ws = Workspace::new(&server.base_url(), &[]);
    ws.write("src/calc.rs", "pub fn one() -> i32 { 1 }\n");
    let subdir = ws.repo.join("src");

    // relative to where you are, and relative to the repo root: both found
    let out = run_cli(
        &["ask", "?", "--path", "calc.rs", "--path", "src/calc.rs"],
        &subdir,
        ws.env.clone(),
    )
    .await;

    assert_eq!(out.code, exit::OK, "{}", out.stderr);
    let user = server.requests()[0]["messages"][1]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        user.contains("## calc.rs\n") && user.contains("## src/calc.rs\n"),
        "{user}"
    );
}

#[tokio::test]
async fn ask_and_doctor_take_a_role() {
    let a = FakeModelServer::start(Script::new()).await;
    let b = FakeModelServer::start(
        Script::new()
            .text("Sum is at src/calc.rs:2.")
            .tool_call("ping", serde_json::json!({ "value": "pong" })),
    )
    .await;
    let ws = Workspace::with_config(serde_json::json!({
        "servers": { "a": { "url": a.base_url() }, "b": { "url": b.base_url() } },
        "roles": {
            "grunt": { "server": "a", "model": "model-a" },
            "goblin": { "server": "b", "model": "model-b" }
        },
    }));
    ws.write(
        "src/calc.rs",
        "pub fn sum(a: i32, b: i32) -> i32 { a + b }\n",
    );

    let out = run_cli(
        &[
            "ask",
            "Where is sum?",
            "--path",
            "src/calc.rs",
            "--role",
            "goblin",
        ],
        &ws.repo,
        ws.env.clone(),
    )
    .await;
    assert_eq!(out.code, exit::OK, "{}", out.stderr);

    let doctor = run_cli(
        &["doctor", "--role", "goblin", "--json"],
        &ws.repo,
        ws.env.clone(),
    )
    .await;
    let checks = json_lines(&doctor.stdout).pop().unwrap()["data"]["checks"].clone();
    let tool_calling = checks
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "tool_calling")
        .unwrap()
        .clone();
    assert_eq!(tool_calling["ok"], true, "{checks}");
    assert!(
        a.requests().is_empty(),
        "nothing went to the grunt's server"
    );
    assert!(b.requests().len() >= 2);
}
