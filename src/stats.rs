//! Numbers from a job's events: tokens and time per model, the turn-by-turn track, how the
//! context grew. `ork tui` shows them, and the web UI (#31) will. Pure: it reads lines of
//! `events.jsonl`, nothing else.

use std::collections::BTreeMap;
use std::net::IpAddr;

use chrono::{DateTime, Utc};

use crate::events::{EventLine, OrkEvent};

/// Where a server runs: on your own machines, or a provider's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Local,
    Cloud,
}

impl Place {
    /// From a server's url. Loopback, private and link-local addresses, Tailscale's `100.64/10`,
    /// and names like `box`, `box.local`, `box.lan` or `box.tail1234.ts.net` are local; anything
    /// else is a provider's. `None` for a url that doesn't parse.
    pub fn of_url(url: &str) -> Option<Place> {
        let url = reqwest::Url::parse(url).ok()?;
        let host = url.host_str()?;
        Some(if is_local_host(host) {
            Place::Local
        } else {
            Place::Cloud
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Place::Local => "local",
            Place::Cloud => "cloud",
        }
    }
}

fn is_local_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(ip) => {
                let [a, b, ..] = ip.octets();
                ip.is_loopback()
                    || ip.is_private()
                    || ip.is_link_local()
                    || ip.is_unspecified()
                    || (a == 100 && (64..128).contains(&b))
            }
            IpAddr::V6(ip) => {
                let first = ip.segments()[0];
                ip.is_loopback()
                    || ip.is_unspecified()
                    || (first & 0xfe00) == 0xfc00 // unique local, fc00::/7
                    || (first & 0xffc0) == 0xfe80 // link-local, fe80::/10
            }
        };
    }
    let host = host.to_ascii_lowercase();
    const LOCAL_SUFFIXES: [&str; 7] = [
        ".localhost",
        ".local",
        ".lan",
        ".home",
        ".internal",
        ".home.arpa",
        ".ts.net",
    ];
    host == "localhost" || !host.contains('.') || LOCAL_SUFFIXES.iter().any(|s| host.ends_with(s))
}

/// What one model did: the sum of its requests.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelStats {
    pub calls: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    /// Wall time of the requests, prompt processing included.
    pub seconds: f64,
    /// Time spent writing the output: each request's output tokens over the server's own speed
    /// figure where it gave one (llama.cpp does), else its wall time.
    pub generating_seconds: f64,
    /// The sum over the requests that reported a cost; `None` when none did (local servers).
    pub cost: Option<f64>,
}

impl ModelStats {
    fn add_call(
        &mut self,
        tokens_in: u64,
        tokens_out: u64,
        seconds: f64,
        tokens_per_second: Option<f64>,
        cost: Option<f64>,
    ) {
        self.calls += 1;
        self.tokens_in += tokens_in;
        self.tokens_out += tokens_out;
        self.seconds += seconds;
        self.generating_seconds += match tokens_per_second {
            Some(tps) if tps > 0.0 => tokens_out as f64 / tps,
            _ => seconds,
        };
        if let Some(cost) = cost {
            *self.cost.get_or_insert(0.0) += cost;
        }
    }

    /// Adds another's numbers to these.
    pub fn merge(&mut self, other: &ModelStats) {
        self.calls += other.calls;
        self.tokens_in += other.tokens_in;
        self.tokens_out += other.tokens_out;
        self.seconds += other.seconds;
        self.generating_seconds += other.generating_seconds;
        if let Some(cost) = other.cost {
            *self.cost.get_or_insert(0.0) += cost;
        }
    }

    /// Output tokens per second of writing them, over all the requests: long replies count for
    /// more than short ones.
    pub fn tokens_per_second(&self) -> Option<f64> {
        (self.tokens_out > 0 && self.generating_seconds > 0.0)
            .then(|| self.tokens_out as f64 / self.generating_seconds)
    }
}

/// A model on a server: the ledger's key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModelKey {
    pub model: String,
    pub server: String,
}

/// One step of a job, in order: the track the TUI draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// A model turn: how many tools it called, and how many of them failed. No tools at all is a
    /// text-only reply (the grunt got nudged).
    Turn { turn: u32, tools: u32, failed: u32 },
    /// The grunt called `finish`.
    Finish,
    /// A check command ran.
    Check { passed: bool },
    /// A repair turn started.
    Repair,
}

/// The latest model request: what the live speed meter shows.
#[derive(Debug, Clone, PartialEq)]
pub struct LastCall {
    pub model: String,
    pub server: String,
    pub tokens_in: u64,
    pub tokens_per_second: Option<f64>,
    pub ts: Option<DateTime<Utc>>,
}

