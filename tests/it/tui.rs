use chrono::{DateTime, Duration, Utc};
use orkestr8r::config::{self, OrkConfig};
use orkestr8r::env::Env;
use orkestr8r::events::OrkEvent;
use orkestr8r::jobs::JobStore;
use orkestr8r::orders::{FileChange, JobStatus, Order, Report, TokenCount};
use orkestr8r::tui::Ork;
use orkestr8r::tui::app::{Action, App, Outcome, PagerKind, Screen, Setup, Tone};
use orkestr8r::tui::theme::ThemeName;
use orkestr8r::tui::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::Color;
use serde_json::json;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use crate::jobs::passing_job;
use crate::support::cli::run_cli;
use crate::support::temp_repo::TempRepo;
use crate::support::workspace::json_lines;

fn setup() -> Setup {
    let config: OrkConfig = serde_json::from_value(json!({
        "servers": {
            "box": { "url": "http://192.168.0.10:9876/v1" },
            "router": { "url": "https://openrouter.ai/api/v1" }
        },
        "roles": { "grunt": { "server": "box", "model": "qwen-27b", "context_tokens": 65536 } }
    }))
    .unwrap();
    Setup::from_config(&config)
}

fn call(turn: u32, model: &str, server: &str, cost: Option<f64>) -> OrkEvent {
    OrkEvent::Call {
        turn,
        model: model.into(),
        server: server.into(),
        tokens_in: 6000 * u64::from(turn),
        tokens_out: 200,
        seconds: 4.0,
        tokens_per_second: Some(47.0),
        cost,
    }
}

fn tool(turn: u32, ok: bool) -> OrkEvent {
    OrkEvent::Tool {
        turn,
        name: "read_file".into(),
        args: Some(json!({ "path": "src/text.rs" })),
        ok,
        ms: 3,
        error: (!ok).then(|| "no such file".into()),
    }
}

fn check(exit: i32) -> OrkEvent {
    OrkEvent::Check {
        cmd: "cargo test".into(),
        exit,
        seconds: 12.0,
        tail: String::new(),
    }
}

fn report(job: &str, status: JobStatus, files: usize) -> Report {
    Report {
        job: job.into(),
        status,
        summary: "Added three tests".into(),
        files_changed: (0..files)
            .map(|i| FileChange {
                path: format!("tests/{i}.rs"),
                added: 10,
                removed: 0,
            })
            .collect(),
        checks: vec![],
        repairs_used: 0,
        turns: 2,
        tokens: TokenCount {
            input: 1,
            output: 1,
        },
        model: "qwen-27b".into(),
        branch: format!("ork/{job}"),
        open_questions: vec![],
        reason: None,
    }
}

/// Three jobs, oldest first: one passed, one with failed checks (on a cloud model), one running.
struct Jobs {
    _repo: TempRepo,
    store: JobStore,
    passed: String,
    failed: String,
    running: String,
    /// 3 minutes 42 seconds after the running job started.
    now: DateTime<Utc>,
}

fn jobs() -> Jobs {
    let repo = TempRepo::new();
    let store = JobStore::new(&repo.path);
    let write = |id: &str, events: &[OrkEvent]| {
        let lines: String = events.iter().map(|e| e.to_line() + "\n").collect();
        std::fs::write(store.paths(id).events, lines).unwrap();
    };
    let order = |goal: &str| Order {
        goal: goal.into(),
        max_turns: Some(30),
        ..Default::default()
    };

    let passed = store
        .create(&order("Tests for text.rs"), "abc123")
        .unwrap()
        .id;
    write(
        &passed,
        &[
            call(1, "qwen-27b", "box", None),
            tool(1, true),
            OrkEvent::Finish {
                turn: 1,
                summary: "Added three tests".into(),
            },
            check(0),
        ],
    );
    store.set_status(&passed, JobStatus::Passed, None).unwrap();
    store
        .write_report(&passed, &report(&passed, JobStatus::Passed, 2))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));

    let failed = store
        .create(&order("Refactor the grep tool"), "abc123")
        .unwrap()
        .id;
    write(
        &failed,
        &[
            call(1, "big", "router", Some(0.0021)),
            tool(1, false),
            check(101),
        ],
    );
    store
        .set_status(
            &failed,
            JobStatus::FailedChecks,
            Some("checks failed".into()),
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));

    let running_state = store.create(&order("Add Sum to calc"), "abc123").unwrap();
    let running = running_state.id.clone();
    write(
        &running,
        &[
            call(1, "qwen-27b", "box", None),
            tool(1, true),
            call(2, "qwen-27b", "box", None),
        ],
    );

    Jobs {
        _repo: repo,
        store,
        passed,
        failed,
        running,
        now: running_state.created_at + Duration::seconds(222),
    }
}

