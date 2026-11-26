//! The TUI's state, and what the keys do to it. No terminal in here: tests drive it with key
//! events and draw it into ratatui's test backend.
//!
//! Data comes from the job files, re-read every second (only the ones that changed). Actions
//! (`diff`, `logs`, `apply`, `discard`) are the CLI's own commands, run in-process: the TUI is a
//! client of the same commands as everyone else, with the same checks.

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::config::OrkConfig;
use crate::events::{EventLine, EventSink, HumanSink};
use crate::io::SharedWriter;
use crate::jobs::{JobState, JobStore};
use crate::orders::JobStatus;
use crate::stats::{JobStats, Ledger, Place};
use crate::tui::theme::{Theme, ThemeName};

/// What the config says about models: where each server runs, and the grunt.
#[derive(Debug, Clone, Default)]
pub struct Setup {
    /// Server name → local or cloud, from its url.
    pub places: BTreeMap<String, Place>,
    pub grunt: Option<Grunt>,
    pub max_turns: u32,
    pub max_repairs: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Grunt {
    pub model: String,
    pub server: String,
    pub context_tokens: u32,
}

impl Setup {
    pub fn from_config(config: &OrkConfig) -> Self {
        Setup {
            places: config
                .servers
                .iter()
                .filter_map(|(name, server)| Some((name.clone(), Place::of_url(&server.url)?)))
                .collect(),
            grunt: config
                .roles
                .get("grunt")
                .filter(|role| !role.model.is_empty())
                .map(|role| Grunt {
                    model: role.model.clone(),
                    server: role.server.clone(),
                    context_tokens: role.context_tokens,
                }),
            max_turns: config.defaults.max_turns,
            max_repairs: config.defaults.max_repairs,
        }
    }

    pub fn place(&self, server: &str) -> Option<Place> {
        self.places.get(server).copied()
    }
}

/// One job, as the TUI shows it.
#[derive(Debug, Clone)]
pub struct JobView {
    pub state: JobState,
    pub stats: JobStats,
    /// The events, as `ork run` prints them for people.
    pub lines: Vec<String>,
    pub max_turns: u32,
    pub max_repairs: u32,
    /// From the report, once there is one.
    pub report: Option<ReportView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReportView {
    pub files: usize,
    pub summary: String,
    pub open_questions: Vec<String>,
}

impl JobView {
    /// How long it ran: until now while it runs, until it finished after. Jobs from before
    /// `finished_at` use their last event's time, or the busy time added up.
    pub fn seconds(&self, now: DateTime<Utc>) -> f64 {
        let start = self.state.created_at;
        let end = match self.state.status {
            JobStatus::Running => Some(now),
            _ => self.state.finished_at.or(self.stats.last_ts),
        };
        match end {
            Some(end) => (end - start).num_milliseconds().max(0) as f64 / 1000.0,
            None => self.stats.busy_seconds(),
        }
    }
}

/// What the TUI asks its runner to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    Refresh,
    Diff {
        job: String,
    },
    Logs {
        job: String,
    },
    Apply {
        job: String,
        squash: bool,
        force: bool,
    },
    Discard {
        job: String,
    },
}

impl Action {
    /// The `ork` command line for it (without the program name); `None` for the TUI's own.
    pub fn args(&self) -> Option<Vec<String>> {
        let mut args: Vec<String> = match self {
            Action::Quit | Action::Refresh => return None,
            Action::Diff { job } => vec!["diff".into(), job.clone()],
            Action::Logs { job } => vec!["logs".into(), job.clone()],
            Action::Discard { job } => vec!["discard".into(), job.clone()],
            Action::Apply { job, .. } => vec!["apply".into(), job.clone()],
        };
        if let Action::Apply { squash, force, .. } = self {
            if *squash {
                args.push("--squash".into());
            }
            if *force {
                args.push("--force".into());
            }
        }
        Some(args)
    }

