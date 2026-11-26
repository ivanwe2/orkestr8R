//! Drawing. The screen, top to bottom:
//!
//! ```text
//!  ●●● orkestr8R   ● WORKING · 1   ◆ YOUR TURN · 2          grunt qwen38 @ box · local · 64k ctx
//! ╭ JOBS ───────────────────────────────╮╭ MODELS ─────────────────────────────────────╮
//! │ ⠹ k58c running        1m   -   Tes… ││ GRUNT · qwen38 @ box · local                 │
//! │   2rjf passed         2h 2 files #… ││ 47 tok/s ▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮▯▯▯▯▯▯ live         │
//! ╰─────────────────────────────────────╯│ model   where calls    in   out tok/s cost  │
//! ╭ RUNNING · TURN 12/40 · REPAIRS 0/2 ────────────────────────────────── 00:03:42 ╮
//! │ k58c  Tests for src/text.rs                                                   │
//! │ TRACK    ▮▮▮▮·▮▮▮◆✗↻▮▮◆✓                                                     │
//! │ CONTEXT  ▁▂▂▃▄▄▅▆                    41.0k of 64k                             │
//! │   t12  read_file src/text.rs   ok  3ms   6.1k→212 tok  52 t/s                │
//! ╰───────────────────────────────────────────────────────────────────────────────╯
//!  ↑↓ job  d diff  l logs  a apply  s squash  x discard  t theme  q quit
//! ```
//!
//! localdub's control room gave the ideas: the status lamp, the LED speed meter on a log scale,
//! the timeline track, timecode clocks. The look is orkestr8R's own (docs/brand.md).

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, Padding, Paragraph, Row, Sparkline, Table, TableState,
    Wrap,
};

use crate::events::short_count;
use crate::orders::JobStatus;
use crate::stats::{JobStats, Mark, ModelStats, Place};
use crate::tui::app::{App, JobView, Pager, PagerKind, Screen, Tone, label};
use crate::tui::theme::Theme;

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The speed meter's scale: log, 1 to 300 tokens per second, like localdub's.
const METER_MAX: f64 = 300.0;
const METER_TICKS: [u32; 5] = [1, 10, 50, 100, 300];

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    frame.render_widget(Block::new().style(app.theme.base()), area);
    match &app.screen {
        Screen::Jobs => draw_jobs_screen(frame, app, area),
        Screen::Pager(pager) => draw_pager(frame, app, pager, area),
    }
    if let Some(confirm) = &app.confirm {
        draw_confirm(frame, app, &confirm.question, area);
    }
}

fn draw_jobs_screen(frame: &mut Frame, app: &App, area: Rect) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);
    draw_header(frame, app, header);

    // Tall enough for the jobs and the ledger, no taller than half: the job view gets the rest.
    let ledger_rows = app.ledger.models.len() + usize::from(app.ledger.models.len() > 1);
    let wanted = (app.jobs.len() + 3).max(ledger_rows + 7).max(9) as u16;
    let top_height = wanted.min(body.height / 2).max(9.min(body.height));
    let [top, detail] =
        Layout::vertical([Constraint::Length(top_height), Constraint::Min(0)]).areas(body);
    if top.width >= 96 {
        let [jobs, models] =
            Layout::horizontal([Constraint::Percentage(54), Constraint::Percentage(46)]).areas(top);
        draw_jobs(frame, app, jobs);
        draw_models(frame, app, models);
    } else {
        draw_jobs(frame, app, top);
    }
    draw_detail(frame, app, detail);
    draw_footer(
        frame,
        app,
        footer,
        &[
            ("↑↓", "job"),
            ("d", "diff"),
            ("l", "logs"),
            ("a", "apply"),
            ("s", "squash"),
            ("x", "discard"),
            ("t", "theme"),
            ("q", "quit"),
        ],
    );
}

fn panel<'a>(t: &Theme, title: impl Into<Line<'a>>) -> Block<'a> {
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(t.border())
        .padding(Padding::horizontal(1))
        .title(title)
}

fn kicker<'a>(t: &Theme, text: impl Into<String>) -> Span<'a> {
    Span::styled(format!(" {} ", text.into()), t.kicker())
}

