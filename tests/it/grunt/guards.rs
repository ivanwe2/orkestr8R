//! T11: the loop's guards and the context keeper.

use orkestr8r::events::OrkEvent;
use orkestr8r::grunt::{AcceptAll, LoopEnd, REPEAT_NOTE};
use serde_json::{Value, json};

use super::{Options, finish, run_loop};
use crate::support::fake_server::Script;

fn warning_codes(events: &[OrkEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            OrkEvent::Warning { code, .. } => Some(code.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn three_text_replies_in_a_row_give_up() {
    let script = Script::new()
        .text("Let me think.")
        .text("Still thinking.")
        .text("Almost there.")
        .text("never");

    let run = run_loop(script, &mut AcceptAll, Options::default()).await;

    assert_eq!(run.outcome.end, LoopEnd::GaveUp);
    assert_eq!(run.outcome.reason.as_deref(), Some("no_tool_calls"));
    assert_eq!(run.outcome.turns, 3);
    // the first two got a nudge
    let third = run.messages(2);
    let nudges = third
        .iter()
        .filter(|m| m["content"] == "Use a tool to make progress, or call finish.")
        .count();
    assert_eq!(nudges, 2);
}

#[tokio::test]
async fn a_tool_call_resets_the_text_counter() {
    let script = Script::new()
        .text("a")
        .text("b")
        .tool_call("read_file", json!({ "path": "README.md" }))
        .text("c")
        .text("d")
        .tool_call("finish", finish("done"));

    let run = run_loop(script, &mut AcceptAll, Options::default()).await;

    assert_eq!(run.outcome.end, LoopEnd::Finished, "{:?}", run.outcome);
    assert_eq!(run.outcome.turns, 6);
}

#[tokio::test]
async fn three_malformed_calls_in_a_row_give_up() {
    let script = Script::new()
        .malformed_args("read_file", r#"{"path": "src/calc.rs""#)
        .tool_call("read_file", json!({ "file": "src/calc.rs" }))
        .tool_call("delete_file", json!({ "path": "src/calc.rs" }))
        .tool_call("finish", finish("never"));

    let run = run_loop(script, &mut AcceptAll, Options::default()).await;

    assert_eq!(run.outcome.end, LoopEnd::GaveUp);
    assert_eq!(run.outcome.reason.as_deref(), Some("malformed_tool_calls"));
    assert_eq!(run.outcome.turns, 3);
    let third = run.messages(2);
    assert!(
        third[3]["content"]
            .as_str()
            .unwrap()
            .contains("not valid JSON")
    );
    assert!(
        third[5]["content"]
            .as_str()
            .unwrap()
            .contains("`path` is required")
    );
}

#[tokio::test]
async fn a_good_call_resets_the_malformed_counter() {
    let script = Script::new()
        .malformed_args("read_file", "{")
        .tool_call("nope", json!({}))
        .tool_call("read_file", json!({ "path": "missing.rs" }))
        .malformed_args("read_file", "{")
        .tool_call("nope", json!({}))
        .tool_call("finish", finish("done"));

    let run = run_loop(script, &mut AcceptAll, Options::default()).await;

    assert_eq!(
        run.outcome.end,
        LoopEnd::Finished,
        "a failed tool isn't a malformed call"
    );
}

#[tokio::test]
async fn the_same_call_again_is_nudged_then_stopped() {
    // the same arguments, written differently: key order and spacing don't matter
    let mut script =
        Script::new().tool_call("read_file", json!({ "path": "src/calc.rs", "start": 1 }));
    for _ in 0..5 {
        script = script.malformed_args("read_file", r#"{ "start": 1,  "path": "src/calc.rs" }"#);
    }
    let script = script.tool_call("finish", finish("never"));

    let run = run_loop(script, &mut AcceptAll, Options::default()).await;

    assert_eq!(run.outcome.end, LoopEnd::GaveUp);
    assert_eq!(run.outcome.reason.as_deref(), Some("repeating"));
    assert_eq!(run.outcome.turns, 6);
    let third = run.messages(2);
    let first_result = third[3]["content"].as_str().unwrap();
    let second_result = third[5]["content"].as_str().unwrap();
    assert!(
        first_result.starts_with("src/calc.rs (lines"),
        "{first_result}"
    );
    assert!(second_result.starts_with(REPEAT_NOTE), "{second_result}");
}

#[tokio::test]
async fn a_cut_off_reply_is_explained_and_the_loop_goes_on() {
    let cut = json!({
        "choices": [{
            "message": { "role": "assistant", "content": "I will start by writing a long plan" },
            "finish_reason": "length"
        }],
        "usage": { "prompt_tokens": 100, "completion_tokens": 512 }
    })
    .to_string();
    let script = Script::new()
        .raw(&cut)
        .raw(&cut)
        .raw(&cut)
        .tool_call("finish", finish("done"));

    let run = run_loop(script, &mut AcceptAll, Options::default()).await;

    assert_eq!(
        run.outcome.end,
        LoopEnd::Finished,
        "cut-off replies don't count as text replies"
    );
    assert_eq!(
        warning_codes(&run.events),
        ["reply_cut", "reply_cut", "reply_cut"]
    );
    let second = run.messages(1);
    assert_eq!(
        second[3]["content"],
        "Your reply was cut off. Keep messages short; one tool call at a time."
    );
}

#[tokio::test]
async fn old_tool_outputs_are_stubbed_when_the_context_fills() {
    let reads = [
        json!({ "path": "src/calc.rs" }),
        json!({ "path": "README.md" }),
        json!({ "path": "src/calc.rs", "start": 2 }),
        json!({ "path": "README.md", "end": 1 }),
        json!({ "path": "src/calc.rs", "end": 2 }),
    ];
    let mut script = Script::new();
    for args in reads {
        script = script.tool_call("read_file", args);
    }
    let script = script.tool_call("finish", finish("done"));
    // every fake reply says the prompt was 100 tokens: past 75% of 120
    let options = Options {
        context_tokens: 120,
        ..Default::default()
    };

    let run = run_loop(script, &mut AcceptAll, options).await;

    assert_eq!(run.outcome.end, LoopEnd::Finished);
    assert_eq!(warning_codes(&run.events), ["context_stubbed"]);
    let last = run.messages(5);
    assert_eq!(last[1]["role"], "user", "the order is never stubbed");
    assert!(last[1]["content"].as_str().unwrap().starts_with("## Goal"));
    assert_eq!(
        last[3]["content"],
        "[read_file output removed to save context; call it again if you need it]"
    );
    // the newest four outputs are kept
    let tool_messages: Vec<&Value> = last.iter().filter(|m| m["role"] == "tool").collect();
    assert_eq!(tool_messages.len(), 5);
    assert!(
        tool_messages[1..]
            .iter()
            .all(|m| m["content"].as_str().unwrap().contains(" │ "))
    );
}

mod context_keeper {
    use orkestr8r::grunt::ContextKeeper;
    use orkestr8r::model::{WireFunctionCall, WireMessage, WireToolCall};

    fn conversation(outputs: &[usize]) -> Vec<WireMessage> {
        let mut messages = vec![
            WireMessage::system("system"),
            WireMessage::user("x".repeat(100)),
        ];
        for (i, len) in outputs.iter().enumerate() {
            let id = format!("call_{i}");
            messages.push(WireMessage {
                role: "assistant".into(),
                content: None,
                tool_calls: Some(vec![WireToolCall {
                    id: id.clone(),
                    kind: "function".into(),
                    function: WireFunctionCall {
                        name: "grep".into(),
                        arguments: "{}".into(),
                    },
                }]),
                tool_call_id: None,
            });
            messages.push(WireMessage::tool(id, "o".repeat(*len)));
        }
        messages
    }

    fn contents(messages: &[WireMessage]) -> Vec<String> {
        messages
            .iter()
            .filter(|m| m.role == "tool")
            .map(|m| m.content.clone().unwrap())
            .collect()
    }

    #[test]
    fn nothing_changes_below_75_percent() {
        let mut messages = conversation(&[1000; 6]);
        let before = messages.clone();

        assert_eq!(ContextKeeper::new(1000).shrink(&mut messages, 750), 0);
        assert_eq!(messages, before);
    }

    #[test]
    fn the_oldest_go_first_and_the_newest_four_stay() {
        let mut messages = conversation(&[1000; 6]);

        let stubbed = ContextKeeper::new(1000).shrink(&mut messages, 900);

        assert_eq!(stubbed, 2, "only two are old enough");
        let tools = contents(&messages);
        assert_eq!(
            tools[0],
            "[grep output removed to save context; call it again if you need it]"
        );
        assert_eq!(tools[1], tools[0]);
        assert!(tools[2..].iter().all(|c| c.len() == 1000));
        assert_eq!(
            messages[1].content.as_deref(),
            Some("x".repeat(100).as_str())
        );
    }

    #[test]
    fn it_stops_once_the_estimate_is_under_60_percent() {
        // one huge old output and small ones: stubbing the huge one is enough
        let mut messages = conversation(&[10_000, 100, 100, 100, 100, 100]);

        let stubbed = ContextKeeper::new(4000).shrink(&mut messages, 3500);

        assert_eq!(stubbed, 1);
        assert_eq!(contents(&messages)[1].len(), 100);
    }
}
