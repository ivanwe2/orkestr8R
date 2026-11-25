use orkestr8r::config::OrkConfig;
use orkestr8r::env::Env;
use orkestr8r::orders::{Order, OrderMode};
use orkestr8r::tools::{ToolContext, ToolRegistry, ToolResult, args};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::support::temp_repo::TempRepo;

struct Setup {
    repo: TempRepo,
    ctx: ToolContext,
    tools: ToolRegistry,
}

impl Setup {
    fn new(files: &[(&str, &str)], scope: &[&str]) -> Self {
        Self::with(files, scope, &["git status", "git grep"], OrderMode::Write)
    }

    fn with(files: &[(&str, &str)], scope: &[&str], allow: &[&str], mode: OrderMode) -> Self {
        let repo = TempRepo::with_files(files);
        let order = Order {
            goal: "test".into(),
            write_scope: scope.iter().map(|s| s.to_string()).collect(),
            mode,
            ..Default::default()
        };
        let config = OrkConfig {
            allow_commands: allow.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        let ctx = ToolContext::new(
            &repo.path,
            &order,
            &config,
            &Env::from_process(),
            CancellationToken::new(),
        )
        .unwrap();
        Self {
            repo,
            ctx,
            tools: ToolRegistry::for_mode(mode),
        }
    }

    async fn call(&self, name: &str, args: Value) -> ToolResult {
        self.tools.call(name, &args.to_string(), &self.ctx).await
    }
}

fn assert_refused(result: &ToolResult, needle: &str) {
    assert!(!result.ok, "expected a refusal, got: {}", result.output);
    assert!(
        result.output.contains(needle),
        "{:?} doesn't mention {needle:?}",
        result.output
    );
}

// read_file

#[tokio::test]
async fn read_file_numbers_the_lines_under_a_header() {
    let s = Setup::new(&[("src/calc.rs", "a\nb\nc\n")], &[]);

    let result = s.call("read_file", json!({ "path": "src/calc.rs" })).await;

    assert!(result.ok);
    assert_eq!(
        result.output,
        "src/calc.rs (lines 1-3 of 3)\n    1 │ a\n    2 │ b\n    3 │ c\n"
    );
}

#[tokio::test]
async fn read_file_reads_a_range() {
    let s = Setup::new(&[("a.txt", "1\n2\n3\n4\n5\n")], &[]);

    let result = s
        .call(
            "read_file",
            json!({ "path": "a.txt", "start": 2, "end": "3" }),
        )
        .await;

    assert_eq!(
        result.output,
        "a.txt (lines 2-3 of 5)\n    2 │ 2\n    3 │ 3\n"
    );
}

#[tokio::test]
async fn read_file_stops_at_400_lines_and_says_how_to_go_on() {
    let long: String = (1..=450).map(|i| format!("line {i}\n")).collect();
    let s = Setup::new(&[("long.txt", &long)], &[]);

    let result = s.call("read_file", json!({ "path": "long.txt" })).await;

    assert!(result.output.starts_with("long.txt (lines 1-400 of 450)\n"));
    assert!(result.output.contains("  400 │ line 400\n"));
    assert!(!result.output.contains("line 401"));
    assert!(
        result
            .output
            .ends_with("… 50 more lines; read_file with start=401 to read on\n")
    );
}

#[tokio::test]
async fn read_file_errors_are_results() {
    let s = Setup::new(&[("src/calc.rs", "x\n"), ("blob.bin", "\0\x01\x02")], &[]);

    assert_refused(
        &s.call("read_file", json!({ "path": "nope.rs" })).await,
        "not found: nope.rs",
    );
    assert_refused(
        &s.call("read_file", json!({ "path": "src" })).await,
        "is a directory; use list_dir",
    );
    assert_refused(
        &s.call("read_file", json!({ "path": "blob.bin" })).await,
        "looks binary",
    );
    assert_refused(
        &s.call("read_file", json!({ "path": "src/calc.rs", "start": 9 }))
            .await,
        "past the end",
    );
}

#[tokio::test]
async fn paths_outside_the_repository_are_refused() {
    let s = Setup::new(&[("a.txt", "a\n")], &["**"]);
    let outside = s.repo.dir.path().join("outside.txt");
    std::fs::write(&outside, "secret\n").unwrap();

    assert_refused(
        &s.call("read_file", json!({ "path": "../outside.txt" }))
            .await,
        "outside the repository",
    );
    assert_refused(
        &s.call("read_file", json!({ "path": "src/../../outside.txt" }))
            .await,
        "outside the repository",
    );
    let absolute = outside.display().to_string();
    assert_refused(
        &s.call("read_file", json!({ "path": absolute })).await,
        "outside the repository",
    );
    assert_refused(
        &s.call("read_file", json!({ "path": ".git/config" })).await,
        "inside .git",
    );
    assert_refused(
        &s.call(
            "write_file",
            json!({ "path": ".git/hooks/pre-commit", "content": "x" }),
        )
        .await,
        "inside .git",
    );
    // `..` that stays inside is fine
    assert!(
        s.call("read_file", json!({ "path": "src/../a.txt" }))
            .await
            .ok
    );
}

#[tokio::test]
async fn a_symlink_out_of_the_repository_is_refused() {
    let s = Setup::new(&[("a.txt", "a\n")], &["**"]);
    let outside = s.repo.dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "secret\n").unwrap();
    let link = s.repo.path.join("link");
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&outside, &link).is_ok();
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(&outside, &link).is_ok();
    if !made {
        eprintln!("skipped: can't create a symlink here (Windows without the privilege)");
        return;
    }

    assert_refused(
        &s.call("read_file", json!({ "path": "link/secret.txt" }))
            .await,
        "through a link",
    );
    assert_refused(
        &s.call(
            "write_file",
            json!({ "path": "link/new.txt", "content": "x" }),
        )
        .await,
        "through a link",
    );
    assert!(!outside.join("new.txt").exists());
}

