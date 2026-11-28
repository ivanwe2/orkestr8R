use orkestr8r::config::OrkConfig;
use orkestr8r::env::Env;
use orkestr8r::events::OrkEvent;
use orkestr8r::grunt::first_message::{self, Seed};
use orkestr8r::grunt::{
    self as grunt, AcceptAll, FinishCall, FinishHandler, FinishVerdict, GruntLoop, LoopEnd,
    LoopOutcome, LoopSettings, Transcript, prompts,
};
use orkestr8r::orders::{Order, OrderMode};
use orkestr8r::tools::{ToolContext, ToolRegistry};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::support::events::CollectSink;
use crate::support::fake_server::{FakeModelServer, Script};
use crate::support::temp_repo::TempRepo;

mod guards;

const CALC: &str = "pub fn one() -> i32 {\n    1\n}\n";

fn order() -> Order {
    Order {
        goal: "Add sum(a, b) to src/calc.rs".into(),
        write_scope: vec!["src/**".into()],
        checks: vec!["git grep -q sum".into()],
        max_turns: Some(10),
        ..Default::default()
    }
}

struct LoopRun {
    outcome: LoopOutcome,
    server: FakeModelServer,
    repo: TempRepo,
    events: Vec<OrkEvent>,
}

impl LoopRun {
    /// The messages of the n-th request (0-based).
    fn messages(&self, request: usize) -> Vec<Value> {
        self.server.requests()[request]["messages"]
            .as_array()
            .unwrap()
            .clone()
    }
}

struct Options {
    order: Order,
    cancel: CancellationToken,
    transcript: Transcript,
    context_tokens: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            order: order(),
            cancel: CancellationToken::new(),
            transcript: Transcript::discard(),
            context_tokens: 32_768,
        }
    }
}

async fn run_loop(script: Script, handler: &mut impl FinishHandler, options: Options) -> LoopRun {
    let server = FakeModelServer::start(script).await;
    let repo = TempRepo::with_files(&[("src/calc.rs", CALC), ("README.md", "# calc\n")]);
    let config = OrkConfig {
        allow_commands: vec!["git status".into()],
        ..Default::default()
    };
    let ctx = ToolContext::new(
        &repo.path,
        &options.order,
        &config,
        &Env::from_process(),
        options.cancel,
    )
    .unwrap();
    let client = server.client();
    let sink = CollectSink::default();
    let settings = LoopSettings {
        max_turns: options.order.max_turns.unwrap_or(10),
        max_minutes: 5,
        temperature: 0.2,
        max_tokens: 512,
        context_tokens: options.context_tokens,
        token_budget: options.order.max_tokens,
    };
    let mut grunt = GruntLoop::new(
        &client,
        ToolRegistry::for_mode(options.order.mode),
        &sink,
        options.transcript,
        settings,
    );
    let outcome = grunt.run(&options.order, &ctx, handler).await;
    LoopRun {
        outcome,
        server,
        repo,
        events: sink.events(),
    }
}

fn finish(summary: &str) -> Value {
    json!({ "summary": summary })
}

#[tokio::test]
async fn read_then_write_then_finish() {
    let script = Script::new()
        .tool_call("read_file", json!({ "path": "src/calc.rs" }))
        .tool_call("write_file", json!({ "path": "src/sum.rs", "content": "pub fn sum(a: i32, b: i32) -> i32 { a + b }\n" }))
        .tool_call("finish", finish("Added sum in src/sum.rs"));

    let run = run_loop(script, &mut AcceptAll, Options::default()).await;

    assert_eq!(run.outcome.end, LoopEnd::Finished, "{:?}", run.outcome);
    assert_eq!(
        run.outcome.summary.as_deref(),
        Some("Added sum in src/sum.rs")
    );
    assert_eq!(run.outcome.turns, 3);
    assert_eq!((run.outcome.tokens_in, run.outcome.tokens_out), (300, 60));
    assert_eq!(
        run.repo.read("src/sum.rs"),
        "pub fn sum(a: i32, b: i32) -> i32 { a + b }\n"
    );

    // request 1: the system prompt, then the first message
    let first = run.messages(0);
    assert_eq!(
        first[0],
        json!({ "role": "system", "content": prompts::GRUNT_SYSTEM })
    );
    assert!(
        first[1]["content"]
            .as_str()
            .unwrap()
            .starts_with("## Goal\nAdd sum(a, b) to src/calc.rs\n")
    );
    // request 2: the assistant's call, then its result, tied by id
    let second = run.messages(1);
    assert_eq!(second.len(), 4);
    assert_eq!(second[2]["role"], "assistant");
    assert_eq!(second[2]["tool_calls"][0]["id"], "call_1");
    assert_eq!(second[3]["role"], "tool");
    assert_eq!(second[3]["tool_call_id"], "call_1");
    assert_eq!(
        second[3]["content"],
        "src/calc.rs (lines 1-3 of 3)\n    1 │ pub fn one() -> i32 {\n    2 │     1\n    3 │ }\n"
    );
    // and the tools were offered every time
    assert_eq!(
        run.server.requests()[2]["tools"].as_array().unwrap().len(),
        7
    );
}

