use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use orkestr8r::config::{self, OrkConfig};
use orkestr8r::env::Env;
use orkestr8r::events::OrkEvent;
use orkestr8r::jobs::JobStore;
use orkestr8r::orders::{FileChange, JobStatus, Order, Report, TokenCount};
use orkestr8r::skill::{Agent, Installed, Target};
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
        duration_s: 0.0,
        tool_calls: 0,
        failed_tool_calls: 0,
        files_read: vec![],
        failing_before: vec![],
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
        "1 WORKING",
        "2 YOUR TURN",
        "grunt qwen-27b @ box · local · 65.5k ctx",
        "JOBS  3",
        "FAILED ",
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
        "RUNNING  TURN 2/30 · REPAIRS 0/2",
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
        "FAILED CHECKS  TURN 1/30",
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
    app.on_key(key(KeyCode::PageUp));
    let scrolled = app.scroll;
    assert!(scrolled > 0, "scrolled back through its events");

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
    assert_eq!(app.scroll, scrolled, "the same job keeps its scroll");

    app.on_key(key(KeyCode::Up));
    assert_eq!(app.scroll, 0, "another job starts at its tail");
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
        &format!("DIFF  {}", jobs.passed),
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
        "frontier plans · local executes",
        "/skill",
        "Start your agent in another terminal",
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

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.on_key(key(KeyCode::Char(c)));
    }
}

fn targets(installed: &[Agent], found: &[Agent]) -> Vec<Target> {
    Agent::ALL
        .into_iter()
        .map(|agent| Target {
            agent,
            path: PathBuf::from(format!(
                "/home/me/.{}/skills/orkestr8r/SKILL.md",
                agent.id()
            )),
            found: found.contains(&agent),
            installed: if installed.contains(&agent) {
                Installed::Current
            } else {
                Installed::No
            },
        })
        .collect()
}

#[test]
fn slash_opens_the_command_line_with_suggestions() {
    let jobs = jobs();
    let mut app = app(&jobs);

    app.on_key(key(KeyCode::Char('/')));
    assert!(app.palette.is_some());
    let s = screen(&app, 150, 40);
    for text in ["COMMANDS", "/apply", "/skill", "/doctor", "Tab complete"] {
        assert!(s.contains(text), "{text:?} not on the screen:\n{s}");
    }

    // `/di` narrows it down; Enter runs the highlighted one on the selected job.
    typed(&mut app, "di");
    let names: Vec<&str> = app
        .palette
        .as_ref()
        .unwrap()
        .suggestions()
        .iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, ["discard", "diff"]);
    app.on_key(key(KeyCode::Down));
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        Some(Action::Diff {
            job: jobs.running.clone()
        })
    );
    assert!(app.palette.is_none(), "it closes when it runs");

    // A job named by its id, or the start of it: selected, then asked about.
    app.on_key(key(KeyCode::Char('/')));
    typed(&mut app, &format!("apply {}", &jobs.passed[..3]));
    assert_eq!(app.on_key(key(KeyCode::Enter)), None);
    assert_eq!(app.selected_job().unwrap().state.id, jobs.passed);
    assert!(app.confirm.as_ref().unwrap().question.contains("passed"));
    app.on_key(key(KeyCode::Esc));

    // Unknown things say so.
    app.on_key(key(KeyCode::Char('/')));
    typed(&mut app, "nope");
    assert_eq!(app.on_key(key(KeyCode::Enter)), None);
    assert_eq!(app.message.as_ref().unwrap().tone, Tone::Bad);
    app.on_key(key(KeyCode::Char('/')));
    typed(&mut app, "diff zzzz");
    app.on_key(key(KeyCode::Enter));
    assert!(app.message.as_ref().unwrap().text.contains("no job zzzz"));

    // Tab completes; Backspace on an empty line and Esc close it.
    app.on_key(key(KeyCode::Char('/')));
    typed(&mut app, "sq");
    app.on_key(key(KeyCode::Tab));
    assert_eq!(app.palette.as_ref().unwrap().input, "squash ");
    app.on_key(key(KeyCode::Esc));
    assert!(app.palette.is_none());
    app.on_key(key(KeyCode::Char('/')));
    app.on_key(key(KeyCode::Backspace));
    assert!(app.palette.is_none());
    // While it's open, letters are text, not keys: q doesn't quit.
    app.on_key(key(KeyCode::Char('/')));
    assert_eq!(app.on_key(key(KeyCode::Char('q'))), None);
}