/// Everything one job's events add up to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JobStats {
    /// The last turn seen.
    pub turns: u32,
    pub tools: u32,
    pub tool_errors: u32,
    pub tool_seconds: f64,
    pub checks_passed: u32,
    pub checks_failed: u32,
    pub check_seconds: f64,
    pub repairs: u32,
    pub warnings: u32,
    /// All the requests together.
    pub total: ModelStats,
    /// Per model and server.
    pub models: BTreeMap<ModelKey, ModelStats>,
    pub track: Vec<Mark>,
    /// The prompt size of each request: how the context grew (and shrank when stubbed).
    pub context: Vec<u64>,
    pub last_call: Option<LastCall>,
    /// The `result` status, once the job wrote one.
    pub result: Option<String>,
    /// The times of the first and the last event that had one.
    pub first_ts: Option<DateTime<Utc>>,
    pub last_ts: Option<DateTime<Utc>>,
}

impl JobStats {
    /// The stats of a whole `events.jsonl`. Lines that don't parse (a log cut mid-line) are
    /// skipped.
    pub fn from_log(text: &str) -> JobStats {
        let mut stats = JobStats::default();
        for line in text.lines().filter_map(EventLine::parse) {
            stats.push(&line);
        }
        stats
    }

    /// Adds one event.
    pub fn push(&mut self, line: &EventLine) {
        if let Some(ts) = line.ts {
            self.first_ts.get_or_insert(ts);
            self.last_ts = Some(ts);
        }
        match &line.event {
            OrkEvent::Call {
                turn,
                model,
                server,
                tokens_in,
                tokens_out,
                seconds,
                tokens_per_second,
                cost,
            } => {
                self.turns = self.turns.max(*turn);
                let key = ModelKey {
                    model: model.clone(),
                    server: server.clone(),
                };
                for stats in [&mut self.total, self.models.entry(key).or_default()] {
                    stats.add_call(*tokens_in, *tokens_out, *seconds, *tokens_per_second, *cost);
                }
                self.track.push(Mark::Turn {
                    turn: *turn,
                    tools: 0,
                    failed: 0,
                });
                self.context.push(*tokens_in);
                self.last_call = Some(LastCall {
                    model: model.clone(),
                    server: server.clone(),
                    tokens_in: *tokens_in,
                    tokens_per_second: *tokens_per_second,
                    ts: line.ts,
                });
            }
            OrkEvent::Tool { ok, ms, .. } => {
                self.tools += 1;
                self.tool_seconds += *ms as f64 / 1000.0;
                if !ok {
                    self.tool_errors += 1;
                }
                if let Some(Mark::Turn { tools, failed, .. }) = self.track.last_mut() {
                    *tools += 1;
                    if !ok {
                        *failed += 1;
                    }
                }
            }
            OrkEvent::Finish { .. } => {
                // `finish` is a tool call too, though it comes as its own event.
                if let Some(Mark::Turn { tools, .. }) = self.track.last_mut() {
                    *tools += 1;
                }
                self.track.push(Mark::Finish);
            }
            OrkEvent::Check { exit, seconds, .. } => {
                let passed = *exit == 0;
                if passed {
                    self.checks_passed += 1;
                } else {
                    self.checks_failed += 1;
                }
                self.check_seconds += seconds;
                self.track.push(Mark::Check { passed });
            }
            OrkEvent::Repair { .. } => {
                self.repairs += 1;
                self.track.push(Mark::Repair);
            }
            OrkEvent::Warning { .. } => self.warnings += 1,
            OrkEvent::Result { status, .. } => self.result = Some(status.clone()),
            OrkEvent::Start { .. } => {}
        }
    }

    /// The seconds the job was busy: model, tool and check time added up. For jobs from before
    /// events had times, it's the only measure of how long they took.
    pub fn busy_seconds(&self) -> f64 {
        self.total.seconds + self.tool_seconds + self.check_seconds
    }
}

/// The ledger of many jobs: every model's numbers, added up.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ledger {
    pub models: BTreeMap<ModelKey, ModelStats>,
}

impl Ledger {
    pub fn add(&mut self, job: &JobStats) {
        for (key, stats) in &job.models {
            self.models.entry(key.clone()).or_default().merge(stats);
        }
    }

    pub fn total(&self) -> ModelStats {
        let mut total = ModelStats::default();
        for stats in self.models.values() {
            total.merge(stats);
        }
        total
    }
}
