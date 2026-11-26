//! The OpenAI-compatible chat wire format: what goes over HTTP, field for field. See
//! docs/agent-loop.md, "One turn on the wire".

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One message in the conversation. The list of these is the model's whole memory: every request
/// sends all of it again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireMessage {
    /// `system`, `user`, `assistant` or `tool`.
    pub role: String,
    /// Sent as `null` when `None`: an assistant message that only calls tools has no text.
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<WireToolCall>>,
    /// On a `tool` message: which call this is the result of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl WireMessage {
    fn plain(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::plain("system", content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::plain("user", content)
    }

    /// A tool's result, tied to the call that asked for it.
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            tool_call_id: Some(tool_call_id.into()),
            ..Self::plain("tool", content)
        }
    }
}

/// A tool call as the model sent it (and as we send it back in the history).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireToolCall {
    pub id: String,
    /// Always `"function"`.
    #[serde(rename = "type")]
    pub kind: String,
    pub function: WireFunctionCall,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireFunctionCall {
    pub name: String,
    /// A JSON object *as a string*: the model writes it as text, so it can be broken. The tool
    /// layer parses and validates it.
    pub arguments: String,
}

/// A tool the model may call: a name, a description for the model, and a JSON Schema for the
/// arguments. Sent as `{ "type": "function", "function": <this> }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// One request: the conversation so far and the tools on offer.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub messages: Vec<WireMessage>,
    /// Empty → no `tools` (and no `tool_choice`) in the request.
    pub tools: Vec<ToolSchema>,
    pub temperature: f64,
    pub max_tokens: u32,
}

/// A tool call from a reply, in our own shape.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// The raw JSON string, unvalidated.
    pub arguments: String,
}

/// What came back from one request, with nothing hidden.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelReply {
    /// The text, with think blocks and parsed `<tool_call>` blocks removed; `None` when empty.
    pub content: Option<String>,
    /// `reasoning_content` (or `reasoning`), when the server splits it out. Only for the
    /// transcript: it never goes back into the messages.
    pub reasoning: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    /// `stop` (done talking), `tool_calls` (wants tools run), `length` (cut off by max_tokens).
    pub finish_reason: Option<String>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub seconds: f64,
    pub tokens_per_second: Option<f64>,
    /// `usage.cost` in US dollars, when the server reports it (OpenRouter does; llama.cpp doesn't).
    pub cost: Option<f64>,
    /// Codes for what the client had to fix up: `think_stripped`, `tool_calls_in_content`.
    pub warnings: Vec<String>,
    /// The whole response body, for the transcript.
    pub raw_json: String,
}

impl ModelReply {
    /// The assistant message to append to the history: the cleaned text plus the tool calls
    /// exactly as received (fallback calls with their generated ids).
    pub fn to_message(&self) -> WireMessage {
        let tool_calls = (!self.tool_calls.is_empty()).then(|| {
            self.tool_calls
                .iter()
                .map(|call| WireToolCall {
                    id: call.id.clone(),
                    kind: "function".into(),
                    function: WireFunctionCall {
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    },
                })
                .collect()
        });
        WireMessage {
            role: "assistant".into(),
            content: self.content.clone(),
            tool_calls,
            tool_call_id: None,
        }
    }
}