#[test]
fn slash_commands_run_the_clis_and_the_tuis_own() {
    let jobs = jobs();
    let mut app = app(&jobs);
    let run = |app: &mut App, line: &str| {
        app.on_key(key(KeyCode::Char('/')));
        typed(app, line);
        app.on_key(key(KeyCode::Enter))
    };

    assert_eq!(run(&mut app, "doctor"), Some(Action::Doctor));
    assert_eq!(run(&mut app, "models"), Some(Action::Models));
    assert_eq!(run(&mut app, "guide"), Some(Action::Guide));
    assert_eq!(run(&mut app, "quit"), Some(Action::Quit));
    assert_eq!(run(&mut app, "theme light"), None);
    assert_eq!(app.theme.name, ThemeName::Light);
    assert_eq!(run(&mut app, "theme"), None);
    assert_eq!(app.theme.name, ThemeName::Dark);

    assert_eq!(run(&mut app, "help"), None);
    let Screen::Pager(pager) = &app.screen else {
        panic!("no help: {:?}", app.screen);
    };
    assert_eq!(pager.kind, PagerKind::Help);
    assert!(pager.lines.iter().any(|l| l.contains("/skill")));
    app.on_key(key(KeyCode::Char('q')));
    app.on_key(key(KeyCode::Char('?')));
    assert!(matches!(&app.screen, Screen::Pager(p) if p.kind == PagerKind::Help));
    app.on_key(key(KeyCode::Char('q')));

    // Doctor's output is the point, even when a check failed (exit 1).
    app.done(
        &Action::Doctor,
        Outcome {
            code: 1,
            stdout: "✓ config        ork.json\n✗ server        connection refused\n".into(),
            stderr: "error: 1 of 2 checks failed\n".into(),
        },
    );
    let Screen::Pager(pager) = &app.screen else {
        panic!("no pager: {:?}", app.screen);
    };
    assert_eq!(pager.kind, PagerKind::Doctor);
    assert_eq!(pager.lines.last().unwrap(), "error: 1 of 2 checks failed");
    let s = screen(&app, 100, 20);
    assert!(
        s.contains("DOCTOR") && s.contains("connection refused"),
        "{s}"
    );

    assert_eq!(Action::Doctor.args().unwrap(), ["doctor"]);
    assert_eq!(
        Action::InstallSkill {
            agents: vec![Agent::Claude, Agent::OpenCode]
        }
        .args()
        .unwrap(),
        ["skill", "install", "claude", "opencode"]
    );
}

#[test]
fn the_picker_asks_which_agents_and_installs_for_those() {
    let repo = TempRepo::new();
    let mut app = App::new(JobStore::new(&repo.path), Setup::default(), ThemeName::Dark);
    app.refresh(Utc::now());
    app.skills = targets(&[], &[Agent::Claude, Agent::Codex]);
    app.open_picker();

    let s = screen(&app, 120, 32);
    for text in [
        "SET UP YOUR AGENTS",
        "Which agents do you use?",
        "Claude Code",
        "OpenCode",
        "Any other agent",
        "on this machine",
        "Space pick",
    ] {
        assert!(s.contains(text), "{text:?} not on the screen:\n{s}");
    }

    // The ones found start ticked; Space changes that; keys go to the picker, not the jobs.
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.chosen(), [Agent::Claude, Agent::Codex]);
    assert_eq!(app.on_key(key(KeyCode::Down)), None);
    app.on_key(key(KeyCode::Char(' '))); // OpenCode on
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Char(' '))); // Codex off
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        Some(Action::InstallSkill {
            agents: vec![Agent::Claude, Agent::OpenCode]
        })
    );
    assert!(app.picker.is_none());

    app.done(
        &Action::InstallSkill {
            agents: vec![Agent::Claude, Agent::OpenCode],
        },
        Outcome::default(),
    );
    let message = app.message.clone().unwrap();
    assert_eq!(message.tone, Tone::Good);
    assert!(
        message.text.contains("Claude Code, OpenCode"),
        "{message:?}"
    );

    // Esc skips, and says where it is.
    app.open_picker();
    assert_eq!(app.on_key(key(KeyCode::Esc)), None);
    assert!(app.picker.is_none());
    assert!(app.message.as_ref().unwrap().text.contains("/skill"));

    // Nothing ticked: Enter takes the one under the cursor.
    app.skills = targets(&[], &[]);
    app.open_picker();
    app.on_key(key(KeyCode::Down));
    assert_eq!(
        app.on_key(key(KeyCode::Enter)),
        Some(Action::InstallSkill {
            agents: vec![Agent::OpenCode]
        })
    );

    // With the skill in place, the welcome says so.
    app.skills = targets(&[Agent::Claude], &[Agent::Claude]);
    let s = screen(&app, 120, 32);
    assert!(s.contains("The skill is installed for Claude Code."), "{s}");
}

