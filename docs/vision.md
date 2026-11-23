# Vision

## The problem

The idea started as "vassal" (July 2026): a cloud model gives orders, a local model carries them out.
The real pain behind it: Ivan plans with Claude, then copies the plan files over to his local agent
and explains and hands them over himself. He is the bottleneck in his own pipeline.

Humans stay in the loop for control (approve, apply, stop), but removing the manual handoff is the
point.

## What orkestr8R is

Two things in one tool:

1. **A harness.** orkestr8R's own agent loop (the *grunt*) runs on your local model: file and shell
   tools, limits, context handling, inside a git worktree. Ivan hasn't built a harness before; this
   is where he learns how one works (docs/agent-loop.md).
2. **An orchestrator between models.** It takes orders from a frontier session (the *chief*), runs
   them on the right model, checks the result with real commands and reports back. Later it runs
   many orders at once and picks a model per order (docs/swarm.md).

Who it's for: home lab people with a GPU box and a frontier subscription or API key (Claude,
OpenRouter). Open source.

## Principles

1. **CLI first; agents are users too.** Every feature is an `ork` command with `--json` events and
   plain exit codes before it's anything else. MCP tools and the web UI sit on the same commands.
2. **Verify, don't trust.** orkestr8R runs the checks and reads the exit codes. The model saying
   "done" counts for nothing.
3. **Isolated by default.** Every job gets its own git worktree and branch. Nothing reaches your
   branch until it's applied.
4. **Frontier tokens for judgment, local tokens for volume.** The chief plans, scopes and reviews
   short reports. Grunts read files, write code, run tests and retry.
5. **Plain rules over clever routing.** Which model gets an order, when to retry, when to
   escalate: fixed rules you can read in the config and see in the events.
6. **Everything is observable.** Every model request and tool call is an event: tokens, seconds,
   tokens per second, exit codes. That feeds the CLI, the UI and the token ledger.
7. **Own the loop.** No agent framework. The loop is short, and owning it is the learning goal and
   the source of control.
8. **Minimal config.** One file, sane defaults, and `ork doctor` to say what's wrong.

## Vocabulary

The orc theme stays in docs and the UI. Command and tool names stay plain (`ork run`,
`ork_delegate`), so models aren't confused.

| Word | Means |
|---|---|
| chief | the frontier session that plans and gives orders (Claude Code, OpenCode on a cloud model) |
| grunt | orkestr8R's agent loop on a big local model: the default worker |
| goblin | a small, fast local model, for lookups, summaries and trivial edits (later) |
| mercenary | a paid cloud model (OpenRouter or any API), for harder orders or more parallel width (later) |
| order | one task: goal, context, files it may touch, checks, limits |
| report | what comes back: status, summary, files changed, check results, tokens |
| plan | several orders with dependencies, run together (later, docs/swarm.md) |

## Borrowed from localdub

localdub (github.com/ivanwe2/localdub) already puts AI into a working system on the same GPU box.
The patterns that worked carry over:

| localdub | orkestr8R |
|---|---|
| One CLI (`localdub`) with `--json`: JSON Lines on stdout, last line `result`, people's messages on stderr | `ork … --json`, same shape (docs/cli.md) |
| Exit codes 0 done, 3 waiting for a human, 1 error | the same, plus 4: finished, but the checks failed |
| `localdub guide` and an Agent Skill for agents; a test keeps them in step with the commands | `ork guide` and `skills/orkestr8r/SKILL.md`, the same test |
| The web app is built on the same commands (the Rust server runs the CLI) | `ork serve` and MCP run the same core |
| `call` events per model request: model, tokens, tokens per second | the same, plus `tool` and `check` events, feeding the token ledger |
| Any model on any server per task (`localdub models --set`, `--test`, OpenRouter preset) | `ork models` with roles (grunt, later goblin and mercenary), `--test`, `--preset openrouter` |
| llama-swap on the GPU box: one URL and key, models started on demand, groups decide what runs together | orkestr8R talks to the same llama-swap; its groups set how many grunts can run at once |
| Resumable runs with review stops | jobs that wait for approval (exit code 3) before `apply` |
| End-to-end tests against fake model servers | the loop tested against a fake OpenAI-compatible server that replies from a script |
| ntfy notifications when a run waits or ends | the same for jobs and plans (iteration 2) |
| "Control room" UI: live tracks, LED speed meters, GPU status | the "war room" UI: grunts live, meters, diffs, approve and apply (iteration 2) |

## Roadmap

| Iteration | What |
|---|---|
| 1 | One grunt, one repo, coding tasks. `ork run` / `ask` / `jobs` / `apply`, checks and repairs, `ork mcp`, `ork guide` and the skill. docs/iteration-1.md |
| 2 | `ork serve`: jobs outlive the chief's session; the war room web UI; ntfy notifications; model roles with the OpenRouter preset (mercenaries); grunts run by other harnesses over ACP (e.g. `opencode acp`) |
| 3 | Swarm: plans (orders with dependencies), parallel grunts within each server's capacity, model per order by tier with escalation, independent verifiers, a merge queue. docs/swarm.md |
| 4 | Beyond coding: grunts with MCP tools for other kinds of work; grunts on other machines (Tailscale); maybe orkestr8R as an A2A agent other agents can hire |
