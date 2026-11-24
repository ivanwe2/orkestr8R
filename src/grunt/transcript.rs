//! The transcript: every message sent to or received from the model, one JSON line each, plus one
//! `meta` line per reply (what the server said about it, and the raw body). It's for debugging a
//! run afterwards (`ork logs`); the chief never needs it.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::model::{ModelReply, WireMessage};

pub struct Transcript {
    path: Option<PathBuf>,
}

impl Transcript {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(path.into()),
        }
    }

    /// A transcript that keeps nothing.
    pub fn discard() -> Self {
        Self { path: None }
    }

    /// `{ "turn", "role", "content", "tool_calls"?, "tool_call_id"? }`
    pub fn message(&self, turn: u32, message: &WireMessage) {
        let mut line = json!({ "turn": turn, "role": message.role, "content": message.content });
        if let Some(calls) = &message.tool_calls {
            line["tool_calls"] = json!(calls);
        }
        if let Some(id) = &message.tool_call_id {
            line["tool_call_id"] = json!(id);
        }
        self.append(&line);
    }

    /// `{ "turn", "role": "meta", "finish_reason", "tokens_in", "tokens_out", "seconds",
    /// "warnings", "reasoning_content"?, "raw" }`
    pub fn meta(&self, turn: u32, reply: &ModelReply) {
        let raw: Value =
            serde_json::from_str(&reply.raw_json).unwrap_or_else(|_| json!(reply.raw_json));
        let mut line = json!({
            "turn": turn,
            "role": "meta",
            "finish_reason": reply.finish_reason,
            "tokens_in": reply.tokens_in,
            "tokens_out": reply.tokens_out,
            "seconds": reply.seconds,
            "warnings": reply.warnings,
            "raw": raw,
        });
        if let Some(reasoning) = &reply.reasoning {
            line["reasoning_content"] = json!(reasoning);
        }
        self.append(&line);
    }

    fn append(&self, line: &Value) {
        let Some(path) = &self.path else {
            return;
        };
        // Open, append, close, like the event log. A transcript that can't be written must not
        // stop the job.
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{line}");
        }
    }
}