// --- the header --------------------------------------------------------------------------------

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let bold = Modifier::BOLD;
    let running = count(app, |s| s == JobStatus::Running);
    let yours = count(app, |s| {
        matches!(
            s,
            JobStatus::Passed | JobStatus::FailedChecks | JobStatus::GaveUp
        )
    });

    // The mark's topknot: one amber chief, two green grunts.
    let mut left = vec![
        Span::raw(" "),
        Span::styled("●", t.fg(t.amber)),
        Span::styled("●●", t.fg(t.green)),
        Span::raw(" "),
        Span::styled("ork", t.fg(t.green).add_modifier(bold)),
        Span::styled("estr", t.fg(t.text).add_modifier(bold)),
        Span::styled("8", t.fg(t.amber).add_modifier(bold)),
        Span::styled("R", t.fg(t.text).add_modifier(bold)),
        Span::raw("   "),
    ];
    if running > 0 {
        left.push(Span::styled(
            format!("● WORKING · {running}"),
            t.fg(t.green).add_modifier(bold),
        ));
        left.push(Span::raw("   "));
    }
    if yours > 0 {
        left.push(Span::styled(
            format!("◆ YOUR TURN · {yours}"),
            t.fg(t.amber).add_modifier(bold),
        ));
    }
    if running == 0 && yours == 0 {
        left.push(Span::styled("○ IDLE", t.fg(t.faint).add_modifier(bold)));
    }

    let right = match &app.setup.grunt {
        Some(grunt) => {
            let mut spans = vec![
                Span::styled("grunt ", t.fg(t.faint)),
                Span::styled(grunt.model.clone(), t.fg(t.text)),
                Span::styled(format!(" @ {}", grunt.server), t.fg(t.muted)),
            ];
            if let Some(place) = app.setup.place(&grunt.server) {
                spans.push(Span::styled(" · ", t.fg(t.faint)));
                spans.push(place_span(t, place));
            }
            spans.push(Span::styled(
                format!(" · {} ctx ", short_count(u64::from(grunt.context_tokens))),
                t.fg(t.faint),
            ));
            Line::from(spans)
        }
        None => Line::from(Span::styled("no grunt set up: ork init ", t.fg(t.amber))),
    };
    let left = Line::from(left);
    let fits = left.width() + right.width() < usize::from(area.width);
    frame.render_widget(Paragraph::new(left), area);
    if fits {
        frame.render_widget(Paragraph::new(right).alignment(Alignment::Right), area);
    }
}

fn count(app: &App, pick: impl Fn(JobStatus) -> bool) -> usize {
    app.jobs.iter().filter(|job| pick(job.state.status)).count()
}

fn place_span<'a>(t: &Theme, place: Place) -> Span<'a> {
    match place {
        Place::Local => Span::styled("local", t.fg(t.green)),
        Place::Cloud => Span::styled("cloud", t.fg(t.amber)),
    }
}

// --- the jobs ----------------------------------------------------------------------------------

fn status_color(t: &Theme, status: JobStatus) -> Color {
    match status {
        JobStatus::Running => t.green,
        JobStatus::Passed => t.amber,
        JobStatus::FailedChecks | JobStatus::GaveUp | JobStatus::Error => t.red,
        JobStatus::Applied | JobStatus::Discarded => t.faint,
    }
}

fn spinner(app: &App) -> &'static str {
    SPINNER[(app.now.timestamp_millis() / 100).rem_euclid(SPINNER.len() as i64) as usize]
}

