<p align="center">
  <img src="assets/logo/mark.svg" width="120" alt="">
</p>

<h1 align="center">orkestr8R</h1>

<p align="center">
  <b>Frontier plans. Local executes.</b><br>
  Claude Code or OpenCode hands real coding work to your own models. The work is run, checked and
  reported back, so you stop copy-pasting plans between them.
</p>

> [!NOTE]
> **Status: building v0.1 (November 2026).** No release yet. The plan is
> [docs/vision.md](docs/vision.md) → [docs/iteration-1.md](docs/iteration-1.md) →
> [docs/plan-v0.1.md](docs/plan-v0.1.md) (the task-by-task build plan).

## The idea

You have a frontier model (Claude Code, OpenCode on a cloud model) and a local one (llama.cpp behind
llama-swap on your own GPUs). Today you are the middleman: you copy the plan, explain it and hand it
over. orkestr8R takes that job:

- the **chief** (your frontier session) writes an *order*: a goal, the files the work may touch,
  and the commands that prove it works;
- a **grunt** (orkestr8R's own agent loop on your local model) does the work in its own git
  worktree;
- orkestr8R runs the checks itself, gives the grunt a couple of tries to fix what fails, and hands the
  chief a short report: what changed, what passed, what's open. Nothing reaches your branch until
  it is applied.

The chief spends its tokens on judgment. Your GPUs do the volume.

## How it plugs in

- **CLI first.** `ork` is one command-line tool with `--json` events and plain exit codes, so a
  person, a script or an AI agent can drive it. `ork guide` and an Agent Skill teach agents to use it.
- **MCP.** `ork mcp` serves the same commands as tools, for clients that prefer them.
- **Web UI** later (`ork serve`), built on the same commands: jobs live, the models' speed, the diffs,
  approve and apply.

## Docs

| | |
|---|---|
| [docs/vision.md](docs/vision.md) | why, principles, vocabulary, what's borrowed from localdub, roadmap |
| [docs/iteration-1.md](docs/iteration-1.md) | the first build: scope, architecture, milestones M0–M5 |
| [docs/plan-v0.1.md](docs/plan-v0.1.md) | v0.1 task by task (T01–T16): contracts, files, acceptance commands, for people and agents |
| [docs/cli.md](docs/cli.md) | the `ork` CLI and MCP contract: commands, orders, reports, events, exit codes |
| [docs/agent-loop.md](docs/agent-loop.md) | what an agent loop is, one turn on the wire, how the grunt's loop works, a learning path |
| [docs/swarm.md](docs/swarm.md) | where it goes next: tiers, plans, parallel grunts, verifiers, merge queue |
| [docs/landscape.md](docs/landscape.md) | what already exists, what's current (Nov 2026), and what we take from it |
| [docs/decisions.md](docs/decisions.md) | decisions so far, open questions |
| [docs/brand.md](docs/brand.md) | logo, colours, type |

## Planned stack

.NET 10 / C# for the core, the CLI and MCP (official MCP C# SDK); React + TypeScript for the web UI
later. Models: any OpenAI-compatible server (llama-server, llama-swap, Ollama, vLLM, LM Studio) or a
cloud API (OpenRouter).

## License

MIT, see [LICENSE](LICENSE).
