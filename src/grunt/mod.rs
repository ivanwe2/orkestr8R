//! The grunt loop: the agent loop from docs/agent-loop.md, for real.
//!
//! ```text
//! messages = [system prompt, first message]
//! each turn:
//!     reply = model(messages, tools)          one HTTP request; the list is its whole memory
//!     append the assistant message            before the results it asked for
//!     for each tool call:
//!         finish → ask on_finish: accept → done; reject → its feedback is the result, go on
//!         other  → validate, run, append the result tied to the call's id
//! ```
//!
//! Everything slow (a model call, a tool) is raced against Ctrl+C and the minute deadline in a
//! `tokio::select!`; the loser is dropped, which cancels it (and kills a running command's tree).

pub mod context;
pub mod first_message;
pub mod guards;
pub mod prompts;
pub mod transcript;

use std::future::Future;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::time::Instant;

pub use context::ContextKeeper;
pub use guards::{LoopGuards, REPEAT_NOTE, Repeat};
pub use transcript::Transcript;

use crate::config::{OrkConfig, RoleConfig};
use crate::events::{EventSink, OrkEvent};
use crate::model::{ChatRequest, ModelClient, ModelReply, WireMessage};
use crate::orders::Order;
use crate::process::{HEAD_TAIL_MAX, head_tail};
use crate::tools::{Tool, ToolContext, ToolRegistry, args};

/// The user message after a reply with no tool call.
pub const NUDGE_NO_TOOLS: &str = "Use a tool to make progress, or call finish.";
/// The user message after a reply that hit `max_tokens`.
pub const NUDGE_CUT_OFF: &str =
    "Your reply was cut off. Keep messages short; one tool call at a time.";

/// The grunt called `finish`.
#[derive(Debug, Clone, PartialEq)]
pub struct FinishCall {
    pub turn: u32,
    pub summary: String,
    pub open_questions: Vec<String>,
}

/// What `on_finish` decides.
#[derive(Debug, Clone, PartialEq)]
pub enum FinishVerdict {
    /// Done: the loop ends.
    Accept,
    /// Not yet: the text goes back as the finish call's result, and the loop goes on. This is how
    /// a repair turn happens (T12): the checks failed, here's their output, fix it.
    Reject(String),
}

/// Decides whether a `finish` ends the job. Keeps the checks out of the loop: the loop only knows
/// "accepted" or "rejected, with this feedback".
///
/// Implementations write `async fn on_finish(&mut self, call: FinishCall) -> FinishVerdict`.
/// (The `+ Send` on the returned future lets the loop run on any tokio worker thread.)
pub trait FinishHandler {
    fn on_finish(&mut self, call: FinishCall) -> impl Future<Output = FinishVerdict> + Send;
}

/// Accepts every `finish`.
pub struct AcceptAll;