// list_dir

#[tokio::test]
async fn list_dir_shows_a_tree_without_ignored_files() {
    let s = Setup::new(
        &[
            ("src/lib.rs", ""),
            ("src/tools/grep.rs", ""),
            ("src/tools/deep/x.rs", ""),
            ("README.md", ""),
            (".gitignore", "target/\n"),
        ],
        &[],
    );
    s.repo.write("notes.txt", "untracked\n");
    s.repo.write("target/debug/ork", "ignored");

    let result = s.call("list_dir", json!({})).await;

    assert!(result.ok);
    assert_eq!(
        result.output,
        ".gitignore\nREADME.md\nnotes.txt\nsrc/\n  lib.rs\n  tools/  (2 files)"
    );

    let deeper = s
        .call("list_dir", json!({ "path": "src", "depth": 3 }))
        .await;
    assert_eq!(
        deeper.output,
        "lib.rs\ntools/\n  deep/\n    x.rs\n  grep.rs"
    );

    let two_levels = s.call("list_dir", json!({ "path": "src" })).await;
    assert_eq!(
        two_levels.output,
        "lib.rs\ntools/\n  deep/  (1 files)\n  grep.rs"
    );
}

#[tokio::test]
async fn list_dir_of_a_file_is_an_error() {
    let s = Setup::new(&[("a.txt", "a\n")], &[]);

    assert_refused(
        &s.call("list_dir", json!({ "path": "a.txt" })).await,
        "not a directory",
    );
}

// grep

#[tokio::test]
async fn grep_finds_lines_as_path_line_text() {
    let s = Setup::new(
        &[
            ("src/calc.rs", "pub fn one() {}\npub fn sum() {}\n"),
            ("README.md", "sum things\n"),
        ],
        &[],
    );

    let result = s.call("grep", json!({ "pattern": "fn sum" })).await;

    assert!(result.ok, "{}", result.output);
    assert_eq!(result.output, "src/calc.rs:2:pub fn sum() {}");
}

#[tokio::test]
async fn grep_without_matches_is_ok() {
    let s = Setup::new(&[("a.txt", "a\n")], &[]);

    let result = s
        .call("grep", json!({ "pattern": "nothing_here_x9" }))
        .await;

    assert_eq!(result, ToolResult::ok("no matches"));
}

#[tokio::test]
async fn grep_narrows_by_glob_and_path() {
    let s = Setup::new(
        &[
            ("src/calc.rs", "sum\n"),
            ("src/deep/more.rs", "sum\n"),
            ("README.md", "sum\n"),
            ("tests/t.rs", "sum\n"),
        ],
        &[],
    );

    let by_glob = s
        .call("grep", json!({ "pattern": "sum", "glob": "*.rs" }))
        .await;
    let mut lines: Vec<&str> = by_glob.output.lines().collect();
    lines.sort();
    assert_eq!(
        lines,
        [
            "src/calc.rs:1:sum",
            "src/deep/more.rs:1:sum",
            "tests/t.rs:1:sum"
        ]
    );

    let by_path = s
        .call("grep", json!({ "pattern": "sum", "path": "tests" }))
        .await;
    assert_eq!(by_path.output, "tests/t.rs:1:sum");
}