fn app(jobs: &Jobs) -> App {
    let mut app = App::new(jobs.store.clone(), setup(), ThemeName::Dark);
    app.refresh(jobs.now);
    app
}

fn screen(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..height {
        for x in 0..width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn the_screen_shows_the_jobs_the_models_and_the_running_job() {
    let jobs = jobs();
    let app = app(&jobs);

    assert_eq!(app.jobs.len(), 3);
    assert_eq!(
        app.selected_job().unwrap().state.id,
        jobs.running,
        "the running job is picked first"
    );
    let s = screen(&app, 150, 40);
    for text in [
        "orkestr8R",
        "WORKING · 1",
        "YOUR TURN · 2",
        "grunt qwen-27b @ box · local · 65.5k ctx",
        "JOBS · 3",
        "failed checks",
        "2 files",
        // the speed readout, live from the running job: 47 in block digits, then the unit
        "█ █ ▀▀█",
        "▀▀█   █ tok/s",
        "live",
        // the ledger: both models, where they run, the cloud one's cost, and the sum
        "qwen-27b @ box",
        "big @ router",
        "cloud",
        "$0.0021",
        "all",
        // the running job
        "RUNNING · TURN 2/30 · REPAIRS 0/2",
        "00:03:42",
        "TRACK",
        "CONTEXT",
        "12.0k of 65.5k (18%)",
        "t1  read_file src/text.rs",
    ] {
        assert!(s.contains(text), "{text:?} not on the screen:\n{s}");
    }
    assert!(s.contains(&jobs.running) && s.contains(&jobs.passed));
}

#[test]
fn a_finished_job_shows_its_track_summary_and_reason() {
    let jobs = jobs();
    let mut app = app(&jobs);
    app.on_key(key(KeyCode::Down));
    assert_eq!(app.selected_job().unwrap().state.id, jobs.failed);

    let s = screen(&app, 150, 40);
    for text in [
        "FAILED CHECKS · TURN 1/30",
        "▮✗", // a turn whose tool failed, then a failed check
        "REASON",
        "checks failed",
        "✗ cargo test  exit 101",
    ] {
        assert!(s.contains(text), "{text:?} not on the screen:\n{s}");
    }

    app.on_key(key(KeyCode::Down));
    let s = screen(&app, 150, 40);
    assert!(s.contains("▮◆✓"), "turn, finish, check passed:\n{s}");
    assert!(
        s.contains("SUMMARY") && s.contains("Added three tests"),
        "{s}"
    );
}

#[test]
fn keys_pick_a_job_and_ask_before_changing_anything() {
    let jobs = jobs();
    let mut app = app(&jobs);

    // the running job: nothing to apply or discard yet
    assert_eq!(app.on_key(key(KeyCode::Char('a'))), None);
    assert!(app.message.as_ref().unwrap().text.contains("still running"));
    assert_eq!(app.on_key(key(KeyCode::Char('x'))), None);
    assert!(app.confirm.is_none());

    // failed checks: applying means --force, and it asks
    app.on_key(key(KeyCode::Down));
    assert_eq!(app.on_key(key(KeyCode::Char('a'))), None);
    let confirm = app.confirm.clone().expect("a question first");
    assert!(
        confirm.question.contains("failed checks"),
        "{}",
        confirm.question
    );
    assert_eq!(
        confirm.action,
        Action::Apply {
            job: jobs.failed.clone(),
            squash: false,
            force: true
        }
    );
    assert_eq!(app.on_key(key(KeyCode::Char('n'))), None, "no means no");
    assert!(app.confirm.is_none());

    // passed: squash, yes
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Char('s')));
    assert!(app.confirm.as_ref().unwrap().question.contains("Squash"));
    assert_eq!(
        app.on_key(key(KeyCode::Char('y'))),
        Some(Action::Apply {
            job: jobs.passed.clone(),
            squash: true,
            force: false
        })
    );

    // discard asks too; other keys leave the question open
    app.on_key(key(KeyCode::Char('x')));
    assert_eq!(app.on_key(key(KeyCode::Char('j'))), None);
    assert!(app.confirm.is_some(), "still asking");
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        Some(Action::Discard {
            job: jobs.passed.clone()
        })
    );

    assert_eq!(
        app.on_key(key(KeyCode::Char('d'))),
        Some(Action::Diff {
            job: jobs.passed.clone()
        })
    );
    assert_eq!(
        app.on_key(key(KeyCode::Char('l'))),
        Some(Action::Logs {
            job: jobs.passed.clone()
        })
    );
    assert_eq!(
        app.on_key(key(KeyCode::Down)),
        None,
        "the last job stays selected"
    );
    assert_eq!(app.selected_job().unwrap().state.id, jobs.passed);

    // a key's release (Windows sends them) does nothing
    let mut release = key(KeyCode::Char('q'));
    release.kind = KeyEventKind::Release;
    assert_eq!(app.on_key(release), None);
    assert_eq!(app.on_key(key(KeyCode::Char('q'))), Some(Action::Quit));
    assert_eq!(
        app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        Some(Action::Quit)
    );
}

