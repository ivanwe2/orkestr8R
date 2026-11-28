# The `ork` CLI and MCP contract

Draft, 2026-11-23. This is a contract: agents, scripts, the MCP server and the web UI all depend on
it. Change it in the same PR as the code.

## Commands

### v0.1

```
ork init --url URL --model MODEL [--key-env VAR] [--context-tokens N] [--user] [--force]
                                           write ork.json (the repo's, or the user's with --user); add .ork/ to .git/info/exclude
ork doctor [--role NAME]                   is everything in place? (config, git, the role's server and tool calling, allow-listed commands)
ork models [--test]                        which model does each role, on which server; --test sends a tiny request
ork ask "QUESTION" --path FILE [--path …] [--role NAME]
                                           a one-shot, read-only answer from the local model
ork run [ORDER.json | -] [--goal "…"] [--context "…"] [--path FILE]… [--scope GLOB]… [--check "CMD"]…
        [--readonly] [--role NAME] [--kind KIND] [--max-turns N] [--max-repairs N] [--max-minutes N]
                                           run one order: worktree, grunt loop, checks, report (flags override the file)
ork jobs                                   every job, newest first
ork status JOB                             one job: status, the last event while running, the report when done
ork diff JOB [--stat]                      the job's changes
ork logs JOB                               the transcript: every message to and from the model
ork apply JOB [--squash] [--force]         merge the job's branch into the current branch, then clean up
ork discard JOB                            drop the worktree and the branch
ork guide                                  the guide for AI agents (this file is the one for people)
ork skill install AGENT… [--repo]          install the skill for the agents you use: claude, opencode, codex, gemini, other, all
ork skill list [--repo]                    each agent: on this machine?, the skill missing, current or outdated, its path
ork tui                                    a live view for people: jobs, the models' tokens, speed and cost (local or cloud),
                                           the turn-by-turn track; diff, logs, apply and discard on keys; / commands
ork                                        alone, in a terminal: the same as ork tui; anywhere else, the help (exit 1)
```

In v0.1, `ork run` blocks until the job ends. An agent whose shell tool has a time limit starts it
in the background, reads the job id from the first line, and polls `ork status JOB --json`. Its
`result` has the job's status (`running` until the run ends) and, once it has ended, the same
report and exit code `ork run` gave. A running job keeps a heartbeat (`.ork/jobs/<id>/heartbeat`, rewritten
every 10 s); one quiet for 90 s is reported as `error`, reason `ork stopped: …`, by `status`, `jobs`
and the TUI. That's computed, not written: if the run was only asleep, its final status still lands.

`ork run -` reads the order JSON from stdin (no quoting trouble with long goals). `--path`, for
`run` and `ask`, is relative to where you are, else to the repo root; `run` stores it relative to
the root. `--scope` globs and an order file's `paths` are always relative to the root.

`ork tui` is the one command without `--json` (it refuses it): it's for people, and needs a terminal
(stdin and stdout). `ork` alone opens it too. It reads the same job files as `ork jobs` and
`ork status`, and its actions run the same commands as typed:

- keys: `d` diff, `l` logs, `a`/`s` apply or squash, `x` discard (changes ask y/n first);
- `/` opens a command line with suggestions: `/apply [job]`, `/squash`, `/discard`, `/diff`, `/logs`
  (a job id or the start of one, else the selected job), `/skill`, `/doctor`, `/models`, `/guide`,
  `/theme [dark|light]`, `/help`, `/quit`. Tab completes; Enter runs the highlighted one;
- the first time, when no agent has the skill, it asks which agents you use and runs
  `ork skill install` for the ones you pick (`/skill` asks again).

### Planned

```
ork run … --detach                         (v0.2) start in the background, print the job id
ork watch JOB                              (v0.2) follow a job's events live
ork mcp                                    (v0.2) the MCP server over stdio
ork init                                   (v0.2) also writes delegation rules for AGENTS.md / CLAUDE.md and MCP registration snippets
ork models --set ROLE=SERVER/MODEL         (later) change a role's model
ork models --server NAME --url URL [--key-env VAR] [--remove]
                                           (later) add, change or remove a server
```

## Exit codes

