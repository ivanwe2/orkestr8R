use chrono::{DateTime, Duration, Utc};
use orkestr8r::events::OrkEvent;
use orkestr8r::stats::{JobStats, Ledger, Mark, ModelKey, Place};

fn at(seconds: i64) -> DateTime<Utc> {
    "2026-11-26T16:00:00Z".parse::<DateTime<Utc>>().unwrap() + Duration::seconds(seconds)
}

fn call(turn: u32, model: &str, out: u64, tps: Option<f64>, cost: Option<f64>) -> OrkEvent {
    OrkEvent::Call {
        turn,
        model: model.into(),
        server: if cost.is_some() { "router" } else { "box" }.into(),
        tokens_in: 1000 * u64::from(turn),
        tokens_out: out,
        seconds: 4.0,
        tokens_per_second: tps,
        cost,
    }
}

fn tool(turn: u32, ok: bool) -> OrkEvent {
    OrkEvent::Tool {
        turn,
        name: "read_file".into(),
        args: None,
        ok,
        ms: 500,
        error: (!ok).then(|| "no such file".into()),
    }
}

fn check(exit: i32) -> OrkEvent {
    OrkEvent::Check {
        cmd: "cargo test".into(),
        exit,
        seconds: 10.0,
        tail: String::new(),
    }
}

#[test]
fn places_come_from_the_server_url() {
    for url in [
        "http://127.0.0.1:8080/v1",
        "http://localhost:9876/v1",
        "http://192.168.0.10:9876/v1",
        "http://10.1.2.3/v1",
        "http://172.20.0.1/v1",
        "http://100.89.99.13:9876/v1", // Tailscale
        "http://box:9876/v1",
        "http://gpu.local/v1",
        "http://gpu.lan:8080/v1",
        "https://box.tail1234.ts.net/v1",
        "http://[::1]:8080/v1",
        "http://[fd12::1]/v1",
    ] {
        assert_eq!(Place::of_url(url), Some(Place::Local), "{url}");
    }
    for url in [
        "https://openrouter.ai/api/v1",
        "https://api.openai.com/v1",
        "http://172.32.0.1/v1",  // just past 172.16/12
        "http://100.128.0.1/v1", // just past 100.64/10
        "https://8.8.8.8/v1",
        "http://[2001:db8::1]/v1",
    ] {
        assert_eq!(Place::of_url(url), Some(Place::Cloud), "{url}");
    }
    assert_eq!(Place::of_url("not a url"), None);
}

#[test]
fn a_log_adds_up_to_the_job_stats() {
    let events = [
        (
            at(0),
            OrkEvent::Start {
                command: "run".into(),
                job: Some("k7f2".into()),
                branch: None,
                worktree: None,
                role: None,
                model: None,
                server: None,
            },
        ),
        // 100 tokens at 50 t/s: 2 s writing
        (at(4), call(1, "qwen", 100, Some(50.0), None)),
        (at(5), tool(1, true)),
        (at(5), tool(1, false)),
        // a text-only reply: no tools
        (at(9), call(2, "qwen", 20, Some(20.0), None)),
        // 300 tokens at 100 t/s: 3 s writing
        (at(13), call(3, "qwen", 300, Some(100.0), None)),
        (
            at(13),
            OrkEvent::Finish {
                turn: 3,
                summary: "done".into(),
            },
        ),
        (at(23), check(1)),
        (at(23), OrkEvent::Repair { attempt: 1 }),
        // no speed figure from a cloud server: the wall time counts
        (at(27), call(4, "big", 80, None, Some(0.0021))),
        (at(27), tool(4, true)),
        (at(37), check(0)),
        (
            at(38),
            OrkEvent::Result {
                status: "passed".into(),
                exit_code: 0,
                job: Some("k7f2".into()),
                report: None,
                data: None,
                error: None,
            },
        ),
    ];
    let mut log: String = events
        .iter()
        .map(|(ts, event)| event.to_line_at(*ts) + "\n")
        .collect();
    log += "{\"event\":\"tool\",\"tu"; // the job was killed mid-line

    let stats = JobStats::from_log(&log);

    assert_eq!(stats.turns, 4);
    assert_eq!((stats.tools, stats.tool_errors), (3, 1));
    assert_eq!((stats.checks_passed, stats.checks_failed), (1, 1));
    assert_eq!(stats.repairs, 1);
    assert_eq!(stats.result.as_deref(), Some("passed"));
    assert_eq!((stats.first_ts, stats.last_ts), (Some(at(0)), Some(at(38))));
    assert_eq!(stats.context, [1000, 2000, 3000, 4000]);
    assert_eq!(
        stats.track,
        [
            Mark::Turn {
                turn: 1,
                tools: 2,
                failed: 1
            },
            Mark::Turn {
                turn: 2,
                tools: 0,
                failed: 0
            },
            Mark::Turn {
                turn: 3,
                tools: 1,
                failed: 0
            },
            Mark::Finish,
            Mark::Check { passed: false },
            Mark::Repair,
            Mark::Turn {
                turn: 4,
                tools: 1,
                failed: 0
            },
            Mark::Check { passed: true },
        ]
    );

    let qwen = &stats.models[&ModelKey {
        model: "qwen".into(),
        server: "box".into(),
    }];
    assert_eq!((qwen.calls, qwen.tokens_out), (3, 420));
    assert_eq!(qwen.cost, None, "local: no cost");
    // 420 tokens over 2 + 1 + 3 s of writing: long replies weigh more than a short one
    assert_eq!(qwen.tokens_per_second(), Some(70.0));

    let big = &stats.models[&ModelKey {
        model: "big".into(),
        server: "router".into(),
    }];
    assert_eq!(big.tokens_per_second(), Some(20.0), "80 tokens in 4 s");
    assert_eq!(big.cost, Some(0.0021));

    assert_eq!(stats.total.calls, 4);
    assert_eq!(stats.total.tokens_in, 10_000);
    assert_eq!(stats.total.cost, Some(0.0021));
    assert_eq!(stats.busy_seconds(), 16.0 + 1.5 + 20.0);
    let last = stats.last_call.unwrap();
    assert_eq!((last.model.as_str(), last.ts), ("big", Some(at(27))));
}

#[test]
fn the_ledger_adds_up_jobs_by_model() {
    let one = JobStats::from_log(&(call(1, "qwen", 100, Some(50.0), None).to_line() + "\n"));
    let two = JobStats::from_log(
        &[
            call(1, "qwen", 300, Some(100.0), None).to_line(),
            call(2, "big", 10, None, Some(0.5)).to_line(),
        ]
        .join("\n"),
    );

    let mut ledger = Ledger::default();
    ledger.add(&one);
    ledger.add(&two);

    assert_eq!(ledger.models.len(), 2);
    let qwen = &ledger.models[&ModelKey {
        model: "qwen".into(),
        server: "box".into(),
    }];
    assert_eq!((qwen.calls, qwen.tokens_out), (2, 400));
    assert_eq!(qwen.tokens_per_second(), Some(80.0));
    let total = ledger.total();
    assert_eq!((total.calls, total.cost), (3, Some(0.5)));
}

#[test]
fn logs_from_before_times_still_add_up() {
    let log = "{\"event\":\"call\",\"turn\":1,\"model\":\"qwen\",\"server\":\"box\",\"tokens_in\":5,\"tokens_out\":5,\"seconds\":1.0}\n";
    let stats = JobStats::from_log(log);
    assert_eq!(stats.total.calls, 1);
    assert_eq!(stats.first_ts, None);
    assert_eq!(stats.total.tokens_per_second(), Some(5.0));
}
