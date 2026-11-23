# The `ork` CLI and MCP contract

Draft, 2026-11-23. This is a contract: agents, scripts, the MCP server and the web UI all depend on
it. Change it in the same PR as the code.

## Commands (iteration 1)

```
ork init                                   config, delegation rules for AGENTS.md / CLAUDE.md, MCP registration snippets
ork doctor                                 is everything in place? (server, tool calling, git, allow-listed commands)
ork models                                 which model does each role, on which server
          [--set ROLE=SERVER/MODEL] [--test ROLE]
ork models --server NAME --url URL [--key-stdin] [--remove]
                                           add or change a server (any OpenAI-compatible one)
ork ask "QUESTION" --path FILE [--path …]  a one-shot, read-only answer from the local model
ork run ORDER.json                         run one order: worktree, grunt loop, checks, report
ork run --goal "…" --scope GLOB --check "CMD" [--path FILE] [--max-turns N] [--detach]
                                           the same with the order on the command line
ork jobs                                   every job, newest first
ork status JOB                             one job: status, and the report when done
ork watch JOB                              follow a job's events live
ork diff JOB [--stat]                      the job's changes
ork apply JOB [--squash]                   merge the job's branch into the current branch
ork discard JOB                            drop the worktree and the branch
ork logs JOB                               the transcript: every message to and from the model
ork mcp                                    the MCP server over stdio
ork guide                                  the guide for AI agents (this file is the one for people)
```

`ork run` blocks until the job ends. With `--detach` it prints the job id and returns; `watch`,
`status` and `apply` take it from there.

## Exit codes

| Code | Means |
|---|---|
| 0 | done (for `run`: checks passed) |
| 1 | error (bad order, server down, git trouble) |
| 3 | waiting for a human (e.g. an order marked `approve: true` waits before `apply`) |
| 4 | finished, but the checks failed or the grunt gave up; the report says which |

## `--json`

Every command takes `--json`. Messages for people go to stderr. stdout carries one JSON object per
line, and the last line is always `result`.

```
{"event": "start", "command": "run", "job": "k7f2", "branch": "ork/k7f2", "worktree": ".ork/worktrees/k7f2", "role": "grunt", "model": "qwen-27b"}
{"event": "call", "turn": 1, "model": "qwen-27b", "server": "box", "tokens_in": 6120, "tokens_out": 212, "seconds": 4.1, "tokens_per_second": 51.7}
{"event": "tool", "turn": 1, "name": "read_file", "args": {"path": "src/Orders/OrderQuery.cs"}, "ok": true, "ms": 3}
{"event": "tool", "turn": 6, "name": "edit_file", "args": {"path": "src/Orders/OrderQuery.cs"}, "ok": true, "ms": 2}
{"event": "finish", "turn": 9, "summary": "Added page and pageSize …"}
{"event": "check", "cmd": "dotnet test tests/Orders", "exit": 1, "seconds": 14.2, "tail": "Failed: 1, Passed: 17"}
{"event": "repair", "attempt": 1}
{"event": "check", "cmd": "dotnet test tests/Orders", "exit": 0, "seconds": 13.8, "tail": "Passed: 18"}
{"event": "result", "status": "passed", "exit_code": 0, "job": "k7f2", "report": {…}}
```

| Event | When |
|---|---|
| `start` | the command started; for `run`: job, branch, worktree, model |
| `call` | every model request: model, server, tokens in/out, seconds, tokens per second |
| `tool` | every tool call: name, short args, ok, ms (errors carry `error`) |
| `finish` | the grunt called `finish` |
| `check` | every check: command, exit code, seconds, last lines |
| `repair` | a repair turn starts |
| `warning` | e.g. a malformed tool call, a truncated output, a think block stripped |
| `result` | always last: `status` (`passed`, `failed_checks`, `gave_up`, `waiting`, `error`), `exit_code`, and the command's result (`report` for `run`/`status`) |

`call` events add up to the token ledger: what each job cost on which model.

## The order

```json
{
  "goal": "Add pagination to GET /api/orders",
  "context": "Constraints and conventions from the chief, in plain words",
  "paths": ["src/Orders/OrdersController.cs", "src/Orders/OrderQuery.cs"],
  "write_scope": ["src/Orders/**", "tests/Orders/**"],
  "checks": ["dotnet build", "dotnet test tests/Orders"],
  "mode": "write",
  "max_turns": 40,
  "max_repairs": 2,
  "approve": false
}
```

- **Required:** `goal`, `checks` (at least one, unless `mode` is `readonly`).
- **`mode: readonly`** removes every writing tool. Use it for investigations ("find every place
  that…"): the answer comes back in the summary.
- **Planned for iteration 3** (docs/swarm.md): `id`, `needs`, `tier`.

## The report

Aim for under ~400 tokens: this is what the chief reads instead of the work.

```json
{
  "job": "k7f2",
  "status": "passed",
  "summary": "≤ 5 lines from the grunt",
  "files_changed": [{ "path": "src/Orders/OrderQuery.cs", "added": 42, "removed": 3 }],
  "checks": [{ "cmd": "dotnet test tests/Orders", "exit": 0, "tail": "Passed: 18" }],
  "repairs_used": 1,
  "turns": 11,
  "tokens": { "in": 61234, "out": 4810 },
  "model": "qwen-27b",
  "branch": "ork/k7f2",
  "open_questions": []
}
```

## MCP tools

Thin adapters over the same core. Six tools, because every tool description costs the chief
context on every turn.

| Tool | Same as | Returns |
|---|---|---|
| `ork_delegate(order)` | `ork run --detach` | the job id; an MCP Task where the client supports Tasks |
| `ork_wait(job, timeout_s ≤ 50)` | `ork status` while waiting | status, plus the report when done |
| `ork_result(job, include_diff?, max_diff_lines?)` | `ork status` + `ork diff` | the report, optionally the diff |
| `ork_apply(job, squash?)` | `ork apply` | the merge result |
| `ork_discard(job)` | `ork discard` | ok |
| `ork_ask(question, paths[])` | `ork ask` | a short answer |

## For agents: the skill

`skills/orkestr8r/SKILL.md` says when to delegate (the rules in docs/iteration-1.md) and to run
`ork guide` for the commands. Same shape as localdub's skill. Install it in Claude Code with
`cp -r skills/orkestr8r ~/.claude/skills/`. A test checks that `ork guide`, the skill and this file
name the same commands.
