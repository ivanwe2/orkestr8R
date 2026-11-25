//! `list_dir(path?, depth?)`: the files under a directory as a tree, from `git ls-files` (tracked
//! and new files; not ignored ones, so no `target/` or `node_modules/`).

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{ToolContext, ToolResult, arg_str, arg_usize};

pub const DESCRIPTION: &str = "List the files under a directory as a tree (tracked and new files, not \
     ignored ones). Directories end in /.";
const MAX_ENTRIES: usize = 300;
const MAX_DEPTH: usize = 4;

pub fn parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Directory, relative to the repository root (default .)" },
            "depth": { "type": "integer", "description": "How many levels to show (default 2, at most 4)" }
        }
    })
}

pub async fn run(args: &Value, ctx: &ToolContext) -> ToolResult {
    let rel = arg_str(args, "path", ".");
    let depth = arg_usize(args, "depth").unwrap_or(2).clamp(1, MAX_DEPTH);
    let dir = match ctx.guard.resolve(rel, false) {
        Ok(dir) => dir,
        Err(refused) => return ToolResult::error(refused),
    };
    if !dir.is_dir() {
        return ToolResult::error(format!("{rel} is not a directory"));
    }
    let files = match ctx.git().ls_files(&dir).await {
        Ok(files) => files,
        Err(err) => return ToolResult::error(format!("listing {rel} failed: {err:#}")),
    };
    if files.is_empty() {
        return ToolResult::ok(format!("{rel}: no files"));
    }
    ToolResult::ok(render(&files, depth, MAX_ENTRIES))
}

/// A tree of `files` (paths with `/`), `depth` levels deep, at most `max_entries` lines; deeper
/// directories show a file count.
pub fn render(files: &[String], depth: usize, max_entries: usize) -> String {
    #[derive(Default)]
    struct Node {
        children: BTreeMap<String, Node>,
        files: usize,
    }

    let mut root = Node::default();
    for file in files {
        let mut node = &mut root;
        for part in file.split('/') {
            node.files += 1;
            node = node.children.entry(part.to_string()).or_default();
        }
    }

    fn walk(node: &Node, level: usize, depth: usize, lines: &mut Vec<String>) {
        for (name, child) in &node.children {
            let indent = "  ".repeat(level);
            if child.children.is_empty() {
                lines.push(format!("{indent}{name}"));
            } else if level + 1 >= depth {
                lines.push(format!("{indent}{name}/  ({} files)", child.files));
            } else {
                lines.push(format!("{indent}{name}/"));
                walk(child, level + 1, depth, lines);
            }
        }
    }

    let mut lines = Vec::new();
    walk(&root, 0, depth, &mut lines);
    let more = lines.len().saturating_sub(max_entries);
    lines.truncate(max_entries);
    let mut out = lines.join("\n");
    if more > 0 {
        out += &format!("\n… {more} more");
    }
    out
}
