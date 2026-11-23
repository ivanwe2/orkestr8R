# Landscape (checked 2026-11-23)

What already exists, what's current, and what orkestr8R takes from each. Re-check before relying on
any of it; this space moves monthly.

## Closest existing tools

| Tool | What it does | What we take / how we differ |
|---|---|---|
| [houtini-lm](https://dev.to/richardbaxter/making-a-local-llm-mcp-server-deterministic-model-routing-think-block-stripping-and-the-problems-5bmj) | MCP server: Claude Code hands bounded tasks to local or cheap models | Lessons: Claude barely delegates by default; savings come from not reading big files; strip think blocks; serialize requests; plain, fixed routing rules |
| [claude-delegate-local-mcp](https://github.com/ComputerSaysNull/claude-delegate-local-mcp) | The closest: one-shot and agentic modes (the local model writes code, runs tests, retries), bubblewrap sandbox, success judged by exit codes | Same core idea. We add worktrees per job, a CLI-first contract, both Claude Code and OpenCode, .NET, and the swarm direction |
| [helix-agent](https://dev.to/tsunamayo7/stop-burning-api-tokens-auto-route-claude-code-tasks-to-local-ollama-models-31o9), [mcp-local-llm](https://github.com/aplaceforallmystuff/mcp-local-llm), [cc_token_saver_mcp](https://mcpservers.org/servers/csabakecskemeti/cc_token_saver_mcp) | MCP servers that route summaries, drafts and reviews to Ollama or OpenAI-compatible models | Mostly one-shot completions, not a harness |
| [OpenCode agents](https://opencode.ai/v2/docs/agents) | Each agent or subagent can have its own model, local included | Already does "a local subagent" inside OpenCode. orkestr8R works across clients and adds checks and worktrees |
| Claude Code ([issue #38698](https://github.com/anthropics/claude-code/issues/38698)) | Subagents can't be routed to another provider within one session | Why the bridge for Claude Code has to be a tool (CLI or MCP) |
| [Orkestra](https://github.com/andyyaro/orkestra), [Composio agent-orchestrator](https://github.com/ComposioHQ/agent-orchestrator) | Run fleets of Claude Code / Codex agents in worktrees, issue → PR | They orchestrate cloud agents. orkestr8R puts local models to work for a frontier chief. Mind the name next to Orkestra |

## What's current, and how orkestr8R uses it

**Workflows as code: Claude Code dynamic workflows** (GA, 2026)
- Claude writes an orchestration script and the runtime runs many subagents from it. In Anthropic's
  words, a workflow moves the plan into code.
- We take: the chief writes a plan once and orkestr8R executes it. We keep it as data (YAML) rather
  than a script (docs/swarm.md).
- [docs](https://code.claude.com/docs/en/workflows)

**Agent teams and swarms**
- Claude Code agent teams (experimental since February 2026): a lead plus teammates with a shared task
  list and messaging. [docs](https://code.claude.com/docs/en/agent-teams)
- Google Antigravity `/swarm` (preview, November 2026): parallel workers in isolated worktrees,
  separate adversarial verifiers, an append-only shared board.
  [article](https://n8nlab.io/news/gemini-carbon-agent-swarm-orchestration)
- We take: worktree per worker, verifiers apart from workers, the event log as the board.
  Hub-and-spoke first.

**Agent Client Protocol (ACP)**
- JSON-RPC between a client and an agent: sessions, streaming, permissions, cancellation. MCP is
  agent-to-tool; ACP is client-to-agent, so the whole harness becomes swappable.
- Thoughtworks Radar, November 2026: Assess.
- OpenCode runs as an ACP agent with `opencode acp`; `acpx` is a headless ACP client.
- We take: other harnesses as grunt runtimes (iteration 2).
- [Radar](https://www.thoughtworks.com/radar/platforms/agent-client-protocol-acp) ·
  [opencode acp](https://opencode.ai/docs/cli/acp/) · [acpx](https://github.com/openclaw/acpx)

**Agent Skills (SKILL.md)**
- An open standard since December 2025, donated to the Agentic AI Foundation. More than 70 platforms
  load it. Progressive disclosure: only the name and description load at startup, the body when
  needed.
- We take: ship `skills/orkestr8r` (the CLI + skill route costs the chief almost nothing until used),
  and later let grunts load skills too.
- [Anthropic](https://www.anthropic.com/news/donating-agent-skills-to-the-agentic-ai-foundation)

**MCP 2026-07-28**
- Stateless core: no initialize handshake, no sessions.
- Tasks moved into an extension: `tasks/get`, `tasks/update`, `tasks/cancel`.
- Roots, Sampling and Logging are deprecated, so don't design around sampling.
- MCP C# SDK 2.x implements it. The Claude Code CLI supports MCP Tasks (public beta from v2.1.304)
  and moves MCP calls longer than two minutes to the background.
- [spec blog](https://blog.modelcontextprotocol.io/posts/2026-07-28/) ·
  [changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog) ·
  [C# SDK](https://github.com/modelcontextprotocol/csharp-sdk) ·
  [Claude Code + Tasks](https://upstash.com/blog/claude-code-mcp-tasks)

**OpenTelemetry GenAI conventions**
- Standard span names for agents: `invoke_agent`, `chat`, `execute_tool`.
- Moved into their own repo in June 2026; still in development status.
- We take: emit the events as OTel spans too, so any tracing backend can show a job.
- [semantic-conventions-genai](https://github.com/open-telemetry/semantic-conventions-genai)

**A2A (Agent2Agent)**
- v1.0 shipped March 2026; joined the AAIF in August 2026; C# SDK available.
- For agents hiring agents across vendors. Not needed until orkestr8R is something other agents
  call over a network (iteration 4).
- [AAIF](https://aaif.io/blog/a2a-joins-aaif)

**Local serving**
- llama-server slots (`--parallel`) with continuous batching serve several requests at once, the
  context split between slots.
- llama-swap starts models on demand and groups what may run together (already on the GPU box for
  localdub).
- [llama-server](https://www.mintlify.com/ggml-org/llama.cpp/api/tools/llama-server) ·
  [llama-swap](https://github.com/mostlygeek/llama-swap)

## Reading for the agent loop

- **"Harness Engineering: Anatomy, Architecture, and Evolution of Coding Agents"**
  ([arXiv 2609.00006](https://arxiv.org/abs/2609.00006)). A source study of eleven harnesses: the
  seven parts, findings (no agent frameworks, deterministic tools, skills ahead of MCP, ACP in six of
  eleven), 18 recommendations and a 90-line minimum harness.
- **Mini-SWE-Agent.** About 100 lines: one loop, one bash tool, step and cost limits. The best first
  read.