fn draw_jobs(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let block = panel(t, kicker(t, format!("JOBS · {}", app.jobs.len())));
    if let Some(error) = &app.load_error {
        let text = Paragraph::new(Span::styled(error.clone(), t.fg(t.red)))
            .wrap(Wrap { trim: true })
            .block(block);
        frame.render_widget(text, area);
        return;
    }
    if app.jobs.is_empty() {
        let text = Text::from(vec![
            Line::from(Span::styled("No jobs yet.", t.fg(t.text))),
            Line::from(Span::styled(
                "ork run starts one; ork guide explains the order.",
                t.fg(t.muted),
            )),
        ]);
        frame.render_widget(Paragraph::new(text).block(block), area);
        return;
    }

    let rows: Vec<Row> = app
        .jobs
        .iter()
        .map(|job| {
            let status = job.state.status;
            let marker = if status == JobStatus::Running {
                Span::styled(spinner(app), t.fg(t.green))
            } else {
                Span::raw(" ")
            };
            let files = match job.report.as_ref().map(|r| r.files) {
                Some(1) => "1 file".to_string(),
                Some(n) => format!("{n} files"),
                None => "-".to_string(),
            };
            let goal = job
                .state
                .goal
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();
            Row::new(vec![
                Cell::from(marker),
                Cell::from(Span::styled(job.state.id.clone(), t.fg(t.text))),
                Cell::from(Span::styled(label(status), t.fg(status_color(t, status)))),
                Cell::from(Span::styled(age(job, app), t.fg(t.muted))),
                Cell::from(Span::styled(files, t.fg(t.muted))),
                Cell::from(Span::styled(crate::text::clean(&goal), t.fg(t.text))),
            ])
        })
        .collect();
    let header = Row::new(["", "ID", "STATUS", "AGE", "FILES", "GOAL"]).style(t.kicker());
    let table = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Length(4),
            Constraint::Length(13),
            Constraint::Length(4),
            Constraint::Length(8),
            Constraint::Min(8),
        ],
    )
    .header(header)
    .column_spacing(1)
    .row_highlight_style(t.selected())
    .highlight_symbol(Span::styled("▌", t.fg(t.amber)))
    .block(block);
    let mut state = TableState::default().with_selected(Some(app.selected));
    frame.render_stateful_widget(table, area, &mut state);
}

fn age(job: &JobView, app: &App) -> String {
    let seconds = (app.now - job.state.created_at).num_seconds().max(0);
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m", seconds / 60),
        3600..86_400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

// --- the models --------------------------------------------------------------------------------

fn draw_models(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let block = panel(t, kicker(t, "MODELS"));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    // The live number: the running job's last request; else the selected job's average.
    let live = app
        .jobs
        .iter()
        .find(|job| job.state.status == JobStatus::Running)
        .and_then(|job| job.stats.last_call.as_ref());
    let (who, speed, note) = match live {
        Some(call) => (
            Some((call.model.clone(), call.server.clone())),
            call.tokens_per_second,
            "live",
        ),
        None => match app.selected_job() {
            Some(job) => (
                job.stats
                    .last_call
                    .as_ref()
                    .map(|c| (c.model.clone(), c.server.clone())),
                job.stats.total.tokens_per_second(),
                "this job's average",
            ),
            None => (None, None, ""),
        },
    };
    let who = who.or_else(|| {
        app.setup
            .grunt
            .as_ref()
            .map(|g| (g.model.clone(), g.server.clone()))
    });

    let mut kicker_line = vec![Span::styled("GRUNT · ", t.kicker())];
    if let Some((model, server)) = &who {
        kicker_line.push(Span::styled(model.clone(), t.fg(t.text)));
        kicker_line.push(Span::styled(format!(" @ {server}"), t.fg(t.muted)));
        if let Some(place) = app.setup.place(server) {
            kicker_line.push(Span::styled(" · ", t.fg(t.faint)));
            kicker_line.push(place_span(t, place));
        }
    }

    let [kicker_area, readout, ledger] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(0),
    ])
    .areas(inner);
    let kicker_line = Line::from(kicker_line);
    let note = Line::from(Span::styled(note, t.fg(t.faint)));
    let fits = kicker_line.width() + note.width() < usize::from(kicker_area.width);
    frame.render_widget(Paragraph::new(kicker_line), kicker_area);
    if fits {
        frame.render_widget(
            Paragraph::new(note).alignment(Alignment::Right),
            kicker_area,
        );
    }

    // The readout: the number in big digits, the LEDs on the middle row, the scale under them.
    //   █ █ ▀▀█
    //   ▀▀█   █ tok/s ▮▮▮▮▮▮▮▮▮▮▮▮▮▮▯▯▯▯▯▯
    //     ▀   ▀       1      10     50  100 300
    let number = speed.map_or("–".to_string(), |s| format!("{s:.0}"));
    let digits = big_digits(&number);
    let digits_width = digits[0].chars().count() as u16;
    let [digits_area, unit_area, meter_area] = Layout::horizontal([
        Constraint::Length(digits_width + 1),
        Constraint::Length(6),
        Constraint::Min(4),
    ])
    .areas(readout);
    let number_style = if live.is_some() {
        t.fg(t.amber).add_modifier(Modifier::BOLD)
    } else {
        t.fg(t.muted).add_modifier(Modifier::BOLD)
    };
    let digits: Vec<Line> = digits
        .into_iter()
        .map(|row| Line::from(Span::styled(row, number_style)))
        .collect();
    frame.render_widget(Paragraph::new(digits), digits_area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::default(),
            Line::from(Span::styled("tok/s", t.fg(t.muted))),
        ]),
        unit_area,
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::default(),
            leds(t, speed, meter_area.width, live.is_some()),
            Line::from(Span::styled(scale(meter_area.width), t.fg(t.faint))),
        ]),
        meter_area,
    );

    draw_ledger(frame, app, ledger);
}

