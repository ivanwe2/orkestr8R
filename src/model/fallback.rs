//! Tool calls written into the text.
//!
//! When a server's chat template doesn't turn the model's tool calls into `tool_calls`, models
//! trained on the Hermes/Qwen format write them into the text instead:
//!
//! ```text
//! Let me look.
//! <tool_call>{"name": "read_file", "arguments": {"path": "src/calc.rs"}}</tool_call>
//! ```
//!
//! We recover those as real calls rather than losing the turn.

use serde_json::Value;

use super::wire::ToolCall;

const OPEN: &str = "<tool_call>";
const CLOSE: &str = "</tool_call>";

/// The calls found in `content` (ids `fallback_1`, `fallback_2`, …) and the text without them.
/// A block that isn't `{ "name": …, "arguments": … }` JSON is left in the text as it was.
pub fn parse(content: &str) -> (Vec<ToolCall>, String) {
    let mut calls = Vec::new();
    let mut text = String::new();
    let mut rest = content;

    while let Some(open) = rest.find(OPEN) {
        let after_open = &rest[open + OPEN.len()..];
        let Some(close) = after_open.find(CLOSE) else {
            break;
        };
        let inner = after_open[..close].trim();
        let block_end = open + OPEN.len() + close + CLOSE.len();

        match serde_json::from_str::<Value>(inner) {
            Ok(call) if call["name"].is_string() => {
                text.push_str(&rest[..open]);
                calls.push(ToolCall {
                    id: format!("fallback_{}", calls.len() + 1),
                    name: call["name"].as_str().unwrap_or_default().to_string(),
                    arguments: arguments_string(&call["arguments"]),
                });
            }
            _ => text.push_str(&rest[..block_end]),
        }
        rest = &rest[block_end..];
    }
    text.push_str(rest);

    (calls, text.trim().to_string())
}

/// `arguments` as the JSON string the wire format expects. Models write it as an object here
/// (sometimes as a string, sometimes not at all).
pub fn arguments_string(arguments: &Value) -> String {
    match arguments {
        Value::String(text) => text.clone(),
        Value::Null => "{}".to_string(),
        other => other.to_string(),
    }
}
