//! Events: everything `ork` does is an event. With `--json` they go to stdout as JSON Lines (the
//! last line is always `result`); otherwise as short human lines to stderr. Jobs also keep them in
//! `events.jsonl`. Contract: docs/cli.md.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::io::SharedWriter;
use crate::orders::Report;
use crate::text;

/// One event. Internally tagged: serde writes `"event": "<variant>"` as the first key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum OrkEvent {
    /// The command started; for `run`: job, branch, worktree, role, model, server.
    Start {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        job: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worktree: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        server: Option<String>,
    },
    /// One model request.
    Call {
        turn: u32,
        model: String,
        server: String,
        tokens_in: u64,
        tokens_out: u64,
        seconds: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens_per_second: Option<f64>,
    },
    /// One tool call. `args` has strings over 200 chars replaced by `"<N chars>"`.
    Tool {
        turn: u32,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        args: Option<Value>,
        ok: bool,
        ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// The grunt called `finish`.
    Finish {
        turn: u32,
        summary: String,
    },
    /// One check command.
    Check {
        cmd: String,
        exit: i32,
        seconds: f64,
        tail: String,
    },
    /// A repair turn starts.
    Repair {
        attempt: u32,
    },
    Warning {
        code: String,
        message: String,
    },
    /// Always the last event of a command.
    Result {
        status: String,
        exit_code: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        job: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        report: Option<Report>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

impl OrkEvent {
    /// The event as one line of JSON.
    pub fn to_line(&self) -> String {
        // These types always serialize: string keys only, and serde_json writes non-finite floats
        // as null.
        serde_json::to_string(self).expect("an OrkEvent always serializes")
    }
}

/// Anything that receives events. `Send + Sync` so one sink can be shared across tasks.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: &OrkEvent);
}

/// One JSON line per event, flushed at once (a reader may be waiting on the pipe).
pub struct JsonLinesSink {
    out: SharedWriter,
}

impl JsonLinesSink {
    pub fn new(out: SharedWriter) -> Self {
        Self { out }
    }
}

impl EventSink for JsonLinesSink {
    fn emit(&self, event: &OrkEvent) {
        self.out.write_line(&event.to_line());
    }
}

/// Appends one JSON line per event to a file (a job's `events.jsonl`).
pub struct FileSink {
    path: PathBuf,
    lock: Mutex<()>,
}

impl FileSink {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }
}

impl EventSink for FileSink {
    fn emit(&self, event: &OrkEvent) {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        // Open, append, close: nothing stays open, and a crash loses at most this line. A log
        // that can't be written must not stop the job, so errors are ignored.
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(file, "{}", event.to_line());
        }
    }
}

/// Sends every event to each of its sinks, in order.
pub struct CompositeSink(pub Vec<Arc<dyn EventSink>>);

impl EventSink for CompositeSink {
    fn emit(&self, event: &OrkEvent) {
        for sink in &self.0 {
            sink.emit(event);
        }
    }
}

/// Short lines for people, on stderr:
///
/// ```text
/// ork run k7f2 · grunt qwen-27b on box · .ork/worktrees/k7f2
///   t1  read_file src/calc.rs   ok  3ms   6.1k→212 tok  52 t/s
///   ✗ cargo test  exit 101  14.2s
/// passed · 11 turns · 1 repair · 61.2k in / 4.8k out
/// ```
///
/// A `call` is held back and printed on the line of the first tool (or finish) of its turn.
pub struct HumanSink {
    out: SharedWriter,
    pending_call: Mutex<Option<CallStats>>,
}

struct CallStats {
    turn: u32,
    text: String,
}

impl HumanSink {
    pub fn new(out: SharedWriter) -> Self {
        Self {
            out,
            pending_call: Mutex::new(None),
        }
    }

    /// The held-back call stats, if they belong to `turn`; otherwise they're printed on their own.
    fn take_call(&self, pending: &mut Option<CallStats>, turn: Option<u32>) -> Option<String> {
        match pending.take() {
            Some(call) if Some(call.turn) == turn => Some(call.text),
            Some(call) => {
                self.out
                    .write_line(&format!("  t{}  reply   {}", call.turn, call.text));
                None
            }
            None => None,
        }
    }
}