/// A number in block digits three rows high, for the readout; `–` when there's none.
fn big_digits(number: &str) -> [String; 3] {
    let mut rows = [String::new(), String::new(), String::new()];
    for (i, c) in number.chars().enumerate() {
        let glyph = match c {
            '0' => ["█▀█", "█ █", "▀▀▀"],
            '1' => ["▀█ ", " █ ", " ▀ "],
            '2' => ["▀▀█", "█▀▀", "▀▀▀"],
            '3' => ["▀▀█", " ▀█", "▀▀▀"],
            '4' => ["█ █", "▀▀█", "  ▀"],
            '5' => ["█▀▀", "▀▀█", "▀▀▀"],
            '6' => ["█▀▀", "█▀█", "▀▀▀"],
            '7' => ["▀▀█", "  █", "  ▀"],
            '8' => ["█▀█", "█▀█", "▀▀▀"],
            '9' => ["█▀█", "▀▀█", "▀▀▀"],
            _ => ["   ", "▀▀▀", "   "],
        };
        for (row, part) in rows.iter_mut().zip(glyph) {
            if i > 0 {
                row.push(' ');
            }
            row.push_str(part);
        }
    }
    rows
}

/// The LEDs lit for a speed, on the log scale: green, the top quarter amber.
fn leds(t: &Theme, speed: Option<f64>, width: u16, live: bool) -> Line<'static> {
    let n = usize::from(width);
    let lit = speed.map_or(0, |s| {
        let fraction = (s.max(1.0).ln() / METER_MAX.ln()).clamp(0.0, 1.0);
        ((fraction * n as f64).round() as usize).max(1)
    });
    let dim = !live;
    (0..n)
        .map(|i| {
            if i < lit {
                let color = if i * 4 >= n * 3 { t.amber } else { t.green };
                let style = Style::new().fg(color);
                Span::styled(
                    "▮",
                    if dim {
                        style.add_modifier(Modifier::DIM)
                    } else {
                        style
                    },
                )
            } else {
                Span::styled("▮", t.fg(t.line))
            }
        })
        .collect::<Vec<_>>()
        .into()
}

/// The numbers under the meter, each where its LED is.
fn scale(width: u16) -> String {
    let n = usize::from(width);
    let mut out = vec![' '; n];
    for tick in METER_TICKS {
        let text = tick.to_string();
        let at = ((f64::from(tick).ln() / METER_MAX.ln()) * n as f64).round() as usize;
        let start = at.min(n.saturating_sub(text.len()));
        for (i, c) in text.chars().enumerate() {
            if let Some(slot) = out.get_mut(start + i) {
                *slot = c;
            }
        }
    }
    out.into_iter().collect()
}

