use orkestr8r::cli::exit;
use orkestr8r::env::Env;
use serde_json::{Value, json};

use crate::support::cli::run_cli;
use crate::support::fake_server::{FakeModelServer, Script, unreachable_url};
use crate::support::workspace::{Workspace, json_lines};

fn checks(stdout: &str) -> Vec<Value> {
    let result = json_lines(stdout).pop().unwrap();
    assert_eq!(result["event"], "result");
    result["data"]["checks"].as_array().unwrap().clone()
}

fn check<'a>(checks: &'a [Value], name: &str) -> &'a Value {
    checks
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no {name} check"))
}

#[tokio::test]
async fn all_checks_pass() {
    let server =
        FakeModelServer::start(Script::new().tool_call("ping", json!({ "value": "pong" }))).await;
    let ws = Workspace::new(&server.base_url(), &["git status"]);

    let out = run_cli(&["doctor", "--json"], &ws.repo, ws.env.clone()).await;

    assert_eq!(out.code, exit::OK, "{}", out.stdout);
    let checks = checks(&out.stdout);
    let names: Vec<_> = checks.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        ["config", "git", "server", "tool_calling", "commands"]
    );
    assert!(checks.iter().all(|c| c["ok"] == true), "{checks:#?}");
    assert!(
        check(&checks, "server").get("hint").is_none(),
        "fake-model is listed"
    );

    // one GET /models, then the ping request with exactly one tool
    let ping = &server.requests()[0];
    assert_eq!(ping["tools"].as_array().unwrap().len(), 1);
    assert_eq!(ping["tools"][0]["function"]["name"], "ping");
    assert_eq!(
        ping["messages"][0]["content"],
        "Call the ping tool with value \"pong\"."
    );
}

#[tokio::test]
async fn a_text_reply_fails_tool_calling_with_a_hint() {
    let server = FakeModelServer::start(Script::new().text("pong")).await;
    let ws = Workspace::new(&server.base_url(), &["git status"]);

    let out = run_cli(&["doctor", "--json"], &ws.repo, ws.env.clone()).await;

    assert_eq!(out.code, exit::ERROR);
    let checks = checks(&out.stdout);
    let tool_calling = check(&checks, "tool_calling");
    assert_eq!(tool_calling["ok"], false);
    assert!(
        tool_calling["hint"]
            .as_str()
            .unwrap()
            .contains("chat template")
    );
    assert_eq!(check(&checks, "server")["ok"], true);
}

#[tokio::test]
async fn a_server_that_is_down_fails_server_and_tool_calling() {
    let ws = Workspace::new(&unreachable_url().await, &["git status"]);

    let out = run_cli(&["doctor", "--json"], &ws.repo, ws.env.clone()).await;

    assert_eq!(out.code, exit::ERROR);
    let checks = checks(&out.stdout);
    assert_eq!(check(&checks, "server")["ok"], false);
    assert_eq!(check(&checks, "tool_calling")["ok"], false);
    assert_eq!(check(&checks, "config")["ok"], true);
    let result = json_lines(&out.stdout).pop().unwrap();
    assert_eq!(result["error"], "2 of 5 checks failed");
}

#[tokio::test]
async fn a_missing_command_fails_its_check() {
    let server =
        FakeModelServer::start(Script::new().tool_call("ping", json!({ "value": "pong" }))).await;
    let ws = Workspace::new(&server.base_url(), &["git status", "no-such-tool-x9 test"]);

    let out = run_cli(&["doctor", "--json"], &ws.repo, ws.env.clone()).await;

    assert_eq!(out.code, exit::ERROR);
    let commands = check(&checks(&out.stdout), "commands").clone();
    assert_eq!(commands["ok"], false);
    assert_eq!(commands["detail"], "not on PATH: no-such-tool-x9");
}

#[tokio::test]
async fn no_config_at_all_says_run_init() {
    let ws = Workspace::new("http://unused/v1", &[]);
    let empty = ws
        .dir
        .path()
        .join("empty-config-home")
        .display()
        .to_string();
    // the real environment, minus any ORK_CONFIG, with the user file in an empty directory
    let process = Env::from_process();
    let env = Env::from_pairs(process.iter().filter(|(name, _)| *name != "ORK_CONFIG"))
        .with("APPDATA", &empty)
        .with("XDG_CONFIG_HOME", &empty);

    let out = run_cli(&["doctor", "--json"], &ws.repo, env).await;

    assert_eq!(out.code, exit::ERROR);
    let checks = checks(&out.stdout);
    let config = check(&checks, "config");
    assert_eq!(config["detail"], "no ork.json found");
    assert!(config["hint"].as_str().unwrap().contains("ork init"));
}

#[tokio::test]
async fn doctor_prints_a_checklist_for_people() {
    let server = FakeModelServer::start(Script::new().text("pong")).await;
    let ws = Workspace::new(&server.base_url(), &["git status"]);

    let out = run_cli(&["doctor"], &ws.repo, ws.env.clone()).await;

    assert_eq!(out.code, exit::ERROR);
    assert!(out.stdout.contains("✓ config"), "{}", out.stdout);
    assert!(out.stdout.contains("✗ tool_calling"), "{}", out.stdout);
    assert!(
        out.stdout.contains("hint: the model replied with text"),
        "{}",
        out.stdout
    );
    assert_eq!(out.stderr, "error: 1 of 5 checks failed\n");
}

#[tokio::test]
async fn models_lists_the_roles() {
    let server = FakeModelServer::start(Script::new()).await;
    let ws = Workspace::new(&server.base_url(), &[]);

    let out = run_cli(&["models", "--json"], &ws.repo, ws.env.clone()).await;

    assert_eq!(out.code, exit::OK, "{}", out.stdout);
    let result = json_lines(&out.stdout).pop().unwrap();
    let grunt = &result["data"]["roles"][0];
    assert_eq!(grunt["role"], "grunt");
    assert_eq!(grunt["server"], "box");
    assert_eq!(grunt["url"], server.base_url());
    assert_eq!(grunt["model"], "fake-model");
    assert_eq!(grunt["key_set"], false);
    assert!(grunt.get("test").is_none());
    assert!(server.requests().is_empty(), "no request without --test");
}

#[tokio::test]
async fn models_test_sends_a_tiny_request() {
    let server = FakeModelServer::start(Script::new().text("OK")).await;
    let ws = Workspace::new(&server.base_url(), &[]);

    let out = run_cli(&["models", "--test", "--json"], &ws.repo, ws.env.clone()).await;

    assert_eq!(out.code, exit::OK, "{}", out.stdout);
    let result = json_lines(&out.stdout).pop().unwrap();
    let test = &result["data"]["roles"][0]["test"];
    assert_eq!(test["ok"], true);
    assert_eq!(
        (test["tokens_in"].as_u64(), test["tokens_out"].as_u64()),
        (Some(100), Some(20))
    );
    assert_eq!(
        server.requests()[0]["messages"][0]["content"],
        "Reply with OK."
    );
}
