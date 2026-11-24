//! `edit_file(path, old, new)`: replaces one exact, unique occurrence of `old`.
//!
//! Exact and unique keeps each edit small and checkable: the model must quote the code it means,
//! and a failure says precisely what to fix (re-read the region, or quote more lines). Shows the
//! changed lines afterwards, numbered, so the model can see what it did.

use serde_json::{Value, json};

use super::{ToolContext, ToolResult, arg_str};
use crate::text;

pub const DESCRIPTION: &str = "Change an existing file: replace one exact occurrence of old with new. \
     old must match the file exactly (whitespace and indentation too) and occur once; include \
     enough surrounding lines to make it unique. Read the file first.";
const CONTEXT_LINES: usize = 3;
const MAX_SHOWN: usize = 40;

pub fn parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Relative to the repository root" },
            "old": { "type": "string", "description": "The exact text to replace" },
            "new": { "type": "string", "description": "What to put in its place" }
        },
        "required": ["path", "old", "new"]
    })
}

pub fn run(args: &Value, ctx: &ToolContext) -> ToolResult {
    let rel = arg_str(args, "path", "");
    let path = match ctx.guard.resolve(rel, true) {
        Ok(path) => path,
        Err(refused) => return ToolResult::error(refused),
    };
    let content = match std::fs::read(&path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => {
                return ToolResult::error(format!("{rel} is not UTF-8 text; it can't be edited"));
            }
        },
        Err(_) => {
            return ToolResult::error(format!("not found: {rel}; use write_file for new files"));
        }
    };

    let (mut old, mut new) = (
        arg_str(args, "old", "").to_string(),
        arg_str(args, "new", "").to_string(),
    );
    if old.is_empty() {
        return ToolResult::error("old is empty; to add text, put a nearby line in old and in new");
    }
    // The model writes `\n`; a Windows file has `\r\n`. Match the file's line endings.
    if content.contains("\r\n") {
        old = to_crlf(&old);
        new = to_crlf(&new);
    }

    let found = content.matches(old.as_str()).count();
    match found {
        0 => {
            return ToolResult::error(format!(
                "old not found in {rel}; read_file the region again, whitespace must match"
            ));
        }
        1 => {}
        n => {
            return ToolResult::error(format!(
                "old found {n} times in {rel}; include more surrounding lines"
            ));
        }
    }

    let at = content.find(old.as_str()).unwrap_or(0);
    let edited = content.replacen(old.as_str(), &new, 1);
    if let Err(err) = std::fs::write(&path, &edited) {
        return ToolResult::error(format!("can't write {rel}: {err}"));
    }

    let first_line = content[..at].matches('\n').count() + 1;
    let (removed, added) = (old.lines().count(), new.lines().count());
    let from = first_line.saturating_sub(CONTEXT_LINES).max(1);
    let to = first_line + added.max(1) - 1 + CONTEXT_LINES;
    let shown: String = edited
        .lines()
        .skip(from - 1)
        .take((to - from + 1).min(MAX_SHOWN))
        .collect::<Vec<_>>()
        .join("\n");
    let (numbered, _) = text::numbered(&shown, from, MAX_SHOWN);
    ToolResult::ok(format!(
        "edited {rel} (-{removed} +{added} lines)\n{numbered}"
    ))
}

fn to_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}