/// Every model the repo's jobs used: requests, tokens, speed, cost, and where it ran.
fn draw_ledger(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    if area.height == 0 {
        return;
    }
    let mut rows: Vec<Row> = app
        .ledger
        .models
        .iter()
        .map(|(key, stats)| {
            let place = app
                .setup
                .place(&key.server)
                .map_or(Span::styled("?", t.fg(t.faint)), |p| place_span(t, p));
            ledger_row(
                t,
                Span::styled(format!("{} @ {}", key.model, key.server), t.fg(t.text)),
                place,
                stats,
            )
        })
        .collect();
    if app.ledger.models.len() > 1 {
        rows.push(ledger_row(
            t,
            Span::styled("all", t.fg(t.muted).add_modifier(Modifier::BOLD)),
            Span::raw(""),
            &app.ledger.total(),
        ));
    }
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No model requests yet: the ledger fills as jobs run.",
                t.fg(t.faint),
            )),
            area,
        );
        return;
    }
    let header =
        Row::new(["MODEL", "WHERE", "CALLS", "IN", "OUT", "TOK/S", "COST"]).style(t.kicker());
    let table = Table::new(
        rows,
        [
            Constraint::Min(10),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(5),
            Constraint::Length(7),
        ],
    )
    .header(header)
    .column_spacing(1);
    frame.render_widget(table, area);
}

fn ledger_row<'a>(t: &Theme, name: Span<'a>, place: Span<'a>, stats: &ModelStats) -> Row<'a> {
    let num =
        |text: String| Cell::from(Line::from(Span::styled(text, t.fg(t.muted))).right_aligned());
    Row::new(vec![
        Cell::from(name),
        Cell::from(place),
        num(stats.calls.to_string()),
        num(short_count(stats.tokens_in)),
        num(short_count(stats.tokens_out)),
        num(stats
            .tokens_per_second()
            .map_or("–".into(), |s| format!("{s:.0}"))),
        num(stats.cost.map_or("–".into(), money)),
    ])
}

/// $0.0042, $1.23.
pub fn money(dollars: f64) -> String {
    if dollars < 0.01 {
        format!("${dollars:.4}")
    } else {
        format!("${dollars:.2}")
    }
}

