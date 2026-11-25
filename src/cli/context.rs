//! What every command handler gets, and what it returns.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{CliIo, exit};
use crate::env::Env;
use crate::events::{EventSink, HumanSink, JsonLinesSink, OrkEvent};
use crate::io::SharedWriter;
use crate::orders::Report;
use crate::text;

pub struct CliContext {
    pub stdout: SharedWriter,
    pub stderr: SharedWriter,
    pub cwd: PathBuf,
    pub env: Env,
    pub cancel: CancellationToken,
    /// `--json`: events as JSON Lines on stdout, instead of human lines on stderr.
    pub json: bool,
    pub events: Arc<dyn EventSink>,
}

impl CliContext {
    pub fn new(io: CliIo, cwd: PathBuf, env: Env, cancel: CancellationToken, json: bool) -> Self {
        let events: Arc<dyn EventSink> = if json {
            Arc::new(JsonLinesSink::new(io.stdout.clone()))
        } else {
            Arc::new(HumanSink::new(io.stderr.clone()))
        };
        Self {
            stdout: io.stdout,
            stderr: io.stderr,
            cwd,
            env,
            cancel,
            json,
            events,
        }
    }

    /// A line of the command's human output: to stdout, and only without `--json` (with it, the
    /// same information goes into `result.data`).
    pub fn say(&self, line: &str) {
        if !self.json {
            // Lines often carry the model's text (summaries, logs): keep the terminal safe.
            self.stdout.write_line(&text::clean(line));
        }
    }

    /// A note for people, on stderr, in both modes.
    pub fn note(&self, line: &str) {
        self.stderr.write_line(line);
    }

    pub fn emit(&self, event: OrkEvent) {
        self.events.emit(&event);
    }
}

/// What a command ends with: the fields of the `result` event.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandResult {
    pub status: String,
    pub exit_code: i32,
    pub job: Option<String>,
    pub report: Option<Report>,
    pub data: Option<Value>,
    pub error: Option<String>,
}

impl CommandResult {
    pub fn new(status: impl Into<String>, exit_code: i32) -> Self {
        Self {
            status: status.into(),
            exit_code,
            job: None,
            report: None,
            data: None,
            error: None,
        }
    }

    /// Status `ok`, exit 0.
    pub fn ok() -> Self {
        Self::new("ok", exit::OK)
    }

    /// Status `ok`, exit 0, with the command's output in `data`.
    pub fn ok_with(data: impl Serialize) -> Self {
        Self::ok().with_data(data)
    }

    /// Status `error`, exit 1.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            error: Some(message.into()),
            ..Self::new("error", exit::ERROR)
        }
    }

    pub fn with_data(mut self, data: impl Serialize) -> Self {
        self.data = serde_json::to_value(data).ok();
        self
    }

    pub fn into_event(self) -> OrkEvent {
        OrkEvent::Result {
            status: self.status,
            exit_code: self.exit_code,
            job: self.job,
            report: self.report,
            data: self.data,
            error: self.error,
        }
    }
}