#[test]
fn the_selection_follows_its_job_as_new_ones_arrive() {
    let jobs = jobs();
    let mut app = app(&jobs);
    app.on_key(key(KeyCode::Down));
    assert_eq!(app.selected_job().unwrap().state.id, jobs.failed);

    jobs.store
        .create(
            &Order {
                goal: "A new one".into(),
                ..Default::default()
            },
            "abc123",
        )
        .unwrap();
    app.refresh(jobs.now);

    assert_eq!(app.jobs.len(), 4);
    assert_eq!(app.selected_job().unwrap().state.id, jobs.failed);
}

#[test]
fn actions_are_the_clis_own_commands() {
    let apply = Action::Apply {
        job: "k7f2".into(),
        squash: true,
        force: true,
    };
    assert_eq!(
        apply.args().unwrap(),
        ["apply", "k7f2", "--squash", "--force"]
    );
    assert_eq!(
        Action::Diff { job: "k7f2".into() }.args().unwrap(),
        ["diff", "k7f2"]
    );
    assert_eq!(
        Action::Discard { job: "k7f2".into() }.args().unwrap(),
        ["discard", "k7f2"]
    );
    assert_eq!(Action::Quit.args(), None);
}

#[test]
fn a_diff_opens_in_the_pager_and_errors_go_to_the_footer() {
    let jobs = jobs();
    let mut app = app(&jobs);

    app.done(
        &Action::Diff {
            job: jobs.passed.clone(),
        },
        Outcome {
            code: 0,
            stdout: "diff --git a/x b/x\n@@ -1 +1 @@\n-old\n+new\n".into(),
            stderr: String::new(),
        },
    );
    let Screen::Pager(pager) = &app.screen else {
        panic!("not the pager: {:?}", app.screen);
    };
    assert_eq!((pager.kind, pager.lines.len()), (PagerKind::Diff, 4));
    let s = screen(&app, 100, 20);
    for text in [
        &format!("DIFF · {}", jobs.passed),
        "+new",
        "-old",
        "1–4 of 4",
    ] {
        assert!(s.contains(text), "{text:?} not on the screen:\n{s}");
    }
    app.on_key(key(KeyCode::Char('q')));
    assert_eq!(app.screen, Screen::Jobs);

    app.done(
        &Action::Apply {
            job: jobs.passed.clone(),
            squash: false,
            force: false,
        },
        Outcome {
            code: 1,
            stdout: String::new(),
            stderr: "error: your checkout has uncommitted changes; commit or stash them first\n"
                .into(),
        },
    );
    let message = app.message.clone().unwrap();
    assert_eq!(message.tone, Tone::Bad);
    assert_eq!(
        message.text,
        "your checkout has uncommitted changes; commit or stash them first"
    );
    assert!(screen(&app, 150, 40).contains("your checkout has uncommitted changes"));
}

