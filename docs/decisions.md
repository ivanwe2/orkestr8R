# Decisions

Newest first. Each line: what was decided, and why. Open questions at the bottom.

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

- **License.** MIT, like localdub?
- **Plan format** (iteration 3): YAML or JSON?
- **Models.** Which small model for goblins, and does it fit beside the grunt on 2× 16 GB, or swap
  through llama-swap?
- **The orc names.** Does goblin / grunt / mercenary stay in the config, or only in the UI and docs?
- **Agents in this repo.** Merge policy: may Claude merge its own PRs once CI is green, as in
  localdub?
