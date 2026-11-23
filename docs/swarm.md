# Swarm: where it goes after iteration 1

Direction, not a spec (2026-11-23). The goal: hand off several orders at once, each to the right
agent for its difficulty, and get back one checked result.

## Tiers and the escalation ladder

| Tier | Who | For |
|---|---|---|
| goblin | small, fast local model | `ask`, summaries, lookups, trivial edits |
| grunt | big local coding model | most orders: tests, refactors, features with clear checks |
| mercenary | paid cloud model (OpenRouter or any API) | harder orders, or more parallel width than the GPUs give |
| chief | the frontier session | planning, scoping, judgment; never a worker |

- Each order carries a `tier`, set by the chief: explicit and visible.
- **Escalation is a fixed rule, not a guess.** If an order fails its checks or gives up, it may go
  one rung up (goblin → grunt → mercenary) up to a ceiling set in the config. After that it goes back
  to the chief with the report. Every move is an event.
- Each tier is a role in `ork models`, so a "mercenary" is just a server and a model, like localdub's
  tasks.

## Plans: the plan is data

The chief writes one plan instead of N separate calls:

```yaml
plan: orders-pagination
checks: ["dotnet build", "dotnet test"]        # run after every merge
orders:
  - id: query
    tier: grunt
    goal: Add page/pageSize to OrderQuery and the repository
    write_scope: ["src/Orders/Query/**"]
    checks: ["dotnet test tests/Orders/Query"]
  - id: api
    needs: [query]
    tier: grunt
    goal: Expose page/pageSize on GET /api/orders, with validation
    write_scope: ["src/Orders/Api/**"]
    checks: ["dotnet test tests/Orders/Api"]
  - id: docs
    tier: goblin
    goal: Document the new query parameters in docs/api.md
    write_scope: ["docs/api.md"]
    checks: ["npx markdownlint docs/api.md"]
```

`ork plan run plan.yaml` (or `ork_plan` over MCP) runs it. Claude Code's dynamic workflows have
the chief write a script that the runtime executes ("a workflow moves the plan into code").
orkestr8R keeps the plan as **data**:
- it can be validated before anything runs (scopes overlap? checks missing? tier allowed?);
- it's shown in the UI as a graph;
- it can be resumed after a crash;
- the chief spends tokens once, on writing it.

## The scheduler

- An order is ready when everything in its `needs` has been merged.
- **Capacity per server.** A llama-server with `--parallel N` serves N requests at once through
  continuous batching. The context is split between the slots (`-c 16384 -np 4` gives each slot
  4096). For mercenaries, a concurrency limit from the config.
- **Orders whose `write_scope`s overlap never run side by side.** It's cheaper than solving merge
  conflicts.
- Idle grunts cost nothing: nothing polls a model.

## Isolation and the merge queue

- The plan gets an integration branch, `ork/plan-<id>`. Every order works in its own worktree,
  branched from it.
- Passed orders merge into the integration branch **one at a time**, and the plan's checks run after
  each merge.
- A conflict, or a check that breaks after a merge, becomes a fixer order (a grunt with both diffs)
  or goes back to the chief.
- At the end the chief (or you) reviews one branch, not N.

## Verifiers: someone whose job is to break it

Current multi-agent systems separate writing from checking:
- Google's Antigravity `/swarm` preview spins up adversarial verifiers that run tests and challenge
  diffs, apart from the workers.
- Claude Code's dynamic workflows use adversarial agents that try to break a result before you see
  it.

For orkestr8R: after an order passes, a **verifier** order (ideally a different model) gets the goal
and the diff. It must either add a test that fails, or say why the change is right. Locally that
costs only GPU time. It catches the class of bug that "the tests pass" doesn't.

## The shared board

The plan's event log (append-only JSONL, the same events as docs/cli.md plus `order` and `merge`)
is what everyone reads:
- `ork watch` and the UI follow it live;
- the chief gets the reports;
- grunts can post an `open_question` that pauses the order (exit code 3) until the chief or you
  answer.

Antigravity's swarm uses the same idea: a shared, append-only message board.

**Not yet:** grunts talking to each other directly, like Claude Code's agent teams. Hub-and-spoke
(chief ↔ orkestr8R ↔ grunts) is easier to debug, and parallel orders shouldn't need to chat if
their scopes don't overlap.

## Local capacity, honestly

On 2× 16 GB, a 27–31B model takes both GPUs, so "a local swarm" means **one big model, several
slots**. Each grunt gets fewer tokens per second, but the total goes up. llama-swap's groups decide
what may load together, as they already do for localdub.

Width beyond that comes from mercenaries. To measure before building the scheduler: tokens per
second per slot and in total at `--parallel 1/2/4`, with the context each grunt needs.

## Other harnesses as grunts (ACP)

A grunt doesn't have to be orkestr8R's own loop. The Agent Client Protocol (ACP) is a JSON-RPC
standard for driving an agent: sessions, streaming updates, permission requests, cancellation.
OpenCode speaks it (`opencode acp`), and so do others. Through ACP, orkestr8R can run OpenCode
pointed at the local model as a grunt runtime. orkestr8R still owns the worktree, the order, the
checks and the report.

That's iteration 2. The native loop stays the default and the reference.