#[tokio::test]
async fn the_events_tell_the_story() {
    let script = Script::new()
        .tool_call("read_file", json!({ "path": "src/calc.rs" }))
        .tool_call("finish", finish("done"));

    let run = run_loop(script, &mut AcceptAll, Options::default()).await;

    let kinds: Vec<&str> = run
        .events
        .iter()
        .map(|e| match e {
            OrkEvent::Call { .. } => "call",
            OrkEvent::Tool { .. } => "tool",
            OrkEvent::Finish { .. } => "finish",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["call", "tool", "call", "finish"]);
    let OrkEvent::Tool {
        turn,
        name,
        args,
        ok,
        ..
    } = &run.events[1]
    else {
        panic!()
    };
    assert_eq!((*turn, name.as_str(), *ok), (1, "read_file", true));
    assert_eq!(args, &Some(json!({ "path": "src/calc.rs" })));
}

#[tokio::test]
async fn a_tool_error_goes_back_as_text() {
    let script = Script::new()
        .tool_call("read_file", json!({ "path": "nope.rs" }))
        .tool_call("finish", finish("gave it a go"));

    let run = run_loop(script, &mut AcceptAll, Options::default()).await;

    assert_eq!(run.outcome.end, LoopEnd::Finished);
    let result = &run.messages(1)[3];
    assert_eq!(result["tool_call_id"], "call_1");
    assert!(
        result["content"]
            .as_str()
            .unwrap()
            .starts_with("not found: nope.rs"),
        "{result}"
    );
    let OrkEvent::Tool { ok, error, .. } = &run.events[1] else {
        panic!("{:?}", run.events[1])
    };
    assert!(!ok);
    assert!(error.as_deref().unwrap().starts_with("not found: nope.rs"));
}

#[tokio::test]
async fn two_calls_in_one_reply_each_get_a_tool_message() {
    let script = Script::new()
        .tool_calls(vec![
            ("read_file", json!({ "path": "src/calc.rs" })),
            ("read_file", json!({ "path": "README.md" })),
        ])
        .tool_call("finish", finish("read both"));

    let run = run_loop(script, &mut AcceptAll, Options::default()).await;

    let second = run.messages(1);
    let roles: Vec<&str> = second.iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, ["system", "user", "assistant", "tool", "tool"]);
    assert_eq!(second[3]["tool_call_id"], "call_1");
    assert_eq!(second[4]["tool_call_id"], "call_2");
    assert!(
        second[4]["content"]
            .as_str()
            .unwrap()
            .starts_with("README.md (lines 1-1 of 1)")
    );
}

#[tokio::test]
async fn max_turns_gives_up() {
    let script = Script::new()
        .tool_call("read_file", json!({ "path": "src/calc.rs" }))
        .tool_call("read_file", json!({ "path": "README.md" }))
        .tool_call("finish", finish("never reached"));
    let options = Options {
        order: Order {
            max_turns: Some(2),
            ..order()
        },
        ..Default::default()
    };

    let run = run_loop(script, &mut AcceptAll, options).await;

    assert_eq!(run.outcome.end, LoopEnd::GaveUp);
    assert_eq!(run.outcome.reason.as_deref(), Some("max_turns"));
    assert_eq!(run.outcome.turns, 2);
    assert_eq!(run.server.requests().len(), 2);
}

/// Rejects the first finish (as failing checks would), accepts the next.
#[derive(Default)]
struct RejectOnce {
    calls: Vec<FinishCall>,
}

