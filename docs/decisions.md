# Decisions

Newest first. Each line: what was decided, and why. Open questions at the bottom.

## 2026-11-24

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
- **Model calls use raw `HttpClient` + `System.Text.Json`**, not Microsoft.Extensions.AI or the
  OpenAI SDK.
  - The wire JSON stays visible, which is the learning goal.
  - We get full control over malformed arguments, think blocks, tool calls written into the text,
    and llama.cpp extras.
  - It can sit behind `IChatClient` later.
- **Job state is files in `.ork/jobs/<id>/`, not SQLite.** SQLite only if the v0.2 daemon needs it.
- **Commands run without a shell.** The command line is split into arguments, allowed by token
  prefix, and refused if it has shell metacharacters. Checks pass the same policy.
- **`write_file` only creates new files**; changes to existing files go through `edit_file` (exact
  match, once). Whole-file rewrites are where local models mangle code.
- **`.ork/` goes into `.git/info/exclude`**, so the user's `.gitignore` stays untouched.
- **Tests:**
  - The CLI runs in-process (`CliApp.RunAsync`).
  - A fake OpenAI-compatible server (Kestrel on port 0) replies from scripts.
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
- **Stack: .NET 10 / C#** for the core, CLI and MCP (official MCP C# SDK 2.x), shipped as a dotnet
  tool. Ivan's strongest stack, and it stands apart from the Python and TypeScript tools in this space.
- **Every job in its own git worktree; checks judged by exit codes; bounded repair turns.**
- **Jobs run in-process in iteration 1** (inside `ork mcp` / `ork run`); the `ork serve` daemon comes
  in iteration 2.

## Open questions

- **Plan format** (iteration 3): YAML or JSON?
- **Models.** Which small model for goblins, and does it fit beside the grunt on 2× 16 GB, or swap
  through llama-swap?
- **The orc names.** Does goblin / grunt / mercenary stay in the config, or only in the UI and docs?
