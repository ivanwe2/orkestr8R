# orkestr8R: notes for coding agents

Claude Code reads this through `CLAUDE.md`; OpenCode and other agents read it directly.

## Working with Ivan (read this first)

- Ivan wants to **learn how agent loops and harnesses work** while building this. When you build the
  grunt loop (docs/agent-loop.md) or anything around it, explain each piece as it goes in: what it
  does, why it's there, and the C# idiom used. Briefly, as the work goes, no lectures up front.
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

## Status

Design stage, no code yet. The plan lives in `docs/`; the first build is docs/iteration-1.md
(milestones M0–M5). Decisions and open questions: docs/decisions.md. Keep both up to date when
something is decided.

## Planned layout and conventions

| Path | What |
|---|---|
| `src/Orkestr8r.Core/` | the grunt loop, tools, orders and reports, jobs, worktrees, checks, model servers |
| `src/Orkestr8r.Cli/` | `ork`: every command, `--json` events, `ork mcp` (the MCP server over stdio) |
| `tests/` | unit tests, and end-to-end runs of the loop against a fake model server |
| `skills/orkestr8r/SKILL.md` | the Agent Skill that points agents to `ork guide` |
| `docs/` | the plan and the contracts |

- .NET 10, C#, nullable on, warnings as errors.
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