impl FinishHandler for RejectOnce {
    async fn on_finish(&mut self, call: FinishCall) -> FinishVerdict {
        self.calls.push(call);
        if self.calls.len() == 1 {
            FinishVerdict::Reject("Checks failed. git grep -q sum exited 1:\n\nFix the problem, then call finish again.".into())
        } else {
            FinishVerdict::Accept
        }
    }
}

#[tokio::test]
async fn a_rejected_finish_reaches_the_model_which_can_finish_again() {
    let script = Script::new()
        .tool_call("finish", json!({ "summary": "done, I think", "open_questions": [] }))
        .tool_call("edit_file", json!({ "path": "src/calc.rs", "old": "    1\n}", "new": "    1\n}\n\npub fn sum() {}" }))
        .tool_call("finish", json!({ "summary": "Added sum", "open_questions": ["Should sum be generic?"] }));
    let mut handler = RejectOnce::default();

    let run = run_loop(script, &mut handler, Options::default()).await;

    assert_eq!(run.outcome.end, LoopEnd::Finished);
    assert_eq!(run.outcome.summary.as_deref(), Some("Added sum"));
    assert_eq!(run.outcome.open_questions, ["Should sum be generic?"]);
    assert_eq!(run.outcome.turns, 3);
    assert_eq!(handler.calls.len(), 2);
    assert_eq!(
        (handler.calls[0].turn, handler.calls[0].summary.as_str()),
        (1, "done, I think")
    );
    // the rejection is the first finish's result
    let second = run.messages(1);
    assert_eq!(second[3]["tool_call_id"], "call_1");
    assert!(
        second[3]["content"]
            .as_str()
            .unwrap()
            .starts_with("Checks failed.")
    );
}

#[tokio::test]
async fn a_text_reply_is_nudged_to_use_a_tool() {
    let script = Script::new()
        .text("I will now read the file.")
        .tool_call("finish", finish("done"));

    let run = run_loop(script, &mut AcceptAll, Options::default()).await;

    assert_eq!(run.outcome.end, LoopEnd::Finished);
    let second = run.messages(1);
    assert_eq!(
        second[2],
        json!({ "role": "assistant", "content": "I will now read the file." })
    );
    assert_eq!(
        second[3],
        json!({ "role": "user", "content": "Use a tool to make progress, or call finish." })
    );
}

#[tokio::test]
async fn a_bad_finish_call_comes_back_with_the_schema() {
    let script = Script::new()
        .tool_call("finish", json!({ "note": "done" }))
        .tool_call("finish", finish("done"));

    let run = run_loop(script, &mut AcceptAll, Options::default()).await;

    assert_eq!(run.outcome.end, LoopEnd::Finished);
    assert_eq!(run.outcome.turns, 2);
    let result = run.messages(1)[3]["content"].as_str().unwrap().to_string();
    assert!(result.contains("`summary` is required"), "{result}");
}

#[tokio::test]
async fn ctrl_c_ends_the_loop_as_an_error() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let options = Options {
        cancel,
        ..Default::default()
    };

    let run = run_loop(Script::new().text("never asked"), &mut AcceptAll, options).await;

    assert_eq!(run.outcome.end, LoopEnd::Error);
    assert_eq!(run.outcome.reason.as_deref(), Some("cancelled"));
    assert!(run.server.requests().is_empty());
}

#[tokio::test]
async fn a_model_error_ends_the_loop_as_an_error() {
    let run = run_loop(
        Script::new().status(400),
        &mut AcceptAll,
        Options::default(),
    )
    .await;

    assert_eq!(run.outcome.end, LoopEnd::Error);
    assert!(run.outcome.reason.as_deref().unwrap().contains("400"));
}

#[tokio::test]
async fn the_transcript_keeps_every_message_and_a_meta_line_per_reply() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("transcript.jsonl");
    let script = Script::new()
        .text("<think>hmm</think>Reading.")
        .tool_call("finish", finish("done"));
    let options = Options {
        transcript: Transcript::new(&path),
        ..Default::default()
    };

    run_loop(script, &mut AcceptAll, options).await;

    let lines: Vec<Value> = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let roles: Vec<&str> = lines.iter().map(|l| l["role"].as_str().unwrap()).collect();
    assert_eq!(
        roles,
        [
            "system",
            "user",
            "meta",
            "assistant",
            "user",
            "meta",
            "assistant",
            "tool"
        ]
    );
    assert_eq!(lines[2]["warnings"], json!(["think_stripped"]));
    assert!(
        lines[2]["raw"]["choices"][0]["message"]["content"]
            .as_str()
            .unwrap()
            .contains("<think>")
    );
    assert_eq!(lines[3]["content"], "Reading.");
    assert_eq!(lines[7]["tool_call_id"], "call_1");
}