impl FinishHandler for AcceptAll {
    async fn on_finish(&mut self, _call: FinishCall) -> FinishVerdict {
        FinishVerdict::Accept
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopEnd {
    Finished,
    GaveUp,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoopOutcome {
    pub end: LoopEnd,
    pub summary: Option<String>,
    pub open_questions: Vec<String>,
    /// Why it gave up (`max_turns`, `max_minutes`, …) or the error.
    pub reason: Option<String>,
    pub turns: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoopSettings {
    pub max_turns: u32,
    pub max_minutes: u32,
    pub temperature: f64,
    pub max_tokens: u32,
    pub context_tokens: u32,
}

impl LoopSettings {
    /// The order's limits where it sets them, else the config's defaults; the role's model
    /// settings.
    pub fn new(order: &Order, config: &OrkConfig, role: &RoleConfig) -> Self {
        Self {
            max_turns: order.max_turns.unwrap_or(config.defaults.max_turns),
            max_minutes: order.max_minutes.unwrap_or(config.defaults.max_minutes),
            temperature: role.temperature,
            max_tokens: role.max_output_tokens,
            context_tokens: role.context_tokens,
        }
    }
}

pub struct GruntLoop<'a> {
    model: &'a ModelClient,
    tools: ToolRegistry,
    events: &'a dyn EventSink,
    transcript: Transcript,
    settings: LoopSettings,
}

impl<'a> GruntLoop<'a> {
    pub fn new(
        model: &'a ModelClient,
        tools: ToolRegistry,
        events: &'a dyn EventSink,
        transcript: Transcript,
        settings: LoopSettings,
    ) -> Self {
        Self {
            model,
            tools,
            events,
            transcript,
            settings,
        }
    }

    /// Runs the order to the end: finished, gave up, or an error. Never fails: every way it can
    /// end is a [`LoopOutcome`].
    pub async fn run(
        &mut self,
        order: &Order,
        ctx: &ToolContext,
        on_finish: &mut impl FinishHandler,
    ) -> LoopOutcome {
        let deadline =
            Instant::now() + Duration::from_secs(u64::from(self.settings.max_minutes) * 60);
        let seed = first_message::seed(order, ctx).await;
        let mut messages = Vec::new();
        self.push(&mut messages, 0, WireMessage::system(prompts::GRUNT_SYSTEM));
        self.push(
            &mut messages,
            0,
            WireMessage::user(first_message::render(order, &seed)),
        );
        let mut tally = Tally::default();
        let mut guards = LoopGuards::default();
        let keeper = ContextKeeper::new(self.settings.context_tokens);
        let mut last_prompt_tokens = 0;

        for turn in 1..=self.settings.max_turns {
            tally.turns = turn;
            // The last reply said how big the prompt was; make room before sending more.
            let stubbed = keeper.shrink(&mut messages, last_prompt_tokens);
            if stubbed > 0 {
                self.events.emit(&OrkEvent::Warning {
                    code: "context_stubbed".into(),
                    message: format!("replaced {stubbed} old tool outputs with stubs to stay in the context window"),
                });
            }
            let request = ChatRequest {
                messages: messages.clone(),
                tools: self.tools.schemas(),
                temperature: self.settings.temperature,
                max_tokens: self.settings.max_tokens,
            };
            // `biased`: check Ctrl+C first, then the deadline, then the work.
            let reply = tokio::select! {
                biased;
                _ = ctx.cancel.cancelled() => return tally.end(LoopEnd::Error, "cancelled"),
                _ = tokio::time::sleep_until(deadline) => return tally.end(LoopEnd::GaveUp, "max_minutes"),
                reply = self.model.chat(&request) => reply,
            };
            let reply = match reply {
                Ok(reply) => reply,
                Err(err) => return tally.end(LoopEnd::Error, format!("{err:#}")),
            };
            self.record_reply(turn, &reply, &mut tally);
            last_prompt_tokens = reply.tokens_in;
            let cut_off = reply.finish_reason.as_deref() == Some("length");
            if cut_off {
                self.events.emit(&OrkEvent::Warning {
                    code: "reply_cut".into(),
                    message: format!("the reply hit max_tokens ({})", self.settings.max_tokens),
                });
            }

            // The assistant message goes in before the results: each result answers a call in it.
            self.push(&mut messages, turn, reply.to_message());

            if reply.tool_calls.is_empty() {
                if cut_off {
                    // Not the model's fault in the same way: tell it why, don't count it.
                    self.push(&mut messages, turn, WireMessage::user(NUDGE_CUT_OFF));
                    continue;
                }
                if let Some(reason) = guards.text_reply() {
                    return tally.end(LoopEnd::GaveUp, reason);
                }
                self.push(&mut messages, turn, WireMessage::user(NUDGE_NO_TOOLS));
                continue;
            }
            guards.tool_reply();

            // Every call gets its own tool message, tied by id, before the next request.
            for call in &reply.tool_calls {
                if call.name == Tool::Finish.name() {
                    let finish =
                        match args::validate(&call.arguments, &Tool::Finish.schema().parameters) {
                            Ok(args) => finish_call(turn, &args),
                            Err(problem) => {
                                self.emit_tool(
                                    turn,
                                    &call.name,
                                    &call.arguments,
                                    false,
                                    0,
                                    &problem,
                                );
                                self.push(
                                    &mut messages,
                                    turn,
                                    WireMessage::tool(&call.id, problem),
                                );
                                if let Some(reason) = guards.malformed_call() {
                                    return tally.end(LoopEnd::GaveUp, reason);
                                }
                                continue;
                            }
                        };
                    self.events.emit(&OrkEvent::Finish {
                        turn,
                        summary: finish.summary.clone(),
                    });
                    let verdict = tokio::select! {
                        biased;
                        _ = ctx.cancel.cancelled() => return tally.end(LoopEnd::Error, "cancelled"),
                        verdict = on_finish.on_finish(finish.clone()) => verdict,
                    };
                    match verdict {
                        FinishVerdict::Accept => {
                            self.push(&mut messages, turn, WireMessage::tool(&call.id, "accepted"));
                            return tally.finished(finish);
                        }
                        FinishVerdict::Reject(feedback) => {
                            self.push(&mut messages, turn, WireMessage::tool(&call.id, feedback));
                        }
                    }
                    continue;
                }

                // Malformed (unknown tool, arguments that don't fit): the error, with the schema,
                // is the result. A streak of them ends the run.
                let (tool, args) = match self.tools.prepare(&call.name, &call.arguments) {
                    Ok(prepared) => prepared,
                    Err(problem) => {
                        self.emit_tool(turn, &call.name, &call.arguments, false, 0, &problem);
                        self.push(&mut messages, turn, WireMessage::tool(&call.id, problem));
                        if let Some(reason) = guards.malformed_call() {
                            return tally.end(LoopEnd::GaveUp, reason);
                        }
                        continue;
                    }
                };
                let repeat = guards.good_call(&call.name, &args);

                let started = std::time::Instant::now();
                let result = tokio::select! {
                    biased;
                    _ = ctx.cancel.cancelled() => return tally.end(LoopEnd::Error, "cancelled"),
                    _ = tokio::time::sleep_until(deadline) => return tally.end(LoopEnd::GaveUp, "max_minutes"),
                    result = tool.run(&args, ctx) => result,
                };
                let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                self.emit_tool(
                    turn,
                    &call.name,
                    &call.arguments,
                    result.ok,
                    ms,
                    &result.output,
                );
                // Long outputs (a test run) would flood the context: keep the head and the tail.
                let mut content = head_tail(&result.output, HEAD_TAIL_MAX);
                if repeat != Repeat::New {
                    content = format!("{REPEAT_NOTE}\n{content}");
                }
                self.push(&mut messages, turn, WireMessage::tool(&call.id, content));
                if repeat == Repeat::GiveUp {
                    return tally.end(LoopEnd::GaveUp, "repeating");
                }
            }
            if cut_off {
                self.push(&mut messages, turn, WireMessage::user(NUDGE_CUT_OFF));
            }
        }
        tally.end(LoopEnd::GaveUp, "max_turns")
    }

    /// Appends a message to the conversation and the transcript.
    fn push(&self, messages: &mut Vec<WireMessage>, turn: u32, message: WireMessage) {
        self.transcript.message(turn, &message);
        messages.push(message);
    }

    fn record_reply(&self, turn: u32, reply: &ModelReply, tally: &mut Tally) {
        tally.tokens_in += reply.tokens_in;
        tally.tokens_out += reply.tokens_out;
        self.events.emit(&OrkEvent::Call {
            turn,
            model: self.model.model().to_string(),
            server: self.model.server_name().to_string(),
            tokens_in: reply.tokens_in,
            tokens_out: reply.tokens_out,
            seconds: reply.seconds,
            tokens_per_second: reply.tokens_per_second,
            cost: reply.cost,
        });
        for code in &reply.warnings {
            self.events.emit(&OrkEvent::Warning {
                code: code.clone(),
                message: warning_message(code),
            });
        }
        self.transcript.meta(turn, reply);
    }

    fn emit_tool(&self, turn: u32, name: &str, arguments: &str, ok: bool, ms: u64, output: &str) {
        let error = (!ok).then(|| {
            output
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect()
        });
        self.events.emit(&OrkEvent::Tool {
            turn,
            name: name.to_string(),
            args: short_args(arguments),
            ok,
            ms,
            error,
        });
    }
}

fn finish_call(turn: u32, args: &Value) -> FinishCall {
    let summary = args["summary"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string();
    let open_questions = args["open_questions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect();
    FinishCall {
        turn,
        summary,
        open_questions,
    }
}

#[derive(Default)]
struct Tally {
    turns: u32,
    tokens_in: u64,
    tokens_out: u64,
}

impl Tally {
    fn end(&self, end: LoopEnd, reason: impl Into<String>) -> LoopOutcome {
        LoopOutcome {
            end,
            summary: None,
            open_questions: Vec::new(),
            reason: Some(reason.into()),
            turns: self.turns,
            tokens_in: self.tokens_in,
            tokens_out: self.tokens_out,
        }
    }

    fn finished(&self, call: FinishCall) -> LoopOutcome {
        LoopOutcome {
            end: LoopEnd::Finished,
            summary: Some(call.summary),
            open_questions: call.open_questions,
            reason: None,
            turns: self.turns,
            tokens_in: self.tokens_in,
            tokens_out: self.tokens_out,
        }
    }
}

fn warning_message(code: &str) -> String {
    match code {
        "think_stripped" => "removed a <think> block from the reply".into(),
        "tool_calls_in_content" => {
            "the reply wrote its tool calls into the text; recovered them".into()
        }
        other => other.into(),
    }
}

/// The arguments for a `tool` event: strings over 200 chars become `"<N chars>"`, so a written
/// file doesn't end up in the event stream.
pub fn short_args(arguments: &str) -> Option<Value> {
    let Ok(mut value) = serde_json::from_str::<Value>(arguments) else {
        let shown: String = arguments.chars().take(200).collect();
        return Some(json!(shown));
    };
    fn shorten(value: &mut Value) {
        match value {
            Value::String(text) if text.chars().count() > 200 => {
                *value = json!(format!("<{} chars>", text.chars().count()));
            }
            Value::Array(items) => items.iter_mut().for_each(shorten),
            Value::Object(fields) => fields.values_mut().for_each(shorten),
            _ => {}
        }
    }
    shorten(&mut value);
    Some(value)
}
