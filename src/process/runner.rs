//! Running one command: no shell, a scrubbed environment, a timeout, the whole process tree
//! killed when it's over.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use process_wrap::tokio::*;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::sync::CancellationToken;

use super::find;
use crate::env::Env;

/// Variables a build or test needs, copied from the environment `ork` runs in. Everything else
/// (API keys, tokens, cloud credentials) is left out.
const PASS_NAMES: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LANG",
    "LC_ALL",
    "TMPDIR",
    "TEMP",
    "TMP",
    // Windows
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "PATHEXT",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    // toolchains (named one by one: CARGO_* and NUGET_* can carry tokens)
    "CARGO_HOME",
    // a shared build cache: jobs reuse one target dir instead of each building from scratch
    "CARGO_TARGET_DIR",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "GOPATH",
    "GOCACHE",
    "GOMODCACHE",
    "NUGET_PACKAGES",
];
const PASS_PREFIXES: &[&str] = &["DOTNET_"];
/// Set for every command: no colours, no prompts, no telemetry.
const SET: &[(&str, &str)] = &[
    ("CI", "true"),
    ("NO_COLOR", "1"),
    ("TERM", "dumb"),
    ("CARGO_TERM_COLOR", "never"),
    ("DOTNET_CLI_TELEMETRY_OPTOUT", "1"),
    ("DOTNET_NOLOGO", "1"),
];

/// Output kept per command: the first and the last half of this.
const MAX_CAPTURE: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessResult {
    /// -1 when there is none (timed out, killed by a signal).
    pub exit_code: i32,
    /// stdout and stderr together, in the order they arrived.
    pub output: String,
    pub timed_out: bool,
    pub seconds: f64,
}

#[derive(Debug, Clone)]
pub struct ProcessRunner {
    env: Env,
}

impl ProcessRunner {
    /// `env` is where `ork` runs: the program is looked up on its PATH, and the allowed variables
    /// are copied from it.
    pub fn new(env: Env) -> Self {
        Self { env }
    }

    pub fn env(&self) -> &Env {
        &self.env
    }

    /// The environment a command gets.
    pub fn child_env(&self) -> Vec<(String, String)> {
        let same = |a: &str, b: &str| {
            if cfg!(windows) {
                a.eq_ignore_ascii_case(b)
            } else {
                a == b
            }
        };
        let mut vars: Vec<(String, String)> = self
            .env
            .iter()
            .filter(|(name, _)| {
                PASS_NAMES.iter().any(|pass| same(pass, name))
                    || PASS_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
            })
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        vars.extend(
            SET.iter()
                .map(|(name, value)| (name.to_string(), value.to_string())),
        );
        vars
    }

    /// Runs the command to the end, or until the timeout (killed; `timed_out`), or until `cancel`
    /// (killed; an error). Not finding the program is an error too.
    pub async fn run(
        &self,
        spec: ProcessSpec,
        cancel: &CancellationToken,
    ) -> anyhow::Result<ProcessResult> {
        let started = Instant::now();
        let name = spec.argv.first().context("no command to run")?;
        let program = find(name, &self.env, &spec.cwd)
            .with_context(|| format!("{name}: not found on PATH"))?;

        let mut command = tokio::process::Command::new(&program);
        command
            .args(&spec.argv[1..])
            .current_dir(&spec.cwd)
            .env_clear()
            .envs(self.child_env())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Its own process group (Unix) or Job Object (Windows), so a kill reaches the whole tree:
        // `cargo test` starts the test binaries, a test may start a server. KillOnDrop: if this
        // future is dropped (the loop's deadline or Ctrl+C won a `select!`), the tree dies too.
        let mut wrapped = CommandWrap::from(command);
        #[cfg(unix)]
        wrapped.wrap(ProcessGroup::leader());
        #[cfg(windows)]
        wrapped.wrap(JobObject);
        wrapped.wrap(KillOnDrop);
        let mut child = wrapped
            .spawn()
            .with_context(|| format!("starting {}", program.display()))?;

        // Read both pipes while it runs (a full pipe would block it), into one buffer, in the
        // order the output arrives.
        let capture = Arc::new(Mutex::new(Capture::default()));
        let mut readers = Vec::new();
        if let Some(stdout) = child.stdout().take() {
            readers.push(tokio::spawn(read_into(stdout, capture.clone())));
        }
        if let Some(stderr) = child.stderr().take() {
            readers.push(tokio::spawn(read_into(stderr, capture.clone())));
        }

        enum End {
            Exited(i32),
            TimedOut,
            Cancelled,
        }
        let end = tokio::select! {
            status = child.wait() => End::Exited(status?.code().unwrap_or(-1)),
            _ = tokio::time::sleep(spec.timeout) => End::TimedOut,
            _ = cancel.cancelled() => End::Cancelled,
        };
        if !matches!(end, End::Exited(_)) {
            let _ = Box::into_pin(child.kill()).await;
        }
        // The pipes close when the tree is gone. Don't hang on a straggler that escaped it.
        for reader in readers {
            let _ = tokio::time::timeout(Duration::from_secs(5), reader).await;
        }

        let (exit_code, timed_out) = match end {
            End::Exited(code) => (code, false),
            End::TimedOut => (-1, true),
            End::Cancelled => bail!("cancelled"),
        };
        let output = capture
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .text();
        Ok(ProcessResult {
            exit_code,
            output,
            timed_out,
            seconds: started.elapsed().as_secs_f64(),
        })
    }
}

async fn read_into(mut pipe: impl AsyncRead + Unpin, capture: Arc<Mutex<Capture>>) {
    let mut buf = [0u8; 8192];
    while let Ok(n) = pipe.read(&mut buf).await {
        if n == 0 {
            break;
        }
        capture
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(&buf[..n]);
    }
}

/// The first and last `MAX_CAPTURE / 2` bytes of the output, and how much was dropped between.
#[derive(Default)]
struct Capture {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    dropped: usize,
}

impl Capture {
    fn push(&mut self, bytes: &[u8]) {
        let half = MAX_CAPTURE / 2;
        let into_head = bytes.len().min(half - self.head.len());
        self.head.extend_from_slice(&bytes[..into_head]);
        self.tail.extend(&bytes[into_head..]);
        if self.tail.len() > half {
            let excess = self.tail.len() - half;
            self.tail.drain(..excess);
            self.dropped += excess;
        }
    }

    fn text(&self) -> String {
        let mut bytes = self.head.clone();
        if self.dropped > 0 {
            bytes.extend_from_slice(format!("\n…[{} bytes cut]…\n", self.dropped).as_bytes());
        }
        bytes.extend(self.tail.iter());
        String::from_utf8_lossy(&bytes).into_owned()
    }
}
