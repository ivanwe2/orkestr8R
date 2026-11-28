# orkestr8R: notes for coding agents

Claude Code reads this through `CLAUDE.md`; OpenCode and other agents read it directly.

## Working with Ivan (read this first)

- Ivan wants to **learn how agent loops and harnesses work** while building this. When you build the
  grunt loop (docs/agent-loop.md) or anything around it, explain each piece as it goes in: what it
  does, why it's there, and the Rust idiom used. Briefly, as the work goes, no lectures up front.
- He values directness and precise, concise answers. Pragmatic, minimal configuration. Don't build
  things that aren't needed yet.
- When he has to run something, say exactly which command and on which machine (laptop or the GPU
  box), one step at a time, and wait for the output.
- The GPU box runs llama-swap for his other project (localdub) and is sometimes used for gaming:
  ask before running anything there.

## What orkestr8R is

The frontier "chief" (Claude Code, OpenCode) writes orders; orkestr8R runs them with a local "grunt"
agent loop in git worktrees, checks them with real commands and reports back. See README.md and
docs/vision.md.

## Status and what to work on

**v0.1.0 is released (docs/plan-v0.1.md). v0.2 is being built: the work is in docs/plan-v0.2.md,
tasks T01–T16, each specified to be done without asking.**
- Take the first unticked task in its Progress list whose dependencies are ticked.
- One task per branch and PR; tick it in the same PR.
- If the spec is wrong or unclear, fix the spec in the same PR and say so.

Background: docs/iteration-1.md (milestones M0–M5). Decisions and open questions:
docs/decisions.md. Keep both up to date when something is decided. Ideas not built yet, from Ivan,
test sessions or the work: docs/ideas.md. When one comes up, add it there (and an issue if it's
actionable) in the same session, so it isn't lost.

## Merge policy (configurable)

```
merge_policy: frontier
```

Ivan sets the value above. It decides who merges a PR into `main`:

| Value | Who merges |
|---|---|
| `ivan` | only Ivan. Agents open the PR and stop |
| `frontier` | a **frontier agent**, after CI is green and after reviewing the diff. That includes PRs opened by local agents. Local agents open the PR and stop |
| `any` | any agent, its own PR included, once CI is green |

- **Who you are.** A frontier agent runs on a frontier model (Claude, GPT, Gemini class), e.g.
  Claude Code. A local agent runs on a local model, e.g. OpenCode pointed at llama-swap. **If
  you're not sure which you are, you're local.**
- **Before merging, check:**
  - CI is green on Linux and Windows;
  - you've read the whole diff against the task's spec in docs/plan-v0.1.md;
  - the Progress tick is in the PR.
  - Then `gh pr merge --squash --delete-branch`, with the PR title as a conventional commit.
- **Never merge a PR that changes this section, `LICENSE` or `.github/workflows/`**, whatever the
  policy says. Those are Ivan's.

## Planned layout and conventions

| Path | What |
|---|---|
| `src/` | one Rust package: the library (the grunt loop, tools, orders and reports, jobs, worktrees, checks, the model client) and the `ork` binary |
| `src/cli/` | `ork`: every command, `--json` events; `ork mcp` (the MCP server over stdio) from v0.2 |
| `src/tui/` | `ork tui`: the live view for people (ratatui); its numbers come from `src/stats.rs`, which the web UI will share |
| `tests/it/` | one integration-test binary: unit-level tests, and end-to-end runs of the loop against a fake model server; `ork-testtool` (feature `testtool`) for process tests |
| `skills/orkestr8r/SKILL.md` | the Agent Skill that points agents to `ork guide`; compiled into `ork`, which installs it (`ork skill install`) |
| `scripts/` | `dcargo.sh` (cargo in Docker), `build-windows.sh` (a native `ork.exe` without MSVC) |
| `.github/` | CI, the release workflow (on `v*` tags), Dependabot, issue and PR templates |
| `docs/` | the plan and the contracts |

- Rust, edition 2024, the toolchain pinned in `rust-toolchain.toml`. `cargo fmt --check` and
  `cargo clippy --all-targets --all-features -- -D warnings` are clean; run the tests with
  `cargo test --all-features`.
  - **Ivan's laptop (Windows) has no MSVC build tools, so Rust can't link there natively.** Run
    cargo in Docker instead: `scripts/dcargo.sh test --all-features` (same arguments as `cargo`).
    CI covers native Windows.
- **The CLI contract is a contract** (docs/cli.md): with `--json`, stdout carries JSON Lines and
  the last line is always `result`; messages for people go to stderr. Exit codes: 0 done, 1 error,
  3 waiting for a human, 4 finished but checks failed. Change the doc in the same PR as the code; a
  test keeps `ork guide`, the skill and docs/cli.md in step with the commands.
- MCP tools are thin adapters over the same core as the CLI commands: no logic of their own.
- Tests never need a GPU: the loop runs against a fake OpenAI-compatible server that replies from a
  script (tool calls, malformed calls, think blocks). Keep them green; add a test with each fix.
- One branch per meaningful change (`feat/…`, `fix/…`, `docs/…`), conventional commits
  (`feat:`, `fix:`, `docs:`…), PRs into `main`.
- Public repo: never commit keys, `.env` files, or transcripts of private code.
