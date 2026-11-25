//! The checks. When the grunt calls `finish`, orkestr8R runs the order's commands itself and the
//! exit codes decide. The model saying "done, the tests pass" counts for nothing: a local model
//! will happily claim success for code that doesn't compile.

use std::path::Path;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::events::{EventSink, OrkEvent};
use crate::orders::CheckResult;
use crate::process::{CommandPolicy, HEAD_TAIL_MAX, ProcessRunner, ProcessSpec, head_tail};

/// Lines kept in the report and the `check` event: enough to see what failed.
const TAIL_LINES: usize = 15;
/// Lines sent back to the grunt with a failure: enough to fix it.
const FEEDBACK_LINES: usize = 60;
/// Characters, at most, for the report's tail (a line can be very long).
const TAIL_CHARS: usize = 2_000;

/// What one round of checks found.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckRun {
    /// Every check that ran, in order; the last one failed if any did.
    pub results: Vec<CheckResult>,
    pub failure: Option<CheckFailure>,
}

impl CheckRun {
    pub fn passed(&self) -> bool {
        self.failure.is_none()
    }
}

/// The check that failed, with the output the grunt gets back.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckFailure {
    pub cmd: String,
    pub exit: i32,
    /// The last 60 lines of its output.
    pub output: String,
}

#[derive(Debug, Clone)]
pub struct CheckRunner {
    policy: CommandPolicy,
    runner: ProcessRunner,
    timeout: Duration,
}

impl CheckRunner {
    pub fn new(policy: CommandPolicy, runner: ProcessRunner, timeout: Duration) -> Self {
        Self {
            policy,
            runner,
            timeout,
        }
    }

    /// Runs `checks` in order in `root`, stopping at the first failure, and emits a `check` event
    /// for each. A refused, missing or timed-out command is a failure (exit -1). Only a cancel is
    /// an error.
    pub async fn run(
        &self,
        checks: &[String],
        root: &Path,
        events: &dyn EventSink,
        cancel: &CancellationToken,
    ) -> anyhow::Result<CheckRun> {
        let mut results = Vec::new();
        for cmd in checks {
            let (exit, seconds, output) = match self.policy.check(cmd) {
                Err(refused) => (-1, 0.0, refused),
                Ok(argv) => {
                    let spec = ProcessSpec {
                        argv,
                        cwd: root.to_path_buf(),
                        timeout: self.timeout,
                    };
                    match self.runner.run(spec, cancel).await {
                        Ok(result) if result.timed_out => (
                            -1,
                            result.seconds,
                            format!(
                                "{}\ntimed out after {}s and was stopped",
                                result.output,
                                self.timeout.as_secs()
                            ),
                        ),
                        Ok(result) => (result.exit_code, result.seconds, result.output),
                        Err(err) if cancel.is_cancelled() => return Err(err),
                        Err(err) => (-1, 0.0, format!("{err:#}")),
                    }
                }
            };

            let tail = head_tail(&last_lines(&output, TAIL_LINES), TAIL_CHARS);
            events.emit(&OrkEvent::Check {
                cmd: cmd.clone(),
                exit,
                seconds,
                tail: tail.clone(),
            });
            results.push(CheckResult {
                cmd: cmd.clone(),
                exit,
                seconds,
                tail,
            });
            if exit != 0 {
                let output = head_tail(&last_lines(&output, FEEDBACK_LINES), HEAD_TAIL_MAX);
                let failure = CheckFailure {
                    cmd: cmd.clone(),
                    exit,
                    output,
                };
                return Ok(CheckRun {
                    results,
                    failure: Some(failure),
                });
            }
        }
        Ok(CheckRun {
            results,
            failure: None,
        })
    }
}

/// The last `n` lines of `text`.
pub fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}