// write_file

#[tokio::test]
async fn write_file_creates_a_new_file_and_its_directories() {
    let s = Setup::new(&[("a.txt", "a\n")], &["src/**"]);

    let result = s
        .call(
            "write_file",
            json!({ "path": "src/new/sum.rs", "content": "pub fn sum() {}\n// end\n" }),
        )
        .await;

    assert_eq!(result, ToolResult::ok("wrote 2 lines to src/new/sum.rs"));
    assert_eq!(s.repo.read("src/new/sum.rs"), "pub fn sum() {}\n// end\n");
}

#[tokio::test]
async fn write_file_never_overwrites() {
    let s = Setup::new(&[("src/calc.rs", "keep me\n")], &["src/**"]);

    let result = s
        .call(
            "write_file",
            json!({ "path": "src/calc.rs", "content": "gone" }),
        )
        .await;

    assert_refused(&result, "exists; use edit_file");
    assert_eq!(s.repo.read("src/calc.rs"), "keep me\n");
}

#[tokio::test]
async fn writes_outside_the_scope_are_refused() {
    let s = Setup::new(
        &[("src/calc.rs", "x\n"), ("Cargo.toml", "[package]\n")],
        &["src/*.rs"],
    );

    let result = s
        .call(
            "write_file",
            json!({ "path": "src/deep/x.rs", "content": "x" }),
        )
        .await;
    assert_refused(&result, "outside the files you may change (src/*.rs)");
    let result = s
        .call(
            "edit_file",
            json!({ "path": "Cargo.toml", "old": "[package]", "new": "[lib]" }),
        )
        .await;
    assert_refused(&result, "outside the files you may change");
    assert!(
        s.call(
            "write_file",
            json!({ "path": "src/new.rs", "content": "x" })
        )
        .await
        .ok
    );

    let none = Setup::new(&[("a.txt", "a\n")], &[]);
    assert_refused(
        &none
            .call("write_file", json!({ "path": "b.txt", "content": "x" }))
            .await,
        "may not change any files",
    );
}

// edit_file

#[tokio::test]
async fn edit_file_replaces_the_one_match_and_shows_the_lines() {
    let s = Setup::new(
        &[("src/calc.rs", "a\nb\nc\nfn one() {\n    1\n}\nd\ne\nf\ng\n")],
        &["src/**"],
    );

    let result = s
        .call("edit_file", json!({ "path": "src/calc.rs", "old": "fn one() {\n    1\n}", "new": "fn one() {\n    1 + 0\n}" }))
        .await;

    assert!(result.ok, "{}", result.output);
    assert_eq!(
        s.repo.read("src/calc.rs"),
        "a\nb\nc\nfn one() {\n    1 + 0\n}\nd\ne\nf\ng\n"
    );
    assert_eq!(
        result.output,
        "edited src/calc.rs (-3 +3 lines)\n    1 │ a\n    2 │ b\n    3 │ c\n    4 │ fn one() {\n    5 │     1 + 0\n    6 │ }\n    7 │ d\n    8 │ e\n    9 │ f\n"
    );
}

#[tokio::test]
async fn edit_file_needs_an_exact_unique_match() {
    let s = Setup::new(&[("src/calc.rs", "let x = 1;\nlet y = 1;\n")], &["src/**"]);

    let missing = s
        .call(
            "edit_file",
            json!({ "path": "src/calc.rs", "old": "let  x = 1;", "new": "z" }),
        )
        .await;
    assert_refused(
        &missing,
        "not found in src/calc.rs; read_file the region again, whitespace must match",
    );
    let twice = s
        .call(
            "edit_file",
            json!({ "path": "src/calc.rs", "old": "= 1;", "new": "= 2;" }),
        )
        .await;
    assert_refused(&twice, "found 2 times");
    let empty = s
        .call(
            "edit_file",
            json!({ "path": "src/calc.rs", "old": "", "new": "z" }),
        )
        .await;
    assert_refused(&empty, "old is empty");
    let new_file = s
        .call(
            "edit_file",
            json!({ "path": "src/new.rs", "old": "a", "new": "b" }),
        )
        .await;
    assert_refused(&new_file, "use write_file for new files");
    assert_eq!(
        s.repo.read("src/calc.rs"),
        "let x = 1;\nlet y = 1;\n",
        "unchanged"
    );
}

