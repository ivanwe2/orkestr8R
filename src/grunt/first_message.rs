//! The first user message: the order, plus enough of the repository that the grunt doesn't spend
//! its first turns finding its way (the tree, and the files the chief pointed at).

use crate::orders::{Order, OrderMode};
use crate::text;
use crate::tools::{ToolContext, list_dir};

const TREE_ENTRIES: usize = 200;
const MAX_LINES_PER_FILE: usize = 400;
const MAX_FILE_CHARS: usize = 40_000;

/// What the first message shows of the repository.
#[derive(Debug, Clone, Default)]
pub struct Seed {
    /// Every file in the worktree (`git ls-files`, tracked and new), with `/`.
    pub files: Vec<String>,
    /// `(path, content)` for each of the order's `paths`; `None` when it can't be read.
    pub contents: Vec<(String, Option<String>)>,
}

/// Gathers the seed from the worktree. Paths go through the guard like any tool call.
pub async fn seed(order: &Order, ctx: &ToolContext) -> Seed {
    let files = ctx.git().ls_files(&ctx.root).await.unwrap_or_default();
    let contents = order
        .paths
        .iter()
        .map(|rel| {
            let content = ctx
                .guard
                .resolve(rel, false)
                .ok()
                .and_then(|path| std::fs::read(path).ok())
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
            (rel.clone(), content)
        })
        .collect();
    Seed { files, contents }
}

/// The message, in Markdown sections.
pub fn render(order: &Order, seed: &Seed) -> String {
    let mut out = format!("## Goal\n{}\n", order.goal.trim());

    if let Some(context) = order.context.as_deref().filter(|c| !c.trim().is_empty()) {
        out += &format!("\n## Context\n{}\n", context.trim());
    }

    out += "\n## Files you may change\n";
    if order.mode == OrderMode::Readonly {
        out += "None: this is a read-only order. Find the answer and put it in finish's summary.\n";
    } else if order.write_scope.is_empty() {
        out += "None.\n";
    } else {
        for pattern in &order.write_scope {
            out += &format!("- {pattern}\n");
        }
    }

    out += "\n## Checks\n";
    if order.checks.is_empty() {
        out += "None.\n";
    } else {
        out += "orkestr8R runs these when you call finish:\n";
        for check in &order.checks {
            out += &format!("- `{check}`\n");
        }
    }

    out += "\n## Repository\n";
    if seed.files.is_empty() {
        out += "(no files)\n";
    } else {
        out += &format!(
            "```\n{}\n```\n",
            list_dir::render_fit(&seed.files, TREE_ENTRIES)
        );
    }

    if !seed.contents.is_empty() {
        out += "\n## Files\n";
        let mut room = MAX_FILE_CHARS;
        for (path, content) in &seed.contents {
            let Some(content) = content else {
                out += &format!("\n### {path}\n(not found)\n");
                continue;
            };
            let (numbered, cut_lines) = text::numbered(content, 1, MAX_LINES_PER_FILE);
            let (block, cut_chars) = text::cut(&numbered, room);
            room -= block.chars().count();
            let newline = if block.ends_with('\n') || block.is_empty() {
                ""
            } else {
                "\n"
            };
            out += &format!("\n### {path}\n```\n{block}{newline}```\n");
            if cut_lines || cut_chars {
                out += "(truncated; use read_file)\n";
            }
        }
    }
    out
}