#[test]
fn the_command_line_and_the_picker_fit_small_terminals() {
    let jobs = jobs();
    let mut app = app(&jobs);
    app.skills = targets(&[], &[Agent::Claude]);
    app.on_key(key(KeyCode::Char('/')));
    for (width, height) in [(80, 24), (40, 10), (10, 3), (1, 1)] {
        screen(&app, width, height);
    }
    app.on_key(key(KeyCode::Esc));
    app.open_picker();
    for (width, height) in [(80, 24), (40, 10), (10, 3), (1, 1)] {
        screen(&app, width, height);
    }
}

#[tokio::test]
async fn ork_alone_prints_the_help_when_no_one_is_at_a_terminal() {
    let out = run_cli(&[], &std::env::temp_dir(), Env::default()).await;
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("Usage: ork"), "{}", out.stderr);
    assert!(out.stderr.contains("for the dashboard"), "{}", out.stderr);

    let json = run_cli(&["--json"], &std::env::temp_dir(), Env::default()).await;
    assert_eq!(json.code, 1);
    let result = json_lines(&json.stdout).pop().unwrap();
    assert!(result["error"].as_str().unwrap().contains("Usage: ork"));
}

/// Not a test: renders some screens as HTML, colours and all, to look at the design.
/// `ORK_SHOTS=dir cargo test --all-features --test it -- --ignored tui::shots`
#[test]
#[ignore]
fn shots() {
    let Ok(dir) = std::env::var("ORK_SHOTS") else {
        return;
    };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let jobs = jobs();

    let mut main = app(&jobs);
    main.repo = "calc".into();
    main.skills = targets(&[Agent::Claude], &[Agent::Claude]);
    shot(&dir, "1-dark", &main, 150, 40);
    main.on_key(key(KeyCode::Char('t')));
    shot(&dir, "2-light", &main, 150, 40);
    main.on_key(key(KeyCode::Char('t')));
    main.on_key(key(KeyCode::Char('/')));
    shot(&dir, "3-commands", &main, 150, 40);
    main.on_key(key(KeyCode::Esc));
    main.on_key(key(KeyCode::Down));
    shot(&dir, "4-failed-80x24", &main, 80, 24);

    // The README's screenshot.
    let (_demo_repo, demo_store, demo_now) = demo();
    let mut readme = App::new(demo_store, demo_setup(), ThemeName::Dark);
    readme.refresh(demo_now);
    readme.repo = "my-app".into();
    readme.skills = targets(&[Agent::Claude], &[Agent::Claude]);
    shot(&dir, "0-readme", &readme, 140, 30);

    let repo = TempRepo::new();
    let mut empty = App::new(JobStore::new(&repo.path), setup(), ThemeName::Dark);
    empty.refresh(Utc::now());
    empty.repo = "calc".into();
    empty.skills = targets(&[], &[Agent::Claude, Agent::OpenCode]);
    empty.open_picker();
    shot(&dir, "5-first-run", &empty, 120, 32);
    empty.picker = None;
    shot(&dir, "6-welcome", &empty, 120, 32);
}

