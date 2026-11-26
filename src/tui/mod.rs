//! `ork tui`: a live view of the repo's jobs and models, for people (agents use `ork jobs --json`
//! and `ork status JOB --json`). It reads the job files, so it shows runs started anywhere:
//! another terminal, or an agent's shell.
//!
//! - `app`: the state and the keys, no terminal (tested with key events);
//! - `ui`: drawing it with ratatui (tested against its test backend);
//! - `theme`: the brand's colours;
//! - here: the loop that ties them to the terminal, and runs the CLI's commands for the actions.

pub mod app;
pub mod theme;
pub mod ui;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::Utc;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use crate::cli::{self, CliIo};
use crate::env::Env;
use crate::tui::app::{Action, App, Message, Outcome, Tone};

/// How often the screen redraws (the spinner, the clock), and how often the job files are read.
const TICK: Duration = Duration::from_millis(100);
const REFRESH: Duration = Duration::from_secs(1);

/// Runs `ork` commands for the TUI: in-process, with their output captured, exactly as if typed.
pub struct Ork {
    pub handle: Handle,
    pub cwd: PathBuf,
    pub env: Env,
    pub cancel: CancellationToken,
}

impl Ork {
    /// Runs `ork <args>` and returns what it printed. Call it from a blocking thread: it waits
    /// on the async command with the runtime's handle.
    pub fn run(&self, args: Vec<String>) -> Outcome {
        let (io, stdout, stderr) = CliIo::capture();
        let argv = std::iter::once("ork".to_string()).chain(args).collect();
        let code = self.handle.block_on(cli::run(
            argv,
            io,
            self.cwd.clone(),
            self.env.clone(),
            self.cancel.child_token(),
        ));
        Outcome {
            code,
            stdout: stdout.text(),
            stderr: stderr.text(),
        }
    }
}

/// Takes over the terminal until the user quits (or Ctrl+C). The terminal is put back even on
/// a panic: `ratatui::init` installs a hook for that.
pub fn run(mut app: App, ork: Ork) -> anyhow::Result<()> {
    let mut terminal = ratatui::try_init()?;
    let result = event_loop(&mut terminal, &mut app, &ork);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut DefaultTerminal, app: &mut App, ork: &Ork) -> anyhow::Result<()> {
    app.refresh(Utc::now());
    let mut refreshed = Instant::now();
    loop {
        if ork.cancel.is_cancelled() {
            return Ok(());
        }
        app.now = Utc::now();
        terminal.draw(|frame| ui::draw(frame, app))?;

        if event::poll(TICK)?
            && let Event::Key(key) = event::read()?
            && let Some(action) = app.on_key(key)
        {
            match action {
                Action::Quit => return Ok(()),
                Action::Refresh => {}
                action => {
                    // Say what's happening first: a diff or a merge can take a moment.
                    app.message = Some(Message {
                        text: action.doing(),
                        tone: Tone::Info,
                    });
                    terminal.draw(|frame| ui::draw(frame, app))?;
                    let args = action.args().expect("a command action");
                    let outcome = ork.run(args);
                    app.done(&action, outcome);
                }
            }
            app.refresh(Utc::now());
            refreshed = Instant::now();
        }
        if refreshed.elapsed() >= REFRESH {
            app.refresh(Utc::now());
            refreshed = Instant::now();
        }
    }
}
