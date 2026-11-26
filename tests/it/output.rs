use anyhow::anyhow;
use orkestr8r::cli::{self, CliContext, CliIo, CommandResult, exit};
use orkestr8r::env::Env;
use orkestr8r::events::{EventLine, EventSink, FileSink, HumanSink, OrkEvent};
use orkestr8r::io::{Captured, SharedWriter};
use orkestr8r::orders::{CheckResult, FileChange, JobStatus, Report, TokenCount};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::support::cli::run_cli;

fn context(json: bool) -> (CliContext, Captured, Captured) {
    let (io, stdout, stderr) = CliIo::capture();
    let ctx = CliContext::new(
        io,
        std::env::temp_dir(),
        Env::default(),
        CancellationToken::new(),
        json,
    );
    (ctx, stdout, stderr)
}

fn report() -> Report {
    Report {
        job: "k7f2".into(),
        status: JobStatus::Passed,
        summary: "Added Sum".into(),
        files_changed: vec![FileChange {
            path: "src/calc.rs".into(),
            added: 4,
            removed: 1,
        }],
        checks: vec![CheckResult {
            cmd: "cargo test".into(),
            exit: 0,
            seconds: 1.5,
            tail: "ok".into(),
        }],
        repairs_used: 1,
        turns: 11,
        tokens: TokenCount {
            input: 61_234,
            output: 4_810,
        },
        model: "qwen-27b".into(),
        branch: "ork/k7f2".into(),
        open_questions: vec![],
        reason: None,
    }
}

fn start() -> OrkEvent {
    OrkEvent::Start {
        command: "run".into(),
        job: Some("k7f2".into()),
        branch: Some("ork/k7f2".into()),
        worktree: Some(".ork/worktrees/k7f2".into()),
        role: Some("grunt".into()),
        model: Some("qwen-27b".into()),
        server: Some("box".into()),
    }
}

fn lines(captured: &Captured) -> Vec<Value> {
    captured
        .text()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}")))
        .collect()
}