fn demo_setup() -> Setup {
    let config: OrkConfig = serde_json::from_value(json!({
        "servers": {
            "box": { "url": "http://gpu-box:9876/v1" },
            "router": { "url": "https://openrouter.ai/api/v1" }
        },
        "roles": { "grunt": { "server": "box", "model": "qwen3-coder", "context_tokens": 65536 } }
    }))
    .unwrap();
    Setup::from_config(&config)
}

/// A repo for the README's screenshot: a run under way with a repair behind it, and jobs in the
/// other states, on a local and a cloud model.
fn demo() -> (TempRepo, JobStore, DateTime<Utc>) {
    let repo = TempRepo::new();
    let store = JobStore::new(&repo.path);
    let write = |id: &str, events: &[OrkEvent]| {
        let lines: String = events.iter().map(|e| e.to_line() + "\n").collect();
        std::fs::write(store.paths(id).events, lines).unwrap();
    };
    let order = |goal: &str| Order {
        goal: goal.into(),
        max_turns: Some(40),
        ..Default::default()
    };
    let call =
        |turn: u32, model: &str, server: &str, tokens_in: u64, out: u64, tps: f64| OrkEvent::Call {
            turn,
            model: model.into(),
            server: server.into(),
            tokens_in,
            tokens_out: out,
            seconds: out as f64 / tps + 1.2,
            tokens_per_second: Some(tps),
            cost: (server == "router").then_some(tokens_in as f64 * 0.000_000_3),
        };
    let local = |turn: u32, tokens_in: u64, out: u64, tps: f64| {
        call(turn, "qwen3-coder", "box", tokens_in, out, tps)
    };
    let tool = |turn: u32, name: &str, path: &str, ok: bool| OrkEvent::Tool {
        turn,
        name: name.into(),
        args: Some(json!({ "path": path })),
        ok,
        ms: 3,
        error: (!ok).then(|| "old_string not found in the file".into()),
    };
    let check = |cmd: &str, exit: i32, seconds: f64| OrkEvent::Check {
        cmd: cmd.into(),
        exit,
        seconds,
        tail: String::new(),
    };
    let pause = || std::thread::sleep(std::time::Duration::from_millis(5));

    let applied = store
        .create(&order("Tests for parse_duration in src/time.rs"), "abc123")
        .unwrap()
        .id;
    write(
        &applied,
        &[
            local(1, 5800, 96, 48.0),
            tool(1, "read_file", "src/time.rs", true),
            local(2, 6900, 612, 45.0),
            tool(2, "write_file", "tests/time.rs", true),
            OrkEvent::Finish {
                turn: 3,
                summary: "Added 6 tests".into(),
            },
            check("npm run test:int", 0, 8.1),
        ],
    );
    store.set_status(&applied, JobStatus::Passed, None).unwrap();
    store
        .set_status(&applied, JobStatus::Applied, None)
        .unwrap();
    pause();

    let failed = store
        .create(
            &order("Port the CSV export to the streaming writer"),
            "abc123",
        )
        .unwrap()
        .id;
    write(
        &failed,
        &[
            call(1, "deepseek-v4", "router", 9100, 840, 92.0),
            tool(1, "read_file", "src/export/csv.ts", true),
            call(2, "deepseek-v4", "router", 12400, 1210, 88.0),
            tool(2, "edit_file", "src/export/csv.ts", false),
            check("npm run test:int", 1, 11.4),
        ],
    );
    store
        .set_status(
            &failed,
            JobStatus::FailedChecks,
            Some("checks failed".into()),
        )
        .unwrap();
    pause();

    let passed = store
        .create(&order("Pagination for GET /api/orders"), "abc123")
        .unwrap()
        .id;
    write(
        &passed,
        &[
            local(1, 7200, 140, 47.0),
            tool(1, "read_file", "src/orders/route.ts", true),
            local(2, 9800, 920, 46.0),
            tool(2, "edit_file", "src/orders/route.ts", true),
            OrkEvent::Finish {
                turn: 3,
                summary: "Added page and limit".into(),
            },
            check("npm run test:int", 0, 9.0),
        ],
    );
    store.set_status(&passed, JobStatus::Passed, None).unwrap();
    store
        .write_report(&passed, &report(&passed, JobStatus::Passed, 3))
        .unwrap();
    pause();

    let running_state = store
        .create(&order("Rate limiting for the login endpoint"), "abc123")
        .unwrap();
    let running = running_state.id.clone();
    let mut events = vec![
        local(1, 6100, 120, 51.0),
        tool(1, "read_file", "src/auth/login.ts", true),
        local(2, 9400, 210, 49.0),
        tool(2, "grep", "src/", true),
        local(3, 12800, 880, 47.0),
        tool(3, "write_file", "src/auth/limiter.ts", true),
        local(4, 16100, 640, 46.0),
        tool(4, "edit_file", "src/auth/login.ts", false),
        local(5, 17300, 420, 46.0),
        tool(5, "edit_file", "src/auth/login.ts", true),
        local(6, 19900, 96, 45.0),
        OrkEvent::Finish {
            turn: 6,
            summary: "Added a token bucket per IP".into(),
        },
        check("npm run test:int", 1, 10.2),
        OrkEvent::Repair { attempt: 1 },
        local(7, 23600, 310, 44.0),
        tool(7, "read_file", "tests/auth/login.test.ts", true),
        local(8, 26200, 540, 44.0),
        tool(8, "edit_file", "src/auth/limiter.ts", true),
    ];
    events.push(local(9, 28400, 180, 45.0));
    write(&running, &events);

    let now = running_state.created_at + Duration::seconds(252);
    (repo, store, now)
}

