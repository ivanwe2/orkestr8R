//! Keeping the conversation inside the model's context window.
//!
//! Every request resends everything. Tool outputs (files read, test runs) are most of it, and so
//! are the grunt's own big calls: a `write_file` with a 400-line file lives in the call's arguments
//! *and* on disk, and the arguments go out again on every turn. When the server says the prompt is
//! past 75% of the window, the oldest of both are replaced by stubs:
//! - a tool output → a one-line note saying how to get it back (call the tool again);
//! - a long string argument of an old call → `[N chars removed from the history to save context]`.
//!
//! Nothing is removed, only rewritten: the calls stay, with their ids, each paired with its result
//! (servers reject a tool message whose call is gone), so the model still knows what it did.

use std::collections::HashMap;

use serde_json::Value;

use crate::model::WireMessage;

/// Start stubbing when the prompt is past this share of the window...
const START_AT: f64 = 0.75;
/// ...and stop once the estimate is under this.
const STOP_AT: f64 = 0.60;
/// Never stub the newest tool outputs, nor the newest calls: the model is likely working with them.
const KEEP_NEWEST: usize = 4;
/// A string argument longer than this is stubbed (paths and short edits stay).
const LONG_ARGUMENT: usize = 200;
/// A rough characters-per-token figure for code and English.
const CHARS_PER_TOKEN: f64 = 3.5;

/// What a stubbed argument says; `write_file` and `edit_file` refuse content that contains it,
/// because a model tends to copy what it sees in its own history.
pub const ARGUMENT_STUB_MARK: &str = "removed from the history to save context";

/// What one `shrink` did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Shrunk {
    /// Tool outputs replaced by a stub.
    pub outputs: usize,
    /// Long string arguments of old calls replaced by a stub.
    pub arguments: usize,
}

impl Shrunk {
    pub fn any(&self) -> bool {
        self.outputs + self.arguments > 0
    }
}

pub struct ContextKeeper {
    context_tokens: u32,
}

impl ContextKeeper {
    pub fn new(context_tokens: u32) -> Self {
        Self { context_tokens }
    }

    /// Stubs old tool outputs and old long arguments, oldest first, when `prompt_tokens` (from the
    /// last reply's usage) says the window is filling up, until the estimate is under 60%. Never
    /// touches the system message, the first user message (the order), the newest 4 tool outputs or
    /// the newest 4 messages with calls.
    pub fn shrink(&self, messages: &mut [WireMessage], prompt_tokens: u64) -> Shrunk {
        let window = f64::from(self.context_tokens);
        let mut done = Shrunk::default();
        if (prompt_tokens as f64) <= window * START_AT {
            return done;
        }

        // Which tool each result came from, for the stub's text.
        let names: HashMap<String, String> = messages
            .iter()
            .flat_map(|m| m.tool_calls.iter().flatten())
            .map(|call| (call.id.clone(), call.function.name.clone()))
            .collect();

        let outputs = newest_protected(messages, |m| m.role == "tool");
        let calls = newest_protected(messages, |m| {
            m.role == "assistant" && m.tool_calls.as_ref().is_some_and(|c| !c.is_empty())
        });

        for i in 0..messages.len() {
            if estimate(messages) < window * STOP_AT {
                break;
            }
            let message = &mut messages[i];
            if outputs.candidates.contains(&i) {
                if message.content.as_deref().is_some_and(is_stub) {
                    continue;
                }
                let tool = message
                    .tool_call_id
                    .as_ref()
                    .and_then(|id| names.get(id))
                    .map_or("tool", String::as_str);
                message.content = Some(stub(tool));
                done.outputs += 1;
            } else if calls.candidates.contains(&i) {
                for call in message.tool_calls.iter_mut().flatten() {
                    let (arguments, stubbed) = shrink_arguments(&call.function.arguments);
                    if stubbed > 0 {
                        call.function.arguments = arguments;
                        done.arguments += stubbed;
                    }
                }
            }
        }
        done
    }
}

/// The messages that match `pick`, minus the newest few: the ones that may be stubbed.
struct Protected {
    candidates: Vec<usize>,
}

fn newest_protected(messages: &[WireMessage], pick: impl Fn(&WireMessage) -> bool) -> Protected {
    let matching: Vec<usize> = messages
        .iter()
        .enumerate()
        // The system message and the order are never stubbed, whatever they are.
        .filter(|(i, m)| *i > 1 && pick(m))
        .map(|(i, _)| i)
        .collect();
    let keep = matching.len().saturating_sub(KEEP_NEWEST);
    Protected {
        candidates: matching[..keep].to_vec(),
    }
}

/// The call's arguments with every string over [`LONG_ARGUMENT`] characters stubbed, still valid
/// JSON, and how many were. Arguments that don't parse (the model wrote broken JSON) become
/// `{"removed": "N chars"}` when they're long.
fn shrink_arguments(arguments: &str) -> (String, usize) {
    match serde_json::from_str::<Value>(arguments) {
        Ok(mut value) => {
            let stubbed = stub_long_strings(&mut value);
            if stubbed == 0 {
                return (arguments.to_string(), 0);
            }
            (value.to_string(), stubbed)
        }
        Err(_) => {
            let chars = arguments.chars().count();
            if chars <= LONG_ARGUMENT {
                return (arguments.to_string(), 0);
            }
            let stub = serde_json::json!({ "removed": format!("{chars} chars") });
            (stub.to_string(), 1)
        }
    }
}

fn stub_long_strings(value: &mut Value) -> usize {
    match value {
        Value::String(text) => {
            let chars = text.chars().count();
            if chars > LONG_ARGUMENT {
                *text = format!("[{chars} chars {ARGUMENT_STUB_MARK}]");
                1
            } else {
                0
            }
        }
        Value::Array(items) => items.iter_mut().map(stub_long_strings).sum(),
        Value::Object(fields) => fields.values_mut().map(stub_long_strings).sum(),
        _ => 0,
    }
}

pub fn stub(tool: &str) -> String {
    format!("[{tool} output removed to save context; call it again if you need it]")
}

fn is_stub(content: &str) -> bool {
    content.starts_with('[') && content.ends_with("call it again if you need it]")
}

/// Roughly how many tokens the conversation takes: characters ÷ 3.5.
fn estimate(messages: &[WireMessage]) -> f64 {
    let chars: usize = messages
        .iter()
        .map(|m| {
            let content = m.content.as_deref().map_or(0, |c| c.chars().count());
            let calls: usize = m
                .tool_calls
                .iter()
                .flatten()
                .map(|c| c.function.name.len() + c.function.arguments.chars().count())
                .sum();
            content + calls
        })
        .sum();
    chars as f64 / CHARS_PER_TOKEN
}