#[tokio::test]
async fn edit_file_keeps_crlf_line_endings() {
    let s = Setup::new(&[("win.txt", "a\r\nb\r\nc\r\n")], &["*.txt"]);

    let result = s
        .call(
            "edit_file",
            json!({ "path": "win.txt", "old": "a\nb", "new": "a\nB\nb2" }),
        )
        .await;

    assert!(result.ok, "{}", result.output);
    assert_eq!(s.repo.read("win.txt"), "a\r\nB\r\nb2\r\nc\r\n");
}

// run

#[tokio::test]
async fn run_runs_an_allowed_command() {
    let s = Setup::new(&[("a.txt", "a\n")], &[]);
    s.repo.write("b.txt", "new\n");

    let result = s.call("run", json!({ "cmd": "git status --short" })).await;

    assert!(result.ok, "{}", result.output);
    assert!(result.output.starts_with("exit 0 in "), "{}", result.output);
    assert!(result.output.contains("?? b.txt"), "{}", result.output);
}

#[tokio::test]
async fn a_failing_command_is_still_ok_with_its_exit_code() {
    let s = Setup::new(&[("a.txt", "a\n")], &[]);

    let result = s
        .call("run", json!({ "cmd": "git grep -q nothing_matches_x9" }))
        .await;

    assert!(result.ok, "the command ran; the model reads the exit code");
    assert!(result.output.starts_with("exit 1 in "), "{}", result.output);
}

#[tokio::test]
async fn run_refuses_what_isnt_allowed() {
    let s = Setup::new(&[("a.txt", "a\n")], &[]);

    assert_refused(
        &s.call("run", json!({ "cmd": "git push" })).await,
        "allowed: git status, git grep",
    );
    assert_refused(
        &s.call("run", json!({ "cmd": "git status && rm -rf ." }))
            .await,
        "metacharacter",
    );
}

// the registry and the arguments

#[tokio::test]
async fn readonly_orders_get_no_writing_tools() {
    let s = Setup::with(
        &[("a.txt", "a\n")],
        &["**"],
        &["git status"],
        OrderMode::Readonly,
    );

    let names: Vec<&str> = s.tools.tools().iter().map(|tool| tool.name()).collect();
    assert_eq!(names, ["read_file", "list_dir", "grep", "finish"]);
    let schema_names: Vec<String> = s
        .tools
        .schemas()
        .into_iter()
        .map(|schema| schema.name)
        .collect();
    assert_eq!(schema_names, names);
    assert_refused(
        &s.call("write_file", json!({ "path": "b.txt", "content": "x" }))
            .await,
        "unknown tool `write_file`",
    );

    let all = ToolRegistry::for_mode(OrderMode::Write);
    assert_eq!(all.tools().len(), 7);
}