| Code | Means |
|---|---|
| 0 | done (for `run`: checks passed) |
| 1 | error (bad order, server down, git trouble) |
| 3 | waiting for a human (reserved in v0.1; later e.g. an order that needs approval before `apply`, or a grunt's open question) |
| 4 | finished, but the checks failed or the grunt gave up; the report says which |

## `--json`

Every command but `ork tui` takes `--json`. Messages for people go to stderr. stdout carries one JSON object per
line, and the last line is always `result`.

Every event has `event` first and then `ts`, when it happened (RFC 3339, UTC, milliseconds); the
examples below leave `ts` out after the first two.

```
{"event": "start", "ts": "2026-11-26T16:02:11.123Z", "command": "run", "job": "k7f2", "branch": "ork/k7f2", "worktree": ".ork/worktrees/k7f2", "role": "grunt", "model": "qwen-27b", "server": "box"}
{"event": "call", "ts": "2026-11-26T16:02:15.301Z", "turn": 1, "model": "qwen-27b", "server": "box", "tokens_in": 6120, "tokens_out": 212, "seconds": 4.1, "tokens_per_second": 51.7}
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
| `start` | the command started; for `run`: job, branch, worktree, role, model, server |
| `call` | every model request: model, server, tokens in/out, seconds, tokens per second; `cost` in US dollars when the server reports one (OpenRouter does) |
| `tool` | every tool call: name, short args, ok, ms (errors carry `error`) |
| `finish` | the grunt called `finish` |
| `check` | every check: command, exit code, seconds, last lines |
| `repair` | a repair turn starts |
| `warning` | e.g. a malformed tool call, a truncated output, a think block stripped |
| `result` | always last: `status` (`ok`, `passed`, `failed_checks`, `gave_up`, `error`; `ork status` also `running`, `applied`, `discarded`; `waiting` later), `exit_code`; then, as needed, `job`, `report` (`run`, `status`), `data` (other commands' output) and `error` (the message, when the status is `error`) |

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
  "max_minutes": 20,
  "role": "grunt",
  "kind": "feature"
}
```

- **Required:** `goal`; `checks` (at least one) and `write_scope`, unless `mode` is `readonly`.
- **`role`** (`--role`): the role that runs it, a name in the config's `roles`; `grunt` when left
  out. An unknown role is a problem with the order. `ork models` lists the roles; goblin (small,
  fast), grunt (the big local model) and mercenary (a paid cloud model) are suggested names, not
  built in. The job's `state.json` records the role and the model and server it resolved to, and the
  report says `role`.
- **`kind`** (`--kind`): what kind of work it is, for the scoreboard (v0.2): `test`, `fix`, `feature`,
  `refactor`, `docs`, `chore` or `investigate` (a read-only order's default). Never sent to the model.
- The limits are optional; the config's defaults fill them in.
- Every check must pass the command allow-list. A repo's `ork.json` that lists `allow_commands`
  sets it (replacing the user file's). Otherwise it's the user file's plus the commands the repo's
  build files imply (`config::detect_commands`: a Node repo's test, lint and
  typecheck scripts with its package manager, `cargo test`, `go test`, `dotnet test`, `pytest`).
  `ORK_CONFIG` is exact: nothing is added. `ork doctor` shows which commands came from the repo.
- A check whose program isn't on PATH (looked up the way the run will, with `PATHEXT` on Windows)
  is a problem with the order: refused before any job or model request.
- **`mode: readonly`** removes every writing tool. Use it for investigations ("find every place
  that…"): the answer comes back in the summary.
- **Planned:** `approve` (v0.2), and `id`, `needs`, `tier` for plans (iteration 3, docs/swarm.md).

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
  "duration_s": 402.5,
  "tool_calls": 23,
  "failed_tool_calls": 2,
  "tokens": { "in": 61234, "out": 4810 },
  "model": "qwen-27b",
  "branch": "ork/k7f2",
  "open_questions": []
}
```

- `duration_s`: wall time from the job's start to its report. `tool_calls` and `failed_tool_calls`
  count the grunt's tool calls (`finish` isn't one).
- `files_read` (read-only orders only, left out when empty): the files the grunt read, each once,
  so the chief can judge how far it looked.
- `failing_before` (left out when empty): when the checks failed, the failing one is run once more
  on the base commit, in a throwaway worktree; it's listed here if it fails there too, so the
  grunt didn't break it. A warning event (`already_failing`) says the same.
- A read-only job's worktree and branch are removed when it ends: `ork apply` and `ork diff`
  refuse it (it changed nothing), and the TUI shows it as answered, not waiting on you.

## Config: ork.json

The user file (`%APPDATA%\orkestr8r\ork.json` on Windows, else `$XDG_CONFIG_HOME/orkestr8r/ork.json`
or `~/.config/orkestr8r/ork.json`), then the repo's `ork.json`, each key overriding the one before
(`ORK_CONFIG` replaces both). `ork init` writes one; every key is optional except a `grunt` role.

```json
{
  "servers": {
    "box": { "url": "http://gpu-box:9876/v1", "key_env": "LLAMA_API_KEY" },
    "router": { "url": "https://openrouter.ai/api/v1", "key_env": "OPENROUTER_API_KEY" }
  },
  "roles": {
    "grunt": { "server": "box", "model": "qwen3-coder", "context_tokens": 65536, "thinking": false },
    "mercenary": { "server": "router", "model": "deepseek/deepseek-v4" }
  },
  "allow_commands": ["npm run test", "npm run lint"],
  "defaults": { "max_turns": 40, "max_repairs": 2, "max_minutes": 20, "max_tokens": 400000 }
}
```

- `servers.<name>`: `url` (OpenAI-compatible, ending in `/v1`); `key_env`, the variable holding the
  API key (only from the user file: a repo can't send your key elsewhere).
- `roles.<name>`: `server`, `model`; `context_tokens` (32768), `temperature` (0.2),
  `max_output_tokens` (8192).
  - `thinking`: `true`/`false` turns a reasoning model's thinking on or off; left out, the server's
    default. OpenRouter gets `reasoning: {enabled: true}` / `{effort: "none"}` (models whose
    reasoning is mandatory reject `none`); llama.cpp, llama-swap, vLLM and SGLang get
    `chat_template_kwargs: {enable_thinking}`.
  - `extra_body`: an object merged into every request last, for what a server spells its own way
    (not `model`, `messages`, `tools`, `tool_choice` or `stream`).
- `allow_commands`: what checks and the grunt may run (see The order).
- `defaults`: the limits an order doesn't set; `max_tokens` has no default.

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
`ork guide` for the commands. Same shape as localdub's skill. A test checks that `ork guide`, the
skill and this file name the same commands.

The skill is compiled into `ork`, so it always matches the binary. `ork skill install AGENT…`
writes it to `<dir>/orkestr8r/SKILL.md`, for the agents named and no others:

| Agent | id | For you | With `--repo` (commit it: everyone's agent gets it) |
|---|---|---|---|
| Claude Code | `claude` | `~/.claude/skills` (`$CLAUDE_CONFIG_DIR/skills`) | `.claude/skills` |
| OpenCode | `opencode` | `~/.config/opencode/skills` (`$XDG_CONFIG_HOME/opencode/skills`) | `.opencode/skills` |
| Codex | `codex` | `~/.agents/skills` | `.agents/skills` |
| Gemini CLI | `gemini` | `~/.gemini/skills` | `.gemini/skills` |
| any other | `other` | `~/.agents/skills`, the shared location | `.agents/skills` |

`~` is `%USERPROFILE%` on Windows. With `--repo`, nothing below the repo root may be a link: a cloned
`.claude/skills` that points into your home would otherwise get one of your files overwritten. Installing again updates an outdated copy (an older `ork`'s, or
an edited one) and leaves a current one alone. Without agents, a person at a terminal gets a
picker (the agents found on this machine start ticked); anywhere else it's an error that names
the ones found (their program on PATH, or their config folder). `result.data`:

- install: `{"installed": [{"agent", "path", "change": "installed" | "updated" | "unchanged"}]}`;
- list: `{"agents": [{"agent", "name", "found", "skill": "missing" | "current" | "outdated", "path"}]}`.