/// 222.5 → "00:03:42".
pub fn timecode(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

// --- the selected job --------------------------------------------------------------------------

fn draw_detail(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let Some(job) = app.selected_job() else {
        draw_welcome(frame, app, area);
        return;
    };
    let status = job.state.status;
    let color = status_color(t, status);
    let stats = &job.stats;

    let title = format!(
        "{} · TURN {}/{} · REPAIRS {}/{}",
        label(status).to_uppercase(),
        stats.turns,
        job.max_turns,
        stats.repairs,
        job.max_repairs
    );
    let clock = match (status, job.state.finished_at.or(stats.last_ts)) {
        (JobStatus::Running, _) | (_, Some(_)) => timecode(job.seconds(app.now)),
        // From before events had times: the busy time, added up.
        _ => format!("~{}", timecode(job.seconds(app.now))),
    };
    let block = panel(
        t,
        Span::styled(
            format!(" {title} "),
            t.fg(color).add_modifier(Modifier::BOLD),
        ),
    )
    .title_top(
        Line::from(Span::styled(
            format!(" {clock} "),
            t.fg(t.text).add_modifier(Modifier::BOLD),
        ))
        .right_aligned(),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    let mut head: Vec<Line> = Vec::new();
    head.push(Line::from(vec![
        Span::styled(
            job.state.id.clone(),
            t.fg(t.amber).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            crate::text::clean(job.state.goal.lines().next().unwrap_or_default()),
            t.fg(t.text).add_modifier(Modifier::BOLD),
        ),
    ]));
    head.push(facts(app, job));
    head.push(track(t, stats, inner.width, status == JobStatus::Running));
    let context_row = head.len();
    head.push(Line::default());
    if let Some(reason) = &job.state.reason {
        head.push(Line::from(vec![
            Span::styled("REASON   ", t.kicker()),
            Span::styled(crate::text::clean(reason), t.fg(t.red)),
        ]));
    }
    if let Some(report) = &job.report {
        if !report.summary.is_empty() {
            head.push(Line::from(vec![
                Span::styled("SUMMARY  ", t.kicker()),
                Span::styled(
                    crate::text::clean(report.summary.lines().next().unwrap_or_default()),
                    t.fg(t.muted),
                ),
            ]));
        }
        for question in &report.open_questions {
            head.push(Line::from(vec![
                Span::styled("QUESTION ", t.kicker()),
                Span::styled(crate::text::clean(question), t.fg(t.amber)),
            ]));
        }
    }

    let head_height = (head.len() as u16).min(inner.height);
    let [head_area, events_area] =
        Layout::vertical([Constraint::Length(head_height), Constraint::Min(0)]).areas(inner);
    frame.render_widget(Paragraph::new(head), head_area);
    if (context_row as u16) < head_height {
        let row = Rect {
            y: head_area.y + context_row as u16,
            height: 1,
            ..head_area
        };
        draw_context(frame, app, job, row);
    }
    draw_events(frame, app, job, events_area);
}

/// branch · model · tokens · speed · cost · files.
fn facts<'a>(app: &App, job: &JobView) -> Line<'a> {
    let t = &app.theme;
    let stats = &job.stats;
    let sep = || Span::styled("  ·  ", t.fg(t.faint));
    let mut spans = vec![Span::styled(job.state.branch.clone(), t.fg(t.muted))];
    for key in stats.models.keys() {
        spans.push(sep());
        spans.push(Span::styled(
            format!("{} @ {}", key.model, key.server),
            t.fg(t.text),
        ));
        if let Some(place) = app.setup.place(&key.server) {
            spans.push(Span::styled(" (", t.fg(t.faint)));
            spans.push(place_span(t, place));
            spans.push(Span::styled(")", t.fg(t.faint)));
        }
    }
    spans.push(sep());
    spans.push(Span::styled(
        format!(
            "{} in / {} out",
            short_count(stats.total.tokens_in),
            short_count(stats.total.tokens_out)
        ),
        t.fg(t.muted),
    ));
    if let Some(speed) = stats.total.tokens_per_second() {
        spans.push(sep());
        spans.push(Span::styled(format!("{speed:.0} tok/s"), t.fg(t.muted)));
    }
    if let Some(cost) = stats.total.cost {
        spans.push(sep());
        spans.push(Span::styled(money(cost), t.fg(t.amber)));
    }
    if let Some(report) = &job.report {
        spans.push(sep());
        spans.push(Span::styled(
            match report.files {
                1 => "1 file".to_string(),
                n => format!("{n} files"),
            },
            t.fg(t.muted),
        ));
    }
    Line::from(spans)
}

/// One glyph per step: ▮ a turn (red if a tool failed, · if it only talked), ◆ finish,
/// ✓ ✗ the checks, ↻ a repair, ▯ the turn under way. The tail, when it doesn't fit.
fn track<'a>(t: &Theme, stats: &JobStats, width: u16, running: bool) -> Line<'a> {
    let room = usize::from(width).saturating_sub(10);
    let marks = &stats.track;
    let skip = marks.len().saturating_sub(room);
    let mut spans = vec![Span::styled("TRACK    ", t.kicker())];
    if skip > 0 {
        spans.push(Span::styled("…", t.fg(t.faint)));
    }
    let last = marks.len().saturating_sub(1);
    for (i, mark) in marks.iter().enumerate().skip(skip) {
        let (glyph, color) = match *mark {
            // No tools yet while the job runs: the grunt is on this turn now.
            Mark::Turn { tools: 0, .. } if running && i == last => ("▯", t.green),
            Mark::Turn { tools: 0, .. } => ("·", t.amber),
            Mark::Turn { failed, .. } if failed > 0 => ("▮", t.red),
            Mark::Turn { .. } => ("▮", t.green),
            Mark::Finish => ("◆", t.amber),
            Mark::Check { passed: true } => ("✓", t.green),
            Mark::Check { passed: false } => ("✗", t.red),
            Mark::Repair => ("↻", t.amber),
        };
        spans.push(Span::styled(glyph, t.fg(color)));
    }
    if marks.is_empty() {
        spans.push(Span::styled("nothing yet", t.fg(t.faint)));
    }
    Line::from(spans)
}

/// How the prompt grew, request by request, against the context window.
fn draw_context(frame: &mut Frame, app: &App, job: &JobView, row: Rect) {
    let t = &app.theme;
    let window = app
        .setup
        .grunt
        .as_ref()
        .map(|g| u64::from(g.context_tokens));
    let last = job.stats.context.last().copied().unwrap_or(0);
    let note = match window {
        Some(window) if window > 0 => format!(
            " {} of {} ({}%)",
            short_count(last),
            short_count(window),
            last * 100 / window
        ),
        _ => format!(" {}", short_count(last)),
    };
    // CONTEXT  ▁▂▃▄▅ 41.0k of 64k (64%): one bar per request, the newest last, up to 60.
    const LABEL: u16 = 9;
    let room = row.width.saturating_sub(LABEL + 24).min(60);
    let data = &job.stats.context;
    let shown = &data[data.len().saturating_sub(usize::from(room))..];
    let spark_width = shown.len() as u16;
    let label_area = Rect {
        width: LABEL.min(row.width),
        ..row
    };
    let spark_area = Rect {
        x: row.x + label_area.width,
        width: spark_width,
        ..row
    };
    let note_area = Rect {
        x: spark_area.x + spark_width,
        width: row.width.saturating_sub(label_area.width + spark_width),
        ..row
    };
    frame.render_widget(
        Paragraph::new(Span::styled("CONTEXT", t.kicker())),
        label_area,
    );
    let max = window
        .filter(|w| *w > 0)
        .unwrap_or_else(|| shown.iter().copied().max().unwrap_or(1));
    let spark = Sparkline::default()
        .data(shown.iter().copied())
        .max(max)
        .style(t.fg(t.green));
    frame.render_widget(spark, spark_area);
    frame.render_widget(Paragraph::new(Span::styled(note, t.fg(t.muted))), note_area);
}

/// The events as `ork run` prints them, following the tail (PgUp / PgDn scroll back).
fn draw_events(frame: &mut Frame, app: &App, job: &JobView, area: Rect) {
    let t = &app.theme;
    if area.height == 0 {
        return;
    }
    let height = usize::from(area.height);
    let n = job.lines.len();
    let scroll = app.scroll.min(n.saturating_sub(height));
    let end = n - scroll;
    let start = end.saturating_sub(height);
    let lines: Vec<Line> = job.lines[start..end]
        .iter()
        .map(|line| {
            let trimmed = line.trim_start();
            let color = if trimmed.starts_with('✓') {
                t.green
            } else if trimmed.starts_with('✗')
                || line.contains("   error: ")
                || line.starts_with("error:")
            {
                t.red
            } else if trimmed.starts_with("repair") || trimmed.starts_with("warning") {
                t.amber
            } else if line.starts_with("ork ") {
                t.faint
            } else {
                t.muted
            };
            Line::from(Span::styled(line.clone(), t.fg(color)))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// No jobs: the mark's topknot, and what to do.
fn draw_welcome(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let block = panel(t, kicker(t, "WELCOME"));
    let lines = vec![
        Line::default(),
        Line::from(Span::styled("●", t.fg(t.amber))).centered(),
        Line::from(Span::styled("╱ ╲", t.fg(t.faint))).centered(),
        Line::from(vec![
            Span::styled("●", t.fg(t.green)),
            Span::raw("   "),
            Span::styled("●", t.fg(t.green)),
        ])
        .centered(),
        Line::default(),
        Line::from(Span::styled(
            "frontier plans · local executes",
            t.fg(t.muted),
        ))
        .centered(),
        Line::default(),
        Line::from(Span::styled(
            "Jobs show up here as they run: ork run, from you or your agent.",
            t.fg(t.faint),
        ))
        .centered(),
    ];
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

// --- the footer, the pager, the question -------------------------------------------------------

fn draw_footer(frame: &mut Frame, app: &App, area: Rect, keys: &[(&str, &str)]) {
    let t = &app.theme;
    let mut hints = Vec::new();
    for (key, what) in keys {
        hints.push(Span::styled(format!(" {key}"), t.fg(t.amber)));
        hints.push(Span::styled(format!(" {what} "), t.fg(t.faint)));
    }
    let hints = Line::from(hints);
    let hints_width = hints.width() as u16;
    match &app.message {
        Some(message) => {
            let color = match message.tone {
                Tone::Info => t.text,
                Tone::Good => t.green,
                Tone::Bad => t.red,
            };
            let [left, right] = Layout::horizontal([
                Constraint::Min(0),
                Constraint::Length(hints_width.min(area.width / 2)),
            ])
            .areas(area);
            frame.render_widget(
                Paragraph::new(Span::styled(
                    format!(" {}", crate::text::clean(&message.text)),
                    t.fg(color).add_modifier(Modifier::BOLD),
                )),
                left,
            );
            frame.render_widget(Paragraph::new(hints).alignment(Alignment::Right), right);
        }
        None => frame.render_widget(Paragraph::new(hints), area),
    }
}

fn draw_pager(frame: &mut Frame, app: &App, pager: &Pager, area: Rect) {
    let t = &app.theme;
    let [body, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
    let what = match pager.kind {
        PagerKind::Diff => "DIFF",
        PagerKind::Logs => "TRANSCRIPT",
    };
    let block = panel(
        t,
        Span::styled(
            format!(" {what} · {} ", pager.job),
            t.fg(t.amber).add_modifier(Modifier::BOLD),
        ),
    );
    let inner = block.inner(body);
    let height = usize::from(inner.height);
    app.page_height.set(height);
    let n = pager.lines.len();
    let top = pager.top.min(n.saturating_sub(height));
    let end = (top + height).min(n);
    let block = block.title_top(
        Line::from(Span::styled(
            format!(" {}–{} of {} ", top + 1, end, n),
            t.fg(t.faint),
        ))
        .right_aligned(),
    );
    let lines: Vec<Line> = pager.lines[top..end]
        .iter()
        .map(|line| {
            let line = crate::text::clean(line);
            let color = match pager.kind {
                PagerKind::Diff => diff_color(t, &line),
                PagerKind::Logs => logs_color(t, &line),
            };
            Line::from(Span::styled(line, t.fg(color)))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), body);
    draw_footer(
        frame,
        app,
        footer,
        &[
            ("↑↓", "line"),
            ("PgUp PgDn", "page"),
            ("g G", "top, end"),
            ("q", "back"),
        ],
    );
}

fn diff_color(t: &Theme, line: &str) -> Color {
    if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") {
        t.text
    } else if line.starts_with('+') {
        t.green
    } else if line.starts_with('-') {
        t.red
    } else if line.starts_with("@@") {
        t.amber
    } else {
        t.muted
    }
}

fn logs_color(t: &Theme, line: &str) -> Color {
    let role = line.split_whitespace().nth(1).unwrap_or_default();
    match role {
        "assistant" if line.contains(" → ") => t.amber,
        "assistant" => t.text,
        "tool" => t.muted,
        "meta" => t.faint,
        _ => t.green,
    }
}

fn draw_confirm(frame: &mut Frame, app: &App, question: &str, area: Rect) {
    let t = &app.theme;
    let width = area.width.saturating_sub(4).min(64);
    let lines = vec![
        Line::from(Span::styled(question.to_string(), t.fg(t.text))),
        Line::default(),
        Line::from(vec![
            Span::styled("y", t.fg(t.amber).add_modifier(Modifier::BOLD)),
            Span::styled(" yes    ", t.fg(t.muted)),
            Span::styled("n", t.fg(t.amber).add_modifier(Modifier::BOLD)),
            Span::styled(" no", t.fg(t.muted)),
        ]),
    ];
    let text_width = usize::from(width.saturating_sub(4)).max(1);
    let wrapped = question.chars().count().div_ceil(text_width) as u16;
    let height = (wrapped + 4).min(area.height);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(t.fg(t.amber))
        .title(kicker(t, "YOUR CALL"))
        .style(Style::new().bg(t.raised).fg(t.text));
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(block.padding(Padding::horizontal(1))),
        rect,
    );
}