#[tokio::test]
async fn bad_calls_come_back_with_the_expected_schema() {
    let s = Setup::new(&[("a.txt", "a\n")], &[]);

    let unknown = s.call("delete_file", json!({ "path": "a.txt" })).await;
    assert_refused(
        &unknown,
        "unknown tool `delete_file`; the tools are: read_file, list_dir, grep",
    );
    let broken = s
        .tools
        .call("read_file", r#"{"path": "a.txt""#, &s.ctx)
        .await;
    assert_refused(&broken, "not valid JSON");
    assert_refused(
        &broken,
        r#"Expected arguments: {"properties":{"end":{"type":"integer"}"#,
    );
    let missing = s.call("read_file", json!({ "file": "a.txt" })).await;
    assert_refused(&missing, "`path` is required");
}

#[test]
fn argument_validation_cases() {
    let schema = json!({
        "type": "object",
        "properties": {
            "path": { "type": "string" },
            "start": { "type": "integer" },
            "all": { "type": "boolean" },
            "tags": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["path"]
    });

    assert_eq!(
        args::validate(r#"{"path": "a"}"#, &schema).unwrap(),
        json!({ "path": "a" })
    );
    assert_eq!(
        args::validate(
            r#"{"path": "a", "start": "10", "all": "true", "extra": 1}"#,
            &schema
        )
        .unwrap(),
        json!({ "path": "a", "start": 10, "all": true, "extra": 1 }),
        "plain conversions, unknown keys kept and ignored"
    );
    assert_eq!(
        args::validate("", &json!({ "type": "object" })).unwrap(),
        json!({})
    );

    let err = |raw: &str| args::validate(raw, &schema).unwrap_err();
    assert!(err("[1, 2]").contains("must be a JSON object"));
    assert!(err("{}").contains("`path` is required"));
    assert!(err(r#"{"path": 3}"#).contains("`path` must be a string"));
    assert!(err(r#"{"path": "a", "start": "ten"}"#).contains("`start` must be an integer"));
    assert!(err(r#"{"path": "a", "start": 1.5}"#).contains("`start` must be an integer"));
    assert!(
        err(r#"{"path": "a", "tags": ["x", 2]}"#).contains("`tags` must be an array of strings")
    );
    assert!(err(r#"{"path": "a", "all": "yes"}"#).contains("`all` must be true or false"));
}

/// A directory symlink, or false where that isn't allowed (Windows without the privilege).
fn symlink_dir(target: &std::path::Path, link: &std::path::Path) -> bool {
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, link).is_ok();
    #[cfg(windows)]
    return std::os::windows::fs::symlink_dir(target, link).is_ok();
}

#[tokio::test]
async fn a_link_into_dot_git_is_refused_though_it_looks_fine() {
    let s = Setup::new(&[("src/lib.rs", "x\n")], &["src/**"]);
    if !symlink_dir(
        &s.repo.path.join(".git/hooks"),
        &s.repo.path.join("src/hooks"),
    ) {
        eprintln!("skipped: can't create a symlink here");
        return;
    }

    let write = s
        .call(
            "write_file",
            json!({ "path": "src/hooks/pre-commit", "content": "#!/bin/sh\n" }),
        )
        .await;
    assert_refused(&write, "leads into .git through a link");
    assert!(!s.repo.path.join(".git/hooks/pre-commit").exists());
    let read = s
        .call(
            "read_file",
            json!({ "path": "src/hooks/pre-commit.sample" }),
        )
        .await;
    assert_refused(&read, "leads into .git through a link");
}

#[tokio::test]
async fn the_scope_is_judged_on_where_a_link_really_leads() {
    let s = Setup::new(
        &[("src/lib.rs", "x\n"), ("docs/readme.md", "y\n")],
        &["docs/**"],
    );
    if !symlink_dir(&s.repo.path.join("src"), &s.repo.path.join("docs/code")) {
        eprintln!("skipped: can't create a symlink here");
        return;
    }

    // docs/code/new.rs looks in scope, but it's really src/new.rs
    let write = s
        .call(
            "write_file",
            json!({ "path": "docs/code/new.rs", "content": "x" }),
        )
        .await;
    assert_refused(
        &write,
        "src/new.rs is outside the files you may change (docs/**)",
    );
    // reading through a link that stays inside is fine
    assert!(
        s.call("read_file", json!({ "path": "docs/code/lib.rs" }))
            .await
            .ok
    );
}

#[tokio::test]
async fn grep_globs_cannot_lead_outside() {
    let s = Setup::new(&[("src/calc.rs", "sum\n")], &[]);

    for glob in ["../*", ".git/**", "src/../../x", "/etc/*"] {
        let result = s
            .call("grep", json!({ "pattern": "sum", "glob": glob }))
            .await;
        assert_refused(&result, "no `..`, `.git` or absolute paths");
    }
    assert!(
        s.call("grep", json!({ "pattern": "sum", "glob": "src/*.rs" }))
            .await
            .ok
    );
}

#[cfg(windows)]
#[tokio::test]
async fn windows_names_that_alias_other_files_are_refused() {
    let s = Setup::new(&[("a.txt", "a\n")], &["**"]);

    for path in ["a.txt:hidden", r".git.\config", r"src. \x.rs"] {
        let result = s
            .call("write_file", json!({ "path": path, "content": "x" }))
            .await;
        assert!(!result.ok, "{path}: {}", result.output);
    }
}

#[test]
fn normalize_line_rewrites_windows_paths_to_slashes() {
    assert_eq!(
        orkestr8r::tools::grep::normalize_line("src\\calc.rs:12:fn sum() {}"),
        "src/calc.rs:12:fn sum() {}"
    );
}

#[test]
fn normalize_line_leaves_backslashes_in_the_matched_text() {
    assert_eq!(
        orkestr8r::tools::grep::normalize_line(r#"src\calc.rs:12:let p = r"C:\temp\";"#),
        r#"src/calc.rs:12:let p = r"C:\temp\";"#
    );
}

#[test]
fn normalize_line_keeps_lines_that_already_use_slashes() {
    assert_eq!(
        orkestr8r::tools::grep::normalize_line("src/calc.rs:12:fn sum() {}"),
        "src/calc.rs:12:fn sum() {}"
    );
}

#[test]
fn normalize_line_strips_a_leading_dot_backslash() {
    assert_eq!(
        orkestr8r::tools::grep::normalize_line(r".\src\calc.rs:12:fn sum() {}"),
        "src/calc.rs:12:fn sum() {}"
    );
}