#[test]
fn the_first_message_has_every_section() {
    let order = Order {
        goal: "Add sum".into(),
        context: Some("Keep it simple.".into()),
        paths: vec!["src/calc.rs".into(), "src/gone.rs".into()],
        write_scope: vec!["src/**".into(), "tests/**".into()],
        checks: vec!["cargo test".into()],
        ..Default::default()
    };
    let seed = Seed {
        files: vec![
            "src/calc.rs".into(),
            "src/lib.rs".into(),
            "Cargo.toml".into(),
        ],
        contents: vec![
            ("src/calc.rs".into(), Some("fn one() {}\n".into())),
            ("src/gone.rs".into(), None),
        ],
    };

    let message = first_message::render(&order, &seed);

    assert_eq!(
        message,
        "## Goal\nAdd sum\n\n## Context\nKeep it simple.\n\n## Files you may change\n- src/**\n- tests/**\n\n\
         ## Checks\norkestr8R runs these when you call finish:\n- `cargo test`\n\n\
         ## Repository\n```\nCargo.toml\nsrc/\n  calc.rs\n  lib.rs\n```\n\n\
         ## Files\n\n### src/calc.rs\n```\n    1 │ fn one() {}\n```\n\n### src/gone.rs\n(not found)\n"
    );
}

#[test]
fn a_readonly_first_message_says_so() {
    let order = Order {
        goal: "Where is sum used?".into(),
        mode: OrderMode::Readonly,
        ..Default::default()
    };

    let message = first_message::render(&order, &Seed::default());

    assert!(
        message.contains("## Files you may change\nNone: this is a read-only order."),
        "{message}"
    );
    assert!(message.contains("## Checks\nNone.\n"));
}

/// Each fake reply reports 100 tokens in and 20 out: 120 a turn.
#[tokio::test]
async fn the_token_budget_nudges_at_80_percent_then_stops() {
    let script = Script::new()
        .tool_call("read_file", json!({ "path": "src/calc.rs" }))
        .tool_call("read_file", json!({ "path": "README.md" }))
        .tool_call("read_file", json!({ "path": "src/calc.rs" }))
        .tool_call("finish", finish("never reached"));
    let options = Options {
        order: Order {
            max_tokens: Some(300),
            ..order()
        },
        ..Default::default()
    };

    let run = run_loop(script, &mut AcceptAll, options).await;

    // 120, 240 (80%: the nudge), 360 (over: stop before the third call runs)
    assert_eq!(run.outcome.end, LoopEnd::GaveUp);
    assert_eq!(run.outcome.reason.as_deref(), Some("max_tokens"));
    assert_eq!(run.server.requests().len(), 3);
    let nudges = |request: usize| {
        run.messages(request)
            .iter()
            .filter(|m| m["role"] == "user" && m["content"] == grunt::NUDGE_BUDGET)
            .count()
    };
    assert_eq!(
        (nudges(1), nudges(2)),
        (0, 1),
        "once, after the turn that crossed 80%"
    );
    let warnings = run
        .events
        .iter()
        .filter(|e| matches!(e, OrkEvent::Warning { code, .. } if code == "token_budget"))
        .count();
    assert_eq!(warnings, 1);
    let tools_run = run
        .events
        .iter()
        .filter(|e| matches!(e, OrkEvent::Tool { .. }))
        .count();
    assert_eq!(tools_run, 2, "the third reply's call didn't run");
}

#[test]
fn the_orders_budget_wins_over_the_configs() {
    let mut config = OrkConfig::default();
    config.defaults.max_tokens = Some(1000);
    let role = orkestr8r::config::RoleConfig::default();
    let budget =
        |order: &Order, config: &OrkConfig| LoopSettings::new(order, config, &role).token_budget;

    assert_eq!(
        budget(
            &Order {
                max_tokens: Some(300),
                ..order()
            },
            &config
        ),
        Some(300)
    );
    assert_eq!(budget(&order(), &config), Some(1000));
    assert_eq!(
        budget(&order(), &OrkConfig::default()),
        None,
        "no budget, no limit"
    );
}
