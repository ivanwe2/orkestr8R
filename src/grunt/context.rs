//! Keeping the conversation inside the model's context window.
//!
//! Every request resends everything, and tool outputs (files read, test runs) are most of it. When
//! the server says the prompt is past 75% of the window, the oldest tool outputs are replaced by a
//! one-line stub. The calls themselves stay, so the model still knows what it did, and the stub
//! says how to get the output back: call the tool again.

use std::collections::HashMap;

use crate::model::WireMessage;

/// Start stubbing when the prompt is past this share of the window...
const START_AT: f64 = 0.75;
/// ...and stop once the estimate is under this.
const STOP_AT: f64 = 0.60;
/// Never stub the newest tool outputs: the model is likely working with them.
const KEEP_NEWEST: usize = 4;
/// A rough characters-per-token figure for code and English.
const CHARS_PER_TOKEN: f64 = 3.5;

pub struct ContextKeeper {
    context_tokens: u32,
}

impl ContextKeeper {
    pub fn new(context_tokens: u32) -> Self {
        Self { context_tokens }
    }

    /// Stubs old tool outputs when `prompt_tokens` (from the last reply's usage) says the window
    /// is filling up. Returns how many it stubbed. Never touches the first user message (the
    /// order) or the newest tool outputs.
    pub fn shrink(&self, messages: &mut [WireMessage], prompt_tokens: u64) -> usize {
        let window = f64::from(self.context_tokens);
        if (prompt_tokens as f64) <= window * START_AT {
            return 0;
        }

        // Which tool each result came from, for the stub's text.
        let names: HashMap<String, String> = messages
            .iter()
            .flat_map(|m| m.tool_calls.iter().flatten())
            .map(|call| (call.id.clone(), call.function.name.clone()))
            .collect();

        let tool_indexes: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == "tool")
            .map(|(i, _)| i)
            .collect();
        let candidates = &tool_indexes[..tool_indexes.len().saturating_sub(KEEP_NEWEST)];

        let mut stubbed = 0;
        for &i in candidates {
            if estimate(messages) < window * STOP_AT {
                break;
            }
            let message = &mut messages[i];
            if message.content.as_deref().is_some_and(is_stub) {
                continue;
            }
            let tool = message
                .tool_call_id
                .as_ref()
                .and_then(|id| names.get(id))
                .map_or("tool", String::as_str);
            message.content = Some(stub(tool));
            stubbed += 1;
        }
        stubbed
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