    /// The line shown while it runs.
    pub fn doing(&self) -> String {
        match self {
            Action::Diff { job } => format!("reading {job}'s diff…"),
            Action::Logs { job } => format!("reading {job}'s transcript…"),
            Action::Apply { job, .. } => format!("applying {job}…"),
            Action::Discard { job } => format!("discarding {job}…"),
            Action::Quit | Action::Refresh => String::new(),
        }
    }
}

/// What an `ork` command printed, and its exit code.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Outcome {
    /// The error message, from the `error: …` line people get on stderr.
    fn error(&self) -> String {
        self.stderr
            .lines()
            .find_map(|line| line.strip_prefix("error: "))
            .or_else(|| self.stderr.lines().rfind(|line| !line.trim().is_empty()))
            .unwrap_or("it failed")
            .to_string()
    }

    fn last_line(&self) -> String {
        self.stdout
            .lines()
            .rfind(|line| !line.trim().is_empty())
            .unwrap_or_default()
            .to_string()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    Jobs,
    Pager(Pager),
}

/// A full-screen text: a diff or a transcript.
#[derive(Debug, Clone, PartialEq)]
pub struct Pager {
    pub kind: PagerKind,
    pub job: String,
    pub lines: Vec<String>,
    /// The first line shown.
    pub top: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagerKind {
    Diff,
    Logs,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Confirm {
    pub question: String,
    pub action: Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Good,
    Bad,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub text: String,
    pub tone: Tone,
}

/// What a job's events were last read as: re-read only when the file grew.
#[derive(Debug, Clone)]
struct Cached {
    events_len: u64,
    stats: JobStats,
    lines: Vec<String>,
    limits: (u32, u32),
    report: Option<ReportView>,
}

pub struct App {
    store: JobStore,
    pub setup: Setup,
    cache: HashMap<String, Cached>,
    /// Newest first, like `ork jobs`.
    pub jobs: Vec<JobView>,
    /// Every job's models, added up.
    pub ledger: Ledger,
    pub selected: usize,
    selected_id: Option<String>,
    pub screen: Screen,
    pub theme: Theme,
    pub confirm: Option<Confirm>,
    pub message: Option<Message>,
    /// Lines up from the end of the selected job's events; 0 follows the tail.
    pub scroll: usize,
    pub now: DateTime<Utc>,
    /// Why the jobs couldn't be read, when they couldn't.
    pub load_error: Option<String>,
    /// The height of the last pager drawn, for paging by a screenful.
    pub page_height: Cell<usize>,
}

impl App {
    pub fn new(store: JobStore, setup: Setup, theme: ThemeName) -> Self {
        App {
            store,
            setup,
            cache: HashMap::new(),
            jobs: Vec::new(),
            ledger: Ledger::default(),
            selected: 0,
            selected_id: None,
            screen: Screen::Jobs,
            theme: Theme::new(theme),
            confirm: None,
            message: None,
            scroll: 0,
            now: Utc::now(),
            load_error: None,
            page_height: Cell::new(20),
        }
    }

    pub fn selected_job(&self) -> Option<&JobView> {
        self.jobs.get(self.selected)
    }

    /// Re-reads the jobs. A job whose files can't be read is left out; if the list itself
    /// can't be read, the old one stays and the error shows.
    pub fn refresh(&mut self, now: DateTime<Utc>) {
        self.now = now;
        let states = match self.store.list() {
            Ok(states) => {
                self.load_error = None;
                states
            }
            Err(err) => {
                self.load_error = Some(format!("{err:#}"));
                return;
            }
        };

        let mut jobs = Vec::with_capacity(states.len());
        for state in states {
            let cached = self.read_job(&state);
            jobs.push(JobView {
                stats: cached.stats.clone(),
                lines: cached.lines.clone(),
                max_turns: cached.limits.0,
                max_repairs: cached.limits.1,
                report: cached.report.clone(),
                state,
            });
        }
        self.cache
            .retain(|id, _| jobs.iter().any(|job| &job.state.id == id));

        let mut ledger = Ledger::default();
        for job in &jobs {
            ledger.add(&job.stats);
        }
        self.ledger = ledger;
        self.jobs = jobs;

        // Keep the same job selected as the list changes; the first time, the running one.
        let index = match &self.selected_id {
            Some(id) => self.jobs.iter().position(|job| &job.state.id == id),
            None => self
                .jobs
                .iter()
                .position(|job| job.state.status == JobStatus::Running),
        };
        self.select(index.unwrap_or(self.selected.min(self.jobs.len().saturating_sub(1))));
    }

    fn read_job(&mut self, state: &JobState) -> &Cached {
        let paths = self.store.paths(&state.id);
        let events_len = std::fs::metadata(&paths.events).map_or(0, |m| m.len());
        let stale = self
            .cache
            .get(&state.id)
            .is_none_or(|cached| cached.events_len != events_len);
        if stale {
            let text = std::fs::read_to_string(&paths.events).unwrap_or_default();
            let limits = self.store.load_order(&state.id).map_or(
                (self.setup.max_turns, self.setup.max_repairs),
                |order| {
                    (
                        order.max_turns.unwrap_or(self.setup.max_turns),
                        order.max_repairs.unwrap_or(self.setup.max_repairs),
                    )
                },
            );
            let cached = Cached {
                events_len,
                stats: JobStats::from_log(&text),
                lines: human_lines(&text),
                limits,
                report: None,
            };
            self.cache.insert(state.id.clone(), cached);
        }
        let cached = self.cache.get_mut(&state.id).expect("just cached");
        // The report comes once, when the run ends.
        if cached.report.is_none() && state.status != JobStatus::Running {
            cached.report = self
                .store
                .load_report(&state.id)
                .ok()
                .flatten()
                .map(|report| ReportView {
                    files: report.files_changed.len(),
                    summary: report.summary,
                    open_questions: report.open_questions,
                });
        }
        cached
    }

    fn select(&mut self, index: usize) {
        if index != self.selected {
            self.scroll = 0;
        }
        self.selected = index;
        self.selected_id = self.jobs.get(index).map(|job| job.state.id.clone());
    }

    fn say(&mut self, text: impl Into<String>, tone: Tone) {
        self.message = Some(Message {
            text: text.into(),
            tone,
        });
    }

    /// Handles a key; returns what the runner has to do, if anything.
    pub fn on_key(&mut self, key: KeyEvent) -> Option<Action> {
        // Windows reports releases too.
        if key.kind != KeyEventKind::Press {
            return None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Some(Action::Quit);
        }
        if let Some(confirm) = self.confirm.take() {
            return match key.code {
                KeyCode::Char('y' | 'Y') | KeyCode::Enter => Some(confirm.action),
                KeyCode::Char('n' | 'N') | KeyCode::Esc => None,
                _ => {
                    self.confirm = Some(confirm);
                    None
                }
            };
        }
        self.message = None;
        match &mut self.screen {
            Screen::Pager(pager) => {
                let page = self.page_height.get().max(1);
                let last = pager.lines.len().saturating_sub(page);
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => self.screen = Screen::Jobs,
                    KeyCode::Up | KeyCode::Char('k') => pager.top = pager.top.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => pager.top = (pager.top + 1).min(last),
                    KeyCode::PageUp => pager.top = pager.top.saturating_sub(page),
                    KeyCode::PageDown | KeyCode::Char(' ') => {
                        pager.top = (pager.top + page).min(last)
                    }
                    KeyCode::Home | KeyCode::Char('g') => pager.top = 0,
                    KeyCode::End | KeyCode::Char('G') => pager.top = last,
                    _ => {}
                }
                None
            }
            Screen::Jobs => self.on_jobs_key(key.code),
        }
    }

    fn on_jobs_key(&mut self, code: KeyCode) -> Option<Action> {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return Some(Action::Quit),
            KeyCode::Char('r') => return Some(Action::Refresh),
            KeyCode::Char('t') => self.theme = Theme::new(self.theme.name.toggled()),
            KeyCode::Up | KeyCode::Char('k') => self.select(self.selected.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => {
                self.select((self.selected + 1).min(self.jobs.len().saturating_sub(1)))
            }
            KeyCode::Home => self.select(0),
            KeyCode::End => self.select(self.jobs.len().saturating_sub(1)),
            KeyCode::PageUp => {
                let lines = self.selected_job().map_or(0, |job| job.lines.len());
                self.scroll = (self.scroll + 10).min(lines.saturating_sub(1));
            }
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::Char('d') | KeyCode::Enter => return self.job_action(JobKey::Diff),
            KeyCode::Char('l') => return self.job_action(JobKey::Logs),
            KeyCode::Char('a') => return self.job_action(JobKey::Apply { squash: false }),
            KeyCode::Char('s') => return self.job_action(JobKey::Apply { squash: true }),
            KeyCode::Char('x') => return self.job_action(JobKey::Discard),
            _ => {}
        }
        None
    }

    /// A key about the selected job: the action, a question first, or why not.
    fn job_action(&mut self, key: JobKey) -> Option<Action> {
        let job = self.selected_job()?;
        let id = job.state.id.clone();
        let status = job.state.status;
        let gone = matches!(status, JobStatus::Applied | JobStatus::Discarded);
        match key {
            JobKey::Diff if gone => self.say(
                format!("{id} was {}; its branch is gone", status.as_str()),
                Tone::Info,
            ),
            JobKey::Diff => return Some(Action::Diff { job: id }),
            JobKey::Logs => return Some(Action::Logs { job: id }),
            JobKey::Apply { squash } => match status {
                JobStatus::Passed => {
                    let how = if squash {
                        "Squash it into one commit on"
                    } else {
                        "Merge it into"
                    };
                    self.confirm = Some(Confirm {
                        question: format!("{id} passed. {how} your current branch?"),
                        action: Action::Apply {
                            job: id,
                            squash,
                            force: false,
                        },
                    });
                }
                JobStatus::FailedChecks | JobStatus::GaveUp => {
                    self.confirm = Some(Confirm {
                        question: format!(
                            "{id} is {}. Apply it anyway (--force), to finish it yourself?",
                            label(status)
                        ),
                        action: Action::Apply {
                            job: id,
                            squash,
                            force: true,
                        },
                    });
                }
                JobStatus::Running => self.say(format!("{id} is still running"), Tone::Info),
                JobStatus::Error => self.say(
                    format!("{id} ended in an error; there's nothing to apply"),
                    Tone::Info,
                ),
                JobStatus::Applied | JobStatus::Discarded => {
                    self.say(format!("{id} was {} already", status.as_str()), Tone::Info)
                }
            },
            JobKey::Discard => match status {
                JobStatus::Running => self.say(
                    format!("{id} is still running; let it finish first"),
                    Tone::Info,
                ),
                JobStatus::Applied | JobStatus::Discarded => {
                    self.say(format!("{id} was {} already", status.as_str()), Tone::Info)
                }
                _ => {
                    self.confirm = Some(Confirm {
                        question: format!(
                            "Discard {id}? Its worktree and branch go; the logs stay."
                        ),
                        action: Action::Discard { job: id },
                    });
                }
            },
        }
        None
    }

    /// Takes in what an action's command printed.
    pub fn done(&mut self, action: &Action, outcome: Outcome) {
        if outcome.code != 0 {
            let error = outcome.error();
            self.say(error, Tone::Bad);
            return;
        }
        match action {
            Action::Diff { job } => {
                self.screen = Screen::Pager(Pager {
                    kind: PagerKind::Diff,
                    job: job.clone(),
                    lines: lines_or(&outcome.stdout, "no changes"),
                    top: 0,
                })
            }
            Action::Logs { job } => {
                self.screen = Screen::Pager(Pager {
                    kind: PagerKind::Logs,
                    job: job.clone(),
                    lines: lines_or(&outcome.stdout, "no transcript yet"),
                    top: 0,
                })
            }
            Action::Apply { .. } | Action::Discard { .. } => {
                let line = outcome.last_line();
                self.say(line, Tone::Good);
            }
            Action::Quit | Action::Refresh => {}
        }
    }
}

enum JobKey {
    Diff,
    Logs,
    Apply { squash: bool },
    Discard,
}

/// `failed_checks` → "failed checks".
pub fn label(status: JobStatus) -> String {
    status.as_str().replace('_', " ")
}

fn lines_or(text: &str, empty: &str) -> Vec<String> {
    let lines: Vec<String> = text.lines().map(String::from).collect();
    if lines.is_empty() {
        vec![empty.to_string()]
    } else {
        lines
    }
}

/// The events as `ork run` shows them on stderr: the same sink, written into memory.
fn human_lines(text: &str) -> Vec<String> {
    let (out, captured) = SharedWriter::capture();
    let sink = HumanSink::new(out);
    for line in text.lines().filter_map(EventLine::parse) {
        sink.emit(&line.event);
    }
    captured.text().lines().map(String::from).collect()
}
