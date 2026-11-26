# Ideas

Everything proposed and not built yet, so nothing gets lost: Ivan's ideas, feedback from test
sessions, and what came up while building. Each has where it came from, an honest assessment (not
just the suggestion) and where it stands. Actionable ones have an issue; when one is decided, the
decision goes to docs/decisions.md; when it's planned, to a plan file.

**Adding one:** a line under its source, with the date. Rejected ideas stay, with the reason, so
they don't come back without it.

## From Ivan

### 2026-11-27

| Idea | Assessment | Where |
|---|---|---|
| **A ranking of which model does which tasks at what level**, the "self-improving" part | Yes. The data mostly exists: every job records model, turns, repairs, checks, time, tokens, cost, and whether the chief applied or discarded the work. A scoreboard first, routing suggestions next, automatic routing last. It's the harness learning which model to trust with what (the models don't change). Needs volume (show n; nothing under ~10 jobs per cell), a task kind the chief sets on the order, and at least two models. Escalation stays a fixed rule (docs/swarm.md). | #50, v0.2 |
| **Several endpoints for tasks** (OpenRouter, the local box, another Qwen), so the orchestrator delegates to many | Half there: the config holds several servers and roles; v0.1 always uses `grunt`. Needs a role or tier per order (the tiers in docs/swarm.md: goblin, grunt, mercenary) and background jobs to run them at once. | #49, v0.2 |
| **Parallel execution** | Worth it, but one GPU splits its speed across jobs unless the server has parallel slots, so it's about scheduling: a queue with per-server limits, stacked orders, overlap warnings. | #51, after #27 |
| **Is "harness" the right word?** ("harness delegator"?) | Decided: "a delegation harness"; see [Naming](#naming) below. | done: README, vision, repo description |

### 2026-11-26

| Idea | Where |
|---|---|
| A TUI before the web UI | shipped: `ork tui` (#36) |
| Token and model stats from wherever the model runs, local or cloud; localdub-inspired, own theme | shipped in the TUI (#36); the web UI is #31 |
| `ork` should install the skill itself, for the agents you pick (Claude Code, OpenCode, …), not a manual copy | shipped: `ork skill install` (#37), the picker (#38) |
| Run `ork`, pick the agents, keep it open as the dashboard while the agent runs in another window | shipped: `ork` alone opens the dashboard; first run asks (#38) |
| More flair in the TUI, a definitive style; slash commands | shipped (#38) |
| Docker for builds, not local toolchains | how it's built: `scripts/dcargo.sh`, `scripts/build-windows.sh` |

## From the first outside test (2026-11-27)

A fresh Claude Code session, given only the skill, used `ork` in a Next.js + Payload repo
(kc-commerce) and reported back. Its verdict: easy to use; the friction is where `ork` meets the
environment, not in the order, check, report loop. It hadn't run a write order yet.

| Its suggestion | Assessment | Where |
|---|---|---|
| Start from the uncommitted working tree; apply onto a dirty tree | The top one. Its plan had two holes: `git stash create` skips untracked files (the new code you want tests for), so snapshot with a temporary index; and apply must lay on the job's own diff, not merge a branch built on the snapshot. | #47, v0.2 |
| The allow-list as a one-liner for the human (`ork allow`); doctor reading `package.json` | Mostly fixed: a repo's commands now come from its build files (#39). The refusal already names the command and lists what's allowed. `ork allow CMD --repo` maybe later: editing `ork.json` is meant to be the human's control. | #39 (done) |
| Test the checks before spending model time | Running every check first doubles check time on every job. Instead: resolve the check programs up front (cheap), and on failure re-run the checks on the base commit to tell "already failing" from "the grunt broke it"; `--baseline` opt-in. | done (#45): the program check and the base-commit re-run; `--baseline` not built until needed |
| Worktree setup: link `node_modules` and `.env`, run setup commands | Real (it works today only because worktrees sit inside the repo). Linking `.env` hands secrets to the model, so the guard must refuse secret files first. | #48, v0.2 |
| `ork wait JOB… [--any] [--timeout]` | Fine; part of v0.2's background jobs. Claude Code's background shells already notify. | #27 |
| A queue with per-server concurrency; stacked orders (`--base JOB`); overlap warnings | Agreed, after background jobs. | #51 |
| Wall time and tool-call counts in the report; `files_read` for read-only | Yes, cheap. | done (#40) |
| Tell the grunt to describe the code, not its comments; cite line numbers | Yes, a prompt change. | #44, v0.1.x |
| A short file tree up front | It exists, but only two levels deep, which is why it guessed `Pages.ts`. Fill the 200-entry budget instead. | #43, v0.1.x |
| Clean up read-only jobs (or `ork gc`) | Yes: they show as YOUR TURN with nothing to apply. | done (#41): removed when they end, ANSWERED in the TUI |
| The order on stdin (`ork run -`) | Yes: PowerShell quoting. | #42, v0.1.x |
| `--events FILE` | Not needed: every job's events are already in `.ork/jobs/<id>/events.jsonl`, written line by line. `start` comes after the worktree exists by design. The guide should say so. | rejected |
| `ask` and `run` consistent: `--path` relative to the same place | Yes, both relative to where you are. (`data.answer` vs `report.summary` can stay: different commands.) | #46, v0.1.x |

## Came up while building

| Idea | Assessment | Where |
|---|---|---|
| Set up the server from the dashboard (an `ork init` screen in the TUI: url, model, key variable, then a doctor run) | Fits "setup inside the tool" like the skill picker. After the release. | not filed |
| `/run` in the TUI: write and start an order from the dashboard | Needs runs that don't block the TUI and survive quitting it (#27, #21). The chief starting orders is the main path. | not filed |
| `ork allow CMD [--repo]` | Only if editing `ork.json` turns out to be a real hurdle. | not filed |

Tracked elsewhere: the web UI (#31), v0.2 MCP and background jobs (#27), a dead run stays
"running" (#21), a thinking switch per role (#33), a token budget per order (#24), shrinking big
tool arguments (#23), sandboxing (#20), Rust checks on Windows without MSVC (#19).

## Naming

Ivan asked whether "harness" is the right word, or something like "harness delegator".

docs/vision.md already says orkestr8R is two things: **a harness** (the grunt's own agent loop:
tools, guards, context, checks, in a worktree) and **an orchestrator between models** (the chief
hands it orders; it picks the model, checks the result, reports back).

- "Harness" alone is accurate for half of it, but people read it as "another Claude Code", which
  it isn't: it doesn't replace your agent, it works for it.
- "Orchestrator" alone hides the harness, which is where most of the work and the reliability are.
- "Harness delegator" reads awkwardly and doesn't say who delegates to whom.
- **Suggestion: "a delegation harness"**: a harness whose job is taking work handed down by your
  frontier agent. One line: *"orkestr8R is a delegation harness: your frontier agent hands it
  checkable coding work, and it runs that work on your own models, in worktrees, checked by real
  commands."* The tagline stays: frontier plans · local executes.

**Decided 2026-11-27** (Ivan: "update the readme and description"): README, vision and the repo
description say "a delegation harness".