fn shot(dir: &std::path::Path, name: &str, app: &App, width: u16, height: u16) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    let hex = |color: Color, default: &str| match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => default.to_string(),
    };
    let mut html = String::from(
        "<!doctype html><meta charset=utf-8><style>body{margin:0;background:#12160f}\
         pre{margin:0;padding:14px 16px;font:15px/1.13 'Cascadia Mono','Consolas',monospace;}\
         span{white-space:pre}</style><pre>",
    );
    for y in 0..height {
        for x in 0..width {
            let cell = &buffer[(x, y)];
            let symbol = match cell.symbol() {
                "&" => "&amp;",
                "<" => "&lt;",
                ">" => "&gt;",
                s => s,
            };
            let bold = cell.modifier.contains(ratatui::style::Modifier::BOLD);
            let reversed = cell.modifier.contains(ratatui::style::Modifier::REVERSED);
            let (mut fg, mut bg) = (hex(cell.fg, "#f3ebd2"), hex(cell.bg, "#12160f"));
            if reversed {
                std::mem::swap(&mut fg, &mut bg);
            }
            html.push_str(&format!(
                "<span style=\"color:{fg};background:{bg};{}\">{symbol}</span>",
                if bold { "font-weight:bold" } else { "" }
            ));
        }
        html.push('\n');
    }
    html.push_str("</pre>");
    std::fs::write(dir.join(format!("{name}.html")), html).unwrap();
}

#[test]
fn a_read_only_job_shows_as_answered_and_waits_on_no_one() {
    let repo = TempRepo::new();
    let store = JobStore::new(&repo.path);
    let order = Order {
        goal: "Which functions are untested?".into(),
        mode: orkestr8r::orders::OrderMode::Readonly,
        ..Default::default()
    };
    let id = store.create(&order, "abc123").unwrap().id;
    store.set_status(&id, JobStatus::Passed, None).unwrap();
    let mut app = App::new(store, setup(), ThemeName::Dark);
    app.refresh(Utc::now());

    assert!(app.selected_job().unwrap().readonly);
    let s = screen(&app, 150, 40);
    assert!(s.contains("ANSWERED"), "{s}");
    assert!(!s.contains("YOUR TURN"), "nothing to review:\n{s}");
    assert_eq!(app.on_key(key(KeyCode::Char('a'))), None);
    assert!(app.message.as_ref().unwrap().text.contains("read-only"));
    assert_eq!(app.on_key(key(KeyCode::Char('d'))), None);
    assert!(app.confirm.is_none());
}
