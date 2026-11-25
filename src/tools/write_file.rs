//! `write_file(path, content)`: creates a new file. Never overwrites: existing files change through
//! `edit_file`, because whole-file rewrites are where local models drop lines and mangle code.

use serde_json::{Value, json};

use super::{ToolContext, ToolResult, arg_str};

pub const DESCRIPTION: &str = "Create a new file with this content (parent directories too). Fails if \
     the file exists: change existing files with edit_file.";

pub fn parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Relative to the repository root" },
            "content": { "type": "string", "description": "The whole file" }
        },
        "required": ["path", "content"]
    })
}

pub fn run(args: &Value, ctx: &ToolContext) -> ToolResult {
    let rel = arg_str(args, "path", "");
    let content = arg_str(args, "content", "");
    let path = match ctx.guard.resolve(rel, true) {
        Ok(path) => path,
        Err(refused) => return ToolResult::error(refused),
    };
    if path.exists() {
        return ToolResult::error(format!("{rel} exists; use edit_file to change it"));
    }
    if let Some(dir) = path.parent()
        && let Err(err) = std::fs::create_dir_all(dir)
    {
        return ToolResult::error(format!("can't create the directory for {rel}: {err}"));
    }
    match std::fs::write(&path, content) {
        Ok(()) => ToolResult::ok(format!("wrote {} lines to {rel}", content.lines().count())),
        Err(err) => ToolResult::error(format!("can't write {rel}: {err}")),
    }
}
