# Decisions

Newest first. Each line: what was decided, and why. Open questions at the bottom.

## 2026-11-27

- **orkestr8R is "a delegation harness"** (Ivan asked whether "harness" fits): it's a harness (the
  grunt's own loop) whose job is taking work handed down by your frontier agent; "harness" alone
  reads as another Claude Code, "orchestrator" alone hides the harness. Reasoning in
  docs/ideas.md (Naming).
- **A repo's allowed commands come from the repo** (#39): its `ork.json`, else its build files
  (a Node repo's test, lint and typecheck scripts with the lockfile's package manager, cargo, go,
  dotnet, pytest), added to the user file's. The user file no longer gets one repo's commands.

## 2026-11-26

- **`ork` alone opens the dashboard; it sets up the agents on its first run** (Ivan: one command,
  pick the agents, keep it open, start the agent in another window). Only when a person is at a
  terminal; scripts and agents still get the help. The TUI gets `/` commands (a command line with
  suggestions, like a chat box) and a bolder look: solid tabs and status pills, a header band, a
  block-letter wordmark.
- **`ork` installs its own skill: `ork skill install AGENT`** (Ivan: copying the folder by hand
  isn't elegant). The text is compiled in rather than downloaded, so it works offline and always
  matches the binary. It goes only to the agents named (Claude Code, OpenCode, Codex, Gemini CLI,
  or `other` for the shared `.agents/skills`), each in its own skills folder, for you or with
  `--repo` for a repo. Codex's docs now name only `~/.agents/skills`, so that's where its copy goes.
- **A TUI first, in v0.1: `ork tui`** (Ivan: "how about a TUI? can we squeeze that in?"). It reads
  the job files, so it needs no daemon and fits v0.1; the web UI (#31) stays with `ork serve` in
  v0.2.
  - ratatui 0.30 (crossterm backend, the default features off). Windows Terminal and any Unix
    terminal; `NO_COLOR` gets a plain theme.
  - Its numbers come from `src/stats.rs` (pure: a job's events in, totals, a ledger per model and
    the turn-by-turn track out), which the web UI will share.
  - Its actions (`diff`, `logs`, `apply`, `discard`) run the CLI's own commands in-process, so the
    rules (a clean checkout, `--force` for failed checks) live in one place. Changes ask y/n first.
  - **Local or cloud** comes from the server's url: loopback, private, link-local and Tailscale
    addresses, and `.local`/`.lan`/`.ts.net`/dotless names, are local; the rest is cloud.
  - The look: localdub's control-room ideas (the status lamp, a big readout beside an LED speed
    meter on a log scale, the timeline track, timecode clocks) in orkestr8R's palette.
- **Events have times; calls have costs** (#32): `ts` is every event's second key, `call` carries
  `cost` when the server reports `usage.cost` (OpenRouter does), and `state.json` has
  `finished_at`. Additive to the `--json` contract.
- **The web UI shows token and model stats for every model, local or cloud** (Ivan; #31): per
  job and per model, tokens in and out, seconds, tokens per second, and the server it ran on. It
  takes its cues from localdub's control-room UI but follows orkestr8R's own theme
  (docs/brand.md), not localdub's look. The `call` events already carry the numbers.
- **Repo settings:** squash-only merges with the PR body as the message, branches deleted on merge,
  Dependabot (cargo, GitHub Actions), secret scanning with push protection, private vulnerability
  reporting. Releases come from `v*` tags (`.github/workflows/release.yml`).

## 2026-11-24

- **Stack: Rust, not .NET/C#** (replaces the 2026-11-23 decision). Ivan: C# is too boring for
  this; Rust is his pick if it fits, Go the alternative. It fits:
  - one static binary `ork`, no runtime to install on the laptop or the GPU box;
  - serde maps the contracts 1:1 (one internally tagged enum is exactly the JSON Lines events);
  - the pieces exist and are first-rate: clap, tokio, reqwest, `globset` (from ripgrep), axum
    for the fake server and later `ork serve`, and the official MCP SDK `rmcp`, which implements
    MCP 2026-07-28 with long-running Tasks;
  - localdub's server is already Rust.
  - The cost: slower to write, and local models write worse Rust than Go or Python, which matters
    when the grunt works on this repo (T16).
  - **No Python.** Its pull is the LLM and agent libraries, which the plan avoids on purpose, and a
    second language breaks the one-binary story. Maybe later for the v0.3 eval analysis.
- **The Rust shape** (details in docs/plan-v0.1.md):
  - one package: a library plus the `ork` binary; tests in one integration-test binary,
    `tests/it/`;
  - `anyhow` for errors; tool failures are results the model reads, not Rust errors;
  - `ModelClient` is a plain struct, not a trait: tests swap the server, not the client;
  - tools are a closed `enum Tool` dispatched with `match`, no trait objects;
  - the process-test helper is a second binary, `ork-testtool`, behind the `testtool` feature, so
    `cargo install` never installs it;
  - `process-wrap` kills whole process trees (process group on Unix, Job Object on Windows);
  - CI runs on pull requests and on pushes to `main`.
- **The command allow-list is a guardrail, not a sandbox** (from a security review of T09). It
  stops accidents (`git push`, `rm -rf`), not a hostile grunt: allowed commands have flags that
  run other programs (`git grep -O<cmd>`, `cargo test --config build.rustc-wrapper=…`), and every
  check runs code the grunt wrote. Deny-listing flags would be whack-a-mole. The real boundary is
  OS isolation (bubblewrap, v0.3); until then: the worktree, the path guard, the scrubbed
  environment, and not running orkestr8R on code you wouldn't run yourself.
- **Nothing from disk or the model is trusted on the way out** (from a security review of T14):
  job ids and job files are checked before they reach a path or a git argument (a cloned repo
  could carry its own `.ork/`), git calls put revisions after `--end-of-options`, and every line
  shown to people is cleaned of control characters (`text::clean`).
- **The path guard judges the real path.** The `.git` and scope checks run on the canonical path
  (the existing part canonicalized, the rest put back on), so a link is judged by where it leads.
- **A repo's `ork.json` can't direct API keys.** A key is only sent to a server whose `url` and
  `key_env` the user's own file (or `ORK_CONFIG`) sets exactly; otherwise it's dropped with a
  warning. A cloned repo could otherwise point a server at its own host, or name any variable
  (`AWS_SECRET_ACCESS_KEY`) as the key, and receive it as a Bearer token. So `ork init --key-env`
  needs `--user`.
  - The repo file may still set `allow_commands`: running a repo's checks (`cargo test`, build
    scripts) already runs that repo's code, so an allow-list from it adds no new power. Use
    orkestr8R on repos whose tests you'd run yourself.
- **Process environment:** the allow-list no longer passes every `NUGET_*` variable (they can carry
  tokens, e.g. `NUGET_AUTH_TOKEN`); it passes named toolchain variables instead (`CARGO_HOME`,
  `RUSTUP_HOME`, `GOPATH`, `NUGET_PACKAGES`, …) plus `DOTNET_*`.
- **License: MIT**, like localdub (`LICENSE`, copyright ivanwe2).
- **Merges into `main` are configurable, in AGENTS.md** (`merge_policy: ivan | frontier | any`).
  It's set to `frontier`:
  - the smarter, frontier agent reviews and merges, local agents' PRs included;
  - local agents open PRs and stop;
  - no agent merges changes to the merge policy, `LICENSE` or CI workflows.
  - The product follows the same idea: grunts never apply their own work; the chief (or Ivan) runs
    `ork apply`. A configurable apply policy (e.g. auto-apply when checks pass) is a later option,
    not v0.1.
- **v0.1 is the CLI with blocking runs** (docs/plan-v0.1.md). Calling `ork` from the shell, with
  the skill, is the default way in. Running in the background and over MCP comes together in v0.2,
  since MCP mostly matters for long calls (MCP Tasks).
- **Model calls use a raw HTTP client + JSON** (now `reqwest` + `serde_json`), not an LLM SDK or
  agent framework.
  - The wire JSON stays visible, which is the learning goal.
  - We get full control over malformed arguments, think blocks, tool calls written into the text,
    and llama.cpp extras.
  - It can sit behind a trait later, if a second kind of client appears.
- **Job state is files in `.ork/jobs/<id>/`, not SQLite.** SQLite only if the v0.2 daemon needs it.
- **Commands run without a shell.** The command line is split into arguments, allowed by token
  prefix, and refused if it has shell metacharacters. Checks pass the same policy.
- **`write_file` only creates new files**; changes to existing files go through `edit_file` (exact
  match, once). Whole-file rewrites are where local models mangle code.
- **`.ork/` goes into `.git/info/exclude`**, so the user's `.gitignore` stays untouched.
- **Tests:**
  - The CLI runs in-process (`cli::run`).
  - A fake OpenAI-compatible server (axum on port 0) replies from scripts.
  - `git grep` is the check command, so tests are fast and the same on Linux and Windows.
- **The ACP and A2A order.** Handing a task over like a tool call is MCP and the CLI (the agent as a
  tool). ACP (other harnesses as grunts) comes in iteration 2, A2A only if orkestr8R goes networked
  (iteration 4). Keep the job model (id, status, events, result, waiting = exit code 3) clean so
  both map onto it.

## 2026-11-23

- **Start with coding tasks.** Ivan: any tasks could work, but coding first. Code has tests, so
  delegated work can be checked automatically. General tasks with MCP tools come in iteration 4.
- **Build our own agent loop.** It's the learning goal (Ivan: "definitely an agent loop … hope to
  learn") and the source of control. No agent framework. The harness study found none of the
  eleven harnesses use one either.
- **CLI first, for people and agents.** Ivan: besides a UI, command-line control is a must, so AI
  agents can drive it. Contract taken from localdub: `--json` JSON Lines, last line `result`, exit
  codes 0/1/3 plus 4. MCP and the UI sit on the same core (docs/cli.md).
- **The swarm is the direction**: several orders at once, a model per order by difficulty. Planned
  as tiers with escalation, plans as data, a scheduler within each server's capacity, verifiers and
  a merge queue (docs/swarm.md, iteration 3). Iteration 1 stays one grunt, but the order format and
  events leave room for it.
- **Take localdub's patterns** (docs/vision.md): models per role on any server, call events, guide
  + skill, fake model servers in tests, llama-swap on the GPU box, notifications, the control-room UI.
- **A web UI in iteration 2**, served by `ork serve`, React + TypeScript (localdub's stack for the
  page).
- **Name: orkestr8R.** Replaces "vassal". Display name `orkestr8R`; package and repo ids lowercase
  `orkestr8r`; command `ork`. A search found no other software with this name; the neighbour to stay
  distinct from is Orkestra (andyyaro/orkestra).
- **Brand: an original orc head.** Its topknot is three nodes: one amber chief over two green grunts
  (docs/brand.md).
- ~~**Stack: .NET 10 / C#** for the core, CLI and MCP (official MCP C# SDK 2.x), shipped as a dotnet
  tool.~~ Replaced on 2026-11-24: Rust.
- **Every job in its own git worktree; checks judged by exit codes; bounded repair turns.**
- **Jobs run in-process in iteration 1** (inside `ork mcp` / `ork run`); the `ork serve` daemon comes
  in iteration 2.

## Open questions

- **Plan format** (iteration 3): YAML or JSON?
- **Models.** Which small model for goblins, and does it fit beside the grunt on 2× 16 GB, or swap
  through llama-swap?
- **The orc names.** Does goblin / grunt / mercenary stay in the config, or only in the UI and docs?