#[test]
fn an_empty_repo_gets_a_welcome() {
    let repo = TempRepo::new();
    let mut app = App::new(JobStore::new(&repo.path), Setup::default(), ThemeName::Dark);
    app.refresh(Utc::now());

    let s = screen(&app, 120, 30);
    for text in [
        "IDLE",
        "No jobs yet",
        "frontier plans · local executes",
        "no grunt set up: ork init",
    ] {
        assert!(s.contains(text), "{text:?} not on the screen:\n{s}");
    }
}

#[test]
fn small_terminals_and_the_question_box_draw_without_panicking() {
    let jobs = jobs();
    let mut app = app(&jobs);
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Char('x')));
    assert!(app.confirm.is_some());
    for (width, height) in [(80, 24), (60, 12), (20, 5), (1, 1)] {
        screen(&app, width, height);
    }
    let s = screen(&app, 80, 24);
    assert!(s.contains("YOUR CALL") && s.contains("Discard"), "{s}");
}

#[test]
fn themes_toggle_and_no_color_stays_plain() {
    let jobs = jobs();
    let mut app = app(&jobs);
    assert_eq!(app.theme.name, ThemeName::Dark);
    app.on_key(key(KeyCode::Char('t')));
    assert_eq!(app.theme.name, ThemeName::Light);

    let mut plain = App::new(jobs.store.clone(), setup(), ThemeName::Plain);
    plain.on_key(key(KeyCode::Char('t')));
    assert_eq!(plain.theme.name, ThemeName::Plain);
    assert_eq!(plain.theme.green, Color::Reset);
}

#[tokio::test]
async fn ork_tui_is_for_people() {
    let repo = TempRepo::new();
    let out = run_cli(&["tui", "--json"], &repo.path, Env::default()).await;
    assert_eq!(out.code, 1);
    let result = json_lines(&out.stdout).pop().unwrap();
    assert!(
        result["error"]
            .as_str()
            .unwrap()
            .contains("ork jobs --json"),
        "{result}"
    );
}

/// The whole way: a real job, its numbers on the screen, then its diff and apply through the
/// runner, exactly as the keys would.
#[tokio::test(flavor = "multi_thread")]
async fn a_real_job_shows_up_and_applies_through_the_runner() {
    let (ws, _server, job) = passing_job().await;
    let config = config::load(&ws.repo, &ws.env).unwrap().config;
    let mut app = App::new(
        JobStore::new(&ws.repo),
        Setup::from_config(&config),
        ThemeName::Dark,
    );
    app.refresh(Utc::now());
    let view = app.selected_job().unwrap();
    assert_eq!(view.state.status, JobStatus::Passed);
    assert_eq!(view.stats.total.calls, 2);
    let s = screen(&app, 150, 40);
    assert!(s.contains("fake-model @ box (local)"), "{s}");

    let ork = Ork {
        handle: Handle::current(),
        cwd: ws.repo.clone(),
        env: ws.env.clone(),
        cancel: CancellationToken::new(),
    };
    let diff = Action::Diff { job: job.clone() };
    let apply = Action::Apply {
        job: job.clone(),
        squash: false,
        force: false,
    };
    let (args_diff, args_apply) = (diff.args().unwrap(), apply.args().unwrap());
    let (diffed, applied, missing) = tokio::task::spawn_blocking(move || {
        (
            ork.run(args_diff),
            ork.run(args_apply),
            ork.run(vec!["diff".into(), "zzzz".into()]),
        )
    })
    .await
    .unwrap();

    assert_eq!(diffed.code, 0, "{diffed:?}");
    assert!(diffed.stdout.contains("+pub fn Sum"), "{diffed:?}");
    app.done(&diff, diffed);
    assert!(matches!(app.screen, Screen::Pager(_)));

    assert_eq!(applied.code, 0, "{applied:?}");
    app.done(&apply, applied);
    let message = app.message.clone().unwrap();
    assert_eq!(message.tone, Tone::Good);
    assert!(message.text.contains("merged into"), "{message:?}");
    app.refresh(Utc::now());
    assert_eq!(app.selected_job().unwrap().state.status, JobStatus::Applied);

    assert_eq!(missing.code, 1);
    assert!(missing.stderr.contains("error:"), "{missing:?}");
}