impl EventSink for HumanSink {
    fn emit(&self, event: &OrkEvent) {
        let mut pending = self
            .pending_call
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let line = match event {
            OrkEvent::Start {
                command,
                job,
                worktree,
                role,
                model,
                server,
                ..
            } => {
                let mut line = format!("ork {command}");
                if let Some(job) = job {
                    line += &format!(" {job}");
                }
                if let (Some(role), Some(model)) = (role, model) {
                    line += &format!(" · {role} {model}");
                    if let Some(server) = server {
                        line += &format!(" on {server}");
                    }
                }
                if let Some(worktree) = worktree {
                    line += &format!(" · {worktree}");
                }
                Some(line)
            }
            OrkEvent::Call {
                turn,
                tokens_in,
                tokens_out,
                tokens_per_second,
                ..
            } => {
                self.take_call(&mut pending, None);
                let mut text = format!(
                    "{}→{} tok",
                    short_count(*tokens_in),
                    short_count(*tokens_out)
                );
                if let Some(tps) = tokens_per_second {
                    text += &format!("  {tps:.0} t/s");
                }
                *pending = Some(CallStats { turn: *turn, text });
                None
            }
            OrkEvent::Tool {
                turn,
                name,
                args,
                ok,
                ms,
                error,
            } => {
                let call = self.take_call(&mut pending, Some(*turn));
                let mut line = format!("  t{turn}  {name}");
                if let Some(arg) = args.as_ref().and_then(primary_arg) {
                    line += &format!(" {arg}");
                }
                if *ok {
                    line += &format!("   ok  {ms}ms");
                } else {
                    let error = error.as_deref().unwrap_or("failed");
                    line += &format!("   error: {}", first_line(error, 100));
                }
                if let Some(call) = call {
                    line += &format!("   {call}");
                }
                Some(line)
            }
            OrkEvent::Finish { turn, summary } => {
                let call = self.take_call(&mut pending, Some(*turn));
                let mut line = format!("  t{turn}  finish  {}", first_line(summary, 100));
                if let Some(call) = call {
                    line += &format!("   {call}");
                }
                Some(line)
            }
            OrkEvent::Check {
                cmd, exit, seconds, ..
            } => {
                self.take_call(&mut pending, None);
                Some(if *exit == 0 {
                    format!("  ✓ {cmd}  {seconds:.1}s")
                } else {
                    format!("  ✗ {cmd}  exit {exit}  {seconds:.1}s")
                })
            }
            OrkEvent::Repair { attempt } => Some(format!("  repair {attempt}")),
            OrkEvent::Warning { code, message } => Some(format!("  warning {code}: {message}")),
            OrkEvent::Result {
                status,
                error,
                report,
                ..
            } => {
                self.take_call(&mut pending, None);
                if let Some(error) = error {
                    Some(format!("error: {error}"))
                } else if let Some(report) = report {
                    let repairs = match report.repairs_used {
                        1 => "1 repair".to_string(),
                        n => format!("{n} repairs"),
                    };
                    Some(format!(
                        "{status} · {} turns · {repairs} · {} in / {} out",
                        report.turns,
                        short_count(report.tokens.input),
                        short_count(report.tokens.output)
                    ))
                } else if status != "ok" {
                    Some(status.clone())
                } else {
                    None
                }
            }
        };
        if let Some(line) = line {
            self.out.write_line(&text::clean(&line));
        }
    }
}

/// 212 → "212", 6120 → "6.1k", 1_250_000 → "1.2M".
fn short_count(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1_000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

/// The argument worth showing on a tool line: the path, pattern or command.
fn primary_arg(args: &Value) -> Option<String> {
    let object = args.as_object()?;
    let value = ["path", "pattern", "cmd"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .or_else(|| object.values().find_map(Value::as_str))?;
    Some(first_line(value, 60))
}

/// The first line of `text`, cut to `max` chars.
fn first_line(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("");
    if line.chars().count() > max {
        let cut: String = line.chars().take(max).collect();
        format!("{cut}…")
    } else {
        line.to_string()
    }
}
