//! Removes `<think>…</think>` blocks from a reply.
//!
//! Reasoning models write their thinking before the answer. It can be thousands of tokens, and the
//! model never needs it again, so keeping it in the history (which every request resends) would
//! burn context on every turn. Only the answer goes back into the messages; the raw reply, thinking
//! included, stays in the transcript.

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

/// The text without think blocks, trimmed, and whether anything was removed.
///
/// - `<think>…</think>` blocks are removed, wherever they are.
/// - An orphan `</think>` (no `<think>` before it) removes everything before it: some chat
///   templates open the block inside the prompt, so the reply only has the closing tag.
/// - An unclosed `<think>` removes everything after it: the reply was cut off mid-thought.
pub fn strip(text: &str) -> (String, bool) {
    let mut rest = text;
    let mut stripped = false;

    if let Some(close) = rest.find(CLOSE)
        && rest.find(OPEN).is_none_or(|open| open > close)
    {
        rest = &rest[close + CLOSE.len()..];
        stripped = true;
    }

    let mut out = String::with_capacity(rest.len());
    while let Some(open) = rest.find(OPEN) {
        out.push_str(&rest[..open]);
        stripped = true;
        match rest[open..].find(CLOSE) {
            Some(close) => rest = &rest[open + close + CLOSE.len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);

    (out.trim().to_string(), stripped)
}
