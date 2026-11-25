//! `grep(pattern, path?, glob?)`: ripgrep when it's installed, `git grep` otherwise. Both print
//! `path:line:text`; neither looks into ignored files.

use serde_json::{Value, json};

use super::{ToolContext, ToolResult, arg_str};
use crate::process::{ProcessSpec, find};

pub const DESCRIPTION: &str = "Search file contents for a regular expression. Returns path:line:text, \
     up to 100 lines. Narrow it with path (a directory or file) and glob (e.g. *.rs).";
const MAX_LINES: usize = 100;

/// `path:line:text` with `/` in the path on every OS: a leading `./` or `.\` is stripped and
/// backslashes become `/`, but only in the path part (everything before the first `:<digits>:`);
/// the matched text after it is left alone, backslashes and all.
pub fn normalize_line(line: &str) -> String {
    let line = line
        .strip_prefix("./")
        .or_else(|| line.strip_prefix(".\\"))
        .unwrap_or(line);
    let (path, rest) = match find_line_number(line) {
        Some(end) => line.split_at(end),
        None => (line, ""),
    };
    format!("{}{}", path.replace('\\', "/"), rest)
}

/// The index just past the first `:<digits>:` in a `path:line:text` line, or `None` if the line
/// has no line number.
fn find_line_number(line: &str) -> Option<usize> {
    for (i, b) in line.bytes().enumerate() {
        if b != b':' {
            continue;
        }
        let next = match line[i + 1..].find(':') {
            Some(next) => next,
            None => continue,
        };
        let num = &line[i + 1..i + 1 + next];
        if !num.is_empty() && num.bytes().all(|b| b.is_ascii_digit()) {
            return Some(i + 1 + next + 1);
        }
    }
    None
}

pub fn parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "pattern": { "type": "string", "description": "A regular expression" },
            "path": { "type": "string", "description": "Where to search, relative to the repository root (default .)" },
            "glob": { "type": "string", "description": "Only files matching this glob, e.g. *.rs" }
        },
        "required": ["pattern"]
    })
}

pub async fn run(args: &Value, ctx: &ToolContext) -> ToolResult {
    let pattern = arg_str(args, "pattern", "");
    let rel = arg_str(args, "path", ".");
    let glob = args["glob"].as_str().filter(|glob| !glob.is_empty());
    let path = match ctx.guard.resolve(rel, false) {
        Ok(path) => ctx.guard.display(&path),
        Err(refused) => return ToolResult::error(refused),
    };
    // The glob only narrows the search; it must not lead it anywhere the path guard wouldn't
    // (rg's -g overrides its ignore rules).
    if let Some(glob) = glob
        && (glob.starts_with(['/', '\\'])
            || glob.contains(':')
            || glob
                .split(['/', '\\'])
                .any(|part| part == ".." || part.eq_ignore_ascii_case(".git")))
    {
        return ToolResult::error(format!(
            "refused: glob {glob}: no `..`, `.git` or absolute paths; use path to choose where to search"
        ));
    }

    let argv = if find("rg", ctx.runner.env(), &ctx.root).is_some() {
        rg_argv(pattern, &path, glob)
    } else {
        git_grep_argv(pattern, &path, glob)
    };
    let spec = ProcessSpec {
        argv,
        cwd: ctx.root.clone(),
        timeout: ctx.tool_timeout(),
    };
    let result = match ctx.runner.run(spec, &ctx.cancel).await {
        Ok(result) => result,
        Err(err) => return ToolResult::error(format!("grep failed: {err:#}")),
    };

    // Both tools: 0 = found, 1 = no match, anything else = an error (a bad regex, say).
    match result.exit_code {
        0 => {
            let lines: Vec<String> = result
                .output
                .lines()
                .map(normalize_line)
                // belt and braces: nothing from inside .git, however it got matched
                .filter(|l| !l.starts_with(".git/") && !l.contains("/.git/"))
                .collect();
            if lines.is_empty() {
                return ToolResult::ok("no matches");
            }
            let mut out = lines
                .iter()
                .take(MAX_LINES)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            if lines.len() > MAX_LINES {
                out += &format!(
                    "\n… {} more lines; narrow it with path or glob",
                    lines.len() - MAX_LINES
                );
            }
            ToolResult::ok(out)
        }
        1 if result.output.trim().is_empty() => ToolResult::ok("no matches"),
        _ => ToolResult::error(format!("grep failed: {}", result.output.trim())),
    }
}

fn rg_argv(pattern: &str, path: &str, glob: Option<&str>) -> Vec<String> {
    let mut argv: Vec<String> = ["rg", "-n", "--no-heading", "--color", "never", "-m", "50"]
        .map(String::from)
        .to_vec();
    if let Some(glob) = glob {
        argv.extend(["-g".into(), glob.into()]);
    }
    // After the user's glob: the later glob wins in rg, so .git stays out whatever came before.
    argv.extend(["-g".into(), "!.git".into()]);
    argv.extend(["-e".into(), pattern.into(), "--".into(), path.into()]);
    argv
}

/// `git grep`'s pathspec does what `rg -g` does: a glob without `/` matches the file name at any
/// depth (`**/` in front), within `path`.
fn git_grep_argv(pattern: &str, path: &str, glob: Option<&str>) -> Vec<String> {
    let mut argv: Vec<String> = ["git", "grep", "-n", "-I", "--untracked", "-e"]
        .map(String::from)
        .to_vec();
    argv.extend([pattern.to_string(), "--".to_string()]);
    let pathspec = match glob {
        None => path.to_string(),
        Some(glob) => {
            let glob = if glob.contains('/') {
                glob.to_string()
            } else {
                format!("**/{glob}")
            };
            if path == "." {
                format!(":(glob){glob}")
            } else {
                format!(":(glob){path}/{glob}")
            }
        }
    };
    argv.push(pathspec);
    argv
}
