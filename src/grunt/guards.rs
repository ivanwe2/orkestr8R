//! Loop guards: what makes loops fail with local models, caught early.
//!
//! Each guard is a counter with a nudge before a stop. One slip (a text reply, a broken argument,
//! a repeated call) gets a message that says what to do instead, and often that's enough. Only a
//! streak ends the run, as `gave_up` with a reason the chief can read, instead of burning every
//! turn and minute the order allows.

use serde_json::Value;

/// After this many text replies in a row: give up (`no_tool_calls`).
pub const MAX_NO_TOOL_CALLS: u32 = 3;
/// After this many malformed calls in a row: give up (`malformed_tool_calls`).
pub const MAX_MALFORMED: u32 = 3;
/// After this many repeats in a row of the call before: give up (`repeating`).
pub const MAX_REPEATS: u32 = 5;

/// Prefixed to the result of a call that repeats the one before it.
pub const REPEAT_NOTE: &str =
    "You already did this with the same arguments; the result is the same. Try something else.";

#[derive(Debug, Default)]
pub struct LoopGuards {
    text_replies: u32,
    malformed: u32,
    repeats: u32,
    last_call: Option<(String, Value)>,
}

/// What a call did to the repeat guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    /// Not the same as the call before.
    New,
    /// The same as the call before: nudge.
    Again,
    /// The same, too many times in a row: give up.
    GiveUp,
}

impl LoopGuards {
    /// A reply with text but no tool calls. `Some(reason)` when it's time to give up.
    pub fn text_reply(&mut self) -> Option<&'static str> {
        self.text_replies += 1;
        (self.text_replies >= MAX_NO_TOOL_CALLS).then_some("no_tool_calls")
    }

    /// A reply with tool calls: the text streak is over.
    pub fn tool_reply(&mut self) {
        self.text_replies = 0;
    }

    /// A call to an unknown tool, or with arguments that don't fit. `Some(reason)` to give up.
    pub fn malformed_call(&mut self) -> Option<&'static str> {
        self.malformed += 1;
        (self.malformed >= MAX_MALFORMED).then_some("malformed_tool_calls")
    }

    /// A well-formed call (whatever its result): the malformed streak is over, and the call is
    /// compared with the one before it.
    pub fn good_call(&mut self, name: &str, args: &Value) -> Repeat {
        self.malformed = 0;
        // `args` was parsed into a `Value`, whose objects keep keys sorted: key order and
        // whitespace don't make two calls different.
        let call = (name.to_string(), args.clone());
        if self.last_call.as_ref() == Some(&call) {
            self.repeats += 1;
            if self.repeats >= MAX_REPEATS {
                Repeat::GiveUp
            } else {
                Repeat::Again
            }
        } else {
            self.repeats = 0;
            self.last_call = Some(call);
            Repeat::New
        }
    }
}
