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
- **MCP** (v0.2). `ork mcp` serves the same commands as tools, for clients that prefer them.
- **Web UI** later (`ork serve`), built on the same commands: jobs live, the models' speed, the diffs,
  approve and apply.

## Quickstart

You need git, an OpenAI-compatible server for your local model (llama-server, llama-swap, Ollama,
vLLM, LM Studio), and either Rust or a release binary.

1. **Install.** From a clone of this repo: `cargo install --path . --locked`. Or put an `ork` binary
   from the [releases](https://github.com/ivanwe2/orkestr8R/releases) on your PATH.
2. **Point it at your model**, in the repo you want to work on:
   ```
   ork init --url http://gpu-box:9876/v1 --model qwen-27b
   ```
   That writes `ork.json` (the server, the grunt's model, and the build and test commands it may
   run, detected from `Cargo.toml`, `go.mod`, `*.csproj` or `package.json`) and keeps `.ork/` out of
   git. If the server needs an API key, use your own config instead:
   `ork init --user --url … --model … --key-env LLAMA_API_KEY` (the key stays in that variable).
3. **Check.** `ork doctor`: all ✓ means the server answers, the model calls tools, and the allowed
   commands are on PATH.
4. **Run an order.**
   ```
   ork run --goal "Add tests for parse_duration in src/time.rs" \
     --path src/time.rs --scope "src/time.rs" --scope "tests/**" \
     --check "cargo test --test time"
   ```
   Progress goes to stderr; the report to stdout, ending with
   `next: ork diff k7f2 · ork apply k7f2 · ork discard k7f2`.
5. **Land it.** `ork diff k7f2` to look, then `ork apply k7f2` (or `ork discard k7f2`).

**For Claude Code:** `cp -r skills/orkestr8r ~/.claude/skills/`. It then knows when to delegate, and
runs `ork guide` for the details (`--json`, long runs, reading the report).

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

Rust: one static binary, `ork`, for the core, the CLI and MCP (the official Rust SDK, `rmcp`);
React + TypeScript for the web UI later. Models: any OpenAI-compatible server (llama-server,
llama-swap, Ollama, vLLM, LM Studio) or a cloud API (OpenRouter).

## License

MIT, see [LICENSE](LICENSE).