#[test]
fn every_event_round_trips_with_event_first() {
    let events = [
        ("start", start()),
        (
            "call",
            OrkEvent::Call {
                turn: 1,
                model: "qwen-27b".into(),
                server: "box".into(),
                tokens_in: 6120,
                tokens_out: 212,
                seconds: 4.1,
                tokens_per_second: Some(51.7),
                cost: Some(0.00042),
            },
        ),
        (
            "tool",
            OrkEvent::Tool {
                turn: 1,
                name: "read_file".into(),
                args: Some(json!({ "path": "src/calc.rs" })),
                ok: false,
                ms: 3,
                error: Some("not found".into()),
            },
        ),
        (
            "finish",
            OrkEvent::Finish {
                turn: 9,
                summary: "done".into(),
            },
        ),
        (
            "check",
            OrkEvent::Check {
                cmd: "cargo test".into(),
                exit: 101,
                seconds: 14.2,
                tail: "FAILED".into(),
            },
        ),
        ("repair", OrkEvent::Repair { attempt: 1 }),
        (
            "warning",
            OrkEvent::Warning {
                code: "think_stripped".into(),
                message: "…".into(),
            },
        ),
        (
            "result",
            OrkEvent::Result {
                status: "passed".into(),
                exit_code: 0,
                job: Some("k7f2".into()),
                report: Some(report()),
                data: Some(json!({ "answer": 42 })),
                error: None,
            },
        ),
    ];

    for (name, event) in events {
        let line = event.to_line();
        assert!(
            line.starts_with(&format!(r#"{{"event":"{name}""#)),
            "{line}"
        );
        let back: OrkEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(back, event);
    }
}

#[test]
fn every_line_has_its_time_second_and_reads_back() {
    let ts = "2026-11-26T16:02:11.123Z".parse().unwrap();

    let line = OrkEvent::Repair { attempt: 1 }.to_line_at(ts);
    assert_eq!(
        line,
        r#"{"event":"repair","ts":"2026-11-26T16:02:11.123Z","attempt":1}"#
    );
    let back = EventLine::parse(&line).unwrap();
    assert_eq!(back.ts, Some(ts));
    assert_eq!(back.event, OrkEvent::Repair { attempt: 1 });

    // An event whose other fields are all left out still gets its time.
    let report_only = OrkEvent::Result {
        status: "ok".into(),
        exit_code: 0,
        job: None,
        report: None,
        data: None,
        error: None,
    };
    let line = report_only.to_line_at(ts);
    assert!(
        line.starts_with(r#"{"event":"result","ts":"2026-11-26T16:02:11.123Z","status""#),
        "{line}"
    );
    assert_eq!(EventLine::parse(&line).unwrap().event, report_only);

    // Logs from before events had times still read; a torn last line doesn't.
    let old = EventLine::parse(r#"{"event":"repair","attempt":2}"#).unwrap();
    assert_eq!(old.ts, None);
    assert_eq!(EventLine::parse(r#"{"event":"rep"#), None);
}

#[test]
fn none_fields_are_left_out_and_names_are_snake_case() {
    let line = OrkEvent::Result {
        status: "passed".into(),
        exit_code: 0,
        job: None,
        report: Some(report()),
        data: None,
        error: None,
    }
    .to_line();

    assert!(!line.contains("null"), "{line}");
    assert!(line.contains(r#""exit_code":0"#), "{line}");
    assert!(line.contains(r#""status":"passed""#), "{line}");
    assert!(
        line.contains(r#""tokens":{"in":61234,"out":4810}"#),
        "{line}"
    );
}

#[tokio::test]
async fn result_is_last_when_the_handler_fails() {
    let (ctx, stdout, _) = context(true);

    let code = cli::complete(&ctx, async {
        ctx.emit(start());
        Err(anyhow!("server down").context("ork run"))
    })
    .await;

    assert_eq!(code, exit::ERROR);
    let lines = lines(&stdout);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["event"], "start");
    assert_eq!(lines[1]["event"], "result");
    assert_eq!(lines[1]["status"], "error");
    assert_eq!(lines[1]["exit_code"], 1);
    assert_eq!(lines[1]["error"], "ork run: server down");
}

async fn panics() -> anyhow::Result<CommandResult> {
    panic!("kaboom")
}

#[tokio::test]
async fn result_is_last_when_the_handler_panics() {
    let (ctx, stdout, _) = context(true);

    let code = cli::complete(&ctx, panics()).await;

    assert_eq!(code, exit::ERROR);
    let lines = lines(&stdout);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["event"], "result");
    assert_eq!(lines[0]["error"], "internal error: kaboom");
}

#[tokio::test]
async fn a_parse_error_with_json_gives_one_result_line() {
    let out = run_cli(&["--nope", "--json"], &std::env::temp_dir(), Env::default()).await;

    assert_eq!(out.code, exit::ERROR);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(lines.len(), 1, "{}", out.stdout);
    let result: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(result["event"], "result");
    assert_eq!(result["status"], "error");
    assert!(result["error"].as_str().unwrap().contains("--nope"));
    assert!(out.stderr.is_empty());
}

#[tokio::test]
async fn without_json_stdout_has_no_json() {
    let (ctx, stdout, stderr) = context(false);

    let code = cli::complete(&ctx, async {
        ctx.emit(start());
        ctx.say("hello");
        Ok(CommandResult::ok())
    })
    .await;

    assert_eq!(code, exit::OK);
    assert_eq!(stdout.text(), "hello\n");
    // the start line for people; nothing for an ok result
    assert_eq!(
        stderr.text(),
        "ork run k7f2 · grunt qwen-27b on box · .ork/worktrees/k7f2\n"
    );
}

#[tokio::test]
async fn without_json_an_error_goes_to_stderr() {
    let (ctx, stdout, stderr) = context(false);

    let code = cli::complete(&ctx, async { Err(anyhow!("no grunt role")) }).await;

    assert_eq!(code, exit::ERROR);
    assert_eq!(stdout.text(), "");
    assert_eq!(stderr.text(), "error: no grunt role\n");
}

#[test]
fn human_lines_put_the_call_on_its_first_tool_line() {
    let (out, captured) = SharedWriter::capture();
    let sink = HumanSink::new(out);

    sink.emit(&OrkEvent::Call {
        turn: 1,
        model: "qwen-27b".into(),
        server: "box".into(),
        tokens_in: 6120,
        tokens_out: 212,
        seconds: 4.1,
        tokens_per_second: Some(51.7),
        cost: None,
    });
    sink.emit(&OrkEvent::Tool {
        turn: 1,
        name: "read_file".into(),
        args: Some(json!({ "path": "src/calc.rs" })),
        ok: true,
        ms: 3,
        error: None,
    });
    sink.emit(&OrkEvent::Check {
        cmd: "cargo test".into(),
        exit: 101,
        seconds: 14.2,
        tail: String::new(),
    });
    sink.emit(&OrkEvent::Repair { attempt: 1 });
    sink.emit(&OrkEvent::Result {
        status: "passed".into(),
        exit_code: 0,
        job: Some("k7f2".into()),
        report: Some(report()),
        data: None,
        error: None,
    });

    assert_eq!(
        captured.text(),
        "  t1  read_file src/calc.rs   ok  3ms   6.1k→212 tok  52 t/s\n\
         \x20 ✗ cargo test  exit 101  14.2s\n\
         \x20 repair 1\n\
         passed · 11 turns · 1 repair · 61.2k in / 4.8k out\n"
    );
}

#[test]
fn the_file_sink_appends_one_line_per_event() {
    let dir = std::env::temp_dir().join(format!("ork-output-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("events.jsonl");
    let _ = std::fs::remove_file(&path);

    let sink = FileSink::new(&path);
    sink.emit(&start());
    sink.emit(&OrkEvent::Repair { attempt: 1 });

    let text = std::fs::read_to_string(&path).unwrap();
    let events: Vec<OrkEvent> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(events, vec![start(), OrkEvent::Repair { attempt: 1 }]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn human_lines_escape_control_characters_from_the_model() {
    let (out, captured) = SharedWriter::capture();
    let sink = HumanSink::new(out);

    sink.emit(&OrkEvent::Finish {
        turn: 2,
        summary: "done\x1b[2J\x1b]0;pwned\x07 \r ok".into(),
    });
    sink.emit(&OrkEvent::Result {
        status: "error".into(),
        exit_code: 1,
        job: None,
        report: None,
        data: None,
        error: Some("two problems:\n  roles.grunt: missing\x1b[31m".into()),
    });

    let text = captured.text();
    assert!(
        !text.contains('\x1b') && !text.contains('\x07') && !text.contains('\r'),
        "{text:?}"
    );
    assert!(text.contains(r"done\u{1b}[2J"), "{text:?}");
    // newlines in a multi-line error stay readable
    assert!(
        text.contains("error: two problems:\n  roles.grunt: missing\\u{1b}[31m\n"),
        "{text:?}"
    );
}
