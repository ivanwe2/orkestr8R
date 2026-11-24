//! `read_file(path, start?, end?)`: numbered lines, up to 400 at a time.

use serde_json::{Value, json};

use super::{ToolContext, ToolResult, arg_str, arg_usize};
use crate::text;

pub const DESCRIPTION: &str = "Read a text file, with line numbers. Up to 400 lines at a time; pass \
     start and end (1-based, inclusive) to read a part.";
const MAX_LINES: usize = 400;

pub fn parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Relative to the repository root" },
            "start": { "type": "integer", "description": "First line to read (1-based)" },
            "end": { "type": "integer", "description": "Last line to read (inclusive)" }
        },
        "required": ["path"]
    })
}

pub fn run(args: &Value, ctx: &ToolContext) -> ToolResult {
    let rel = arg_str(args, "path", "");
    let path = match ctx.guard.resolve(rel, false) {
        Ok(path) => path,
        Err(refused) => return ToolResult::error(refused),
    };
    if path.is_dir() {
        return ToolResult::error(format!("{rel} is a directory; use list_dir"));
    }
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => {
            return ToolResult::error(format!("not found: {rel}; list_dir shows what's there"));
        }
    };
    if bytes.iter().take(8192).any(|&b| b == 0) {
        return ToolResult::error(format!("{rel} looks binary; not shown"));
    }

    let content = String::from_utf8_lossy(&bytes);
    let total = content.lines().count();
    if total == 0 {
        return ToolResult::ok(format!("{rel} (empty)"));
    }
    let start = arg_usize(args, "start").unwrap_or(1).max(1);
    if start > total {
        return ToolResult::error(format!(
            "start {start} is past the end of {rel} ({total} lines)"
        ));
    }
    let wanted_end = arg_usize(args, "end").unwrap_or(total).clamp(start, total);
    let end = wanted_end.min(start + MAX_LINES - 1);

    let part: String = content
        .lines()
        .skip(start - 1)
        .take(end - start + 1)
        .collect::<Vec<_>>()
        .join("\n");
    let (numbered, _) = text::numbered(&part, start, MAX_LINES);
    let mut output = format!("{rel} (lines {start}-{end} of {total})\n{numbered}");
    if end < wanted_end {
        output += &format!(
            "… {} more lines; read_file with start={} to read on\n",
            total - end,
            end + 1
        );
    }
    ToolResult::ok(output)
}
