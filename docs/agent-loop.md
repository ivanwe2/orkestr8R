# The agent loop

What it is, what one turn looks like on the wire, and how the grunt's loop will work. Written to
learn from; the code in M1 follows it step by step.

## An agent is a model plus a harness

A chat model only turns text into text. It can't read a file or run a test. An **agent** is the model
plus a **harness**: the program around it that offers tools, runs the tools the model asks for, feeds
the results back, and decides when to stop.

The loop at the heart of it is a `while`:

```
messages = [system prompt, the task]
repeat:
    reply = model(messages, tools)        # one HTTP request
    add reply to messages
    if reply has no tool calls: stop      # the model is answering, not acting
    for each tool call in reply:
        result = run the tool             # your code, not the model's
        add result to messages
```

That's the whole trick. The model never runs anything. It *asks*, in a structured format, and the
harness decides whether and how to do it. Each pass is one **turn**. The `messages` list is the
model's entire memory: every request sends all of it again.

## One turn on the wire

OpenAI-compatible chat completions, which llama-server, llama-swap, Ollama, vLLM and OpenRouter all
speak.

**1. The harness sends** the conversation so far, plus the tools as JSON Schema:

```json
{
  "model": "grunt",
  "temperature": 0.2,
  "messages": [
    { "role": "system", "content": "You are a coding agent. Use the tools. Call finish when done." },
    { "role": "user", "content": "Goal: add Sum(int a, int b) to src/Calc.cs. Check: dotnet test" }
  ],
  "tools": [
    { "type": "function", "function": {
        "name": "read_file",
        "description": "Read a file, with line numbers",
        "parameters": { "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"] } } }
  ]
}
```

**2. The model answers** with a tool call instead of text. `finish_reason` is `tool_calls`, and
`arguments` is a JSON *string* the harness has to parse and validate:

```json
{ "choices": [ { "finish_reason": "tool_calls", "message": {
    "role": "assistant", "content": null,
    "tool_calls": [ { "id": "call_1", "type": "function",
      "function": { "name": "read_file", "arguments": "{\"path\": \"src/Calc.cs\"}" } } ] } } ] }
```

**3. The harness runs** `read_file` itself. It appends the assistant message and a `tool` message
with the result, tied by `tool_call_id`, then sends everything again:

```json
{ "role": "tool", "tool_call_id": "call_1", "content": "1  public static class Calc\n2  {\n3  }\n" }
```

**4. Repeat.** The model asks for `edit_file`, then `run("dotnet test")`, then `finish`. Every tool
call in a reply needs its own `tool` message before the next request.

## The loop in Rust (a sketch)

This shows the shape, not real code: `Message`, `Tools` and `Checks` are ours. The real loop
(docs/plan-v0.1.md, T10–T12) talks to the server with raw `reqwest` + `serde_json`, so the JSON
above is exactly what goes over the wire.

```rust
pub async fn run(&mut self, order: &Order) -> anyhow::Result<Report> {
    let mut messages = vec![Message::system(prompts::GRUNT), Message::user(order.to_prompt())];
    let mut repairs = 0;

    for turn in 1..=order.max_turns {
        let reply = self.model.chat(&messages, &self.tools.schemas()).await?;   // one HTTP request
        messages.push(reply.to_message());

        if reply.tool_calls.is_empty() {                     // it wrote text instead of acting
            messages.push(Message::user("Use a tool, or call finish with a summary."));
            continue;
        }

        for call in &reply.tool_calls {
            if call.name != "finish" {
                let result = self.tools.run(call, order).await;          // path guard, allow-list, timeout inside
                messages.push(Message::tool(&call.id, truncate(&result)));
                continue;
            }

            let checks = self.checks.run(&order.checks).await?;         // WE run them; exit codes decide
            if checks.all_passed() || repairs == order.max_repairs {
                return Ok(Report::new(order, call, &checks, repairs, turn));
            }

            repairs += 1;
            messages.push(Message::tool(&call.id,
                format!("Checks failed:\n{}\nFix it, then call finish again.", checks.tail())));
        }
    }
    Ok(Report::gave_up(order, "max turns"))
}
```

Around 40 lines. Everything else in a harness is about making those lines safe, cheap and
reliable.

## What the rest of a harness is for

A source study of eleven coding harnesses (Claude Code, Codex CLI, OpenCode, Aider, Mini-SWE-Agent
and more) splits a harness into seven parts. Where orkestr8R's grunt stands in iteration 1:

| Part | What it does | The grunt in iteration 1 |
|---|---|---|
| Agent loop | model ↔ tools, stop conditions, recovery | the loop above; limits on turns, tokens, minutes; gives up after repeated malformed calls |
| LLM integration | the API, the prompt, reasoning settings | raw HTTP to an OpenAI-compatible server; low temperature; `<think>` blocks stripped |
| Tools and actions | what the model can do; file editing matters most | read, list, grep, write, edit (exact match), run, finish |
| Memory and context | what fits in the window | capped reads, truncated outputs, old outputs and old long arguments stubbed |
| Safety and permissions | what may run, isolation | worktree, path guard, `write_scope`, allow-listed commands, scrubbed env |
| Orchestration | sub-agents | none inside the grunt; orkestr8R itself is the orchestrator |
| Extensibility | config, hooks, skills, MCP | `ork.json` now; skills for grunts later |

Two findings from that study worth keeping in mind:
- A more complex loop doesn't predict better results; most code in real harnesses is safety, UX and
  plumbing.
- None of them use an agent framework or embedding search. They use plain tools like ripgrep and
  Markdown context files.

## What local models get wrong (and what the harness does)

| Problem | Harness answer |
|---|---|
| Malformed JSON in `arguments`, unknown tool names | return the error as the tool result once; repeated → `gave_up` |
| `<think>…</think>` leaking into content, or cut off mid-think | strip closed and orphaned blocks before using the content |
| Rewrites whole files, mangling them | `edit_file` with an exact, unique `old` string; `write_file` only for new files |
| Loops: the same call again and again | detect identical consecutive calls, nudge, then stop |
| Says "done" without testing | `finish` doesn't end the job; the checks do |
| Huge outputs (a test run, a big file) blow the context | head + tail truncation, capped reads, stubs for old outputs |
| A written file sits in the history twice (the call's arguments and later reads) and is resent every turn | old `write_file` / `edit_file` arguments over 200 characters stubbed past 75% of the window (JSON kept valid, ids paired); a write that copies a stub is refused |
| Wanders outside the task | `write_scope`, a turn limit, and a plain prompt that names the goal and the checks |

## Learning path

1. **Read a minimal agent.** Mini-SWE-Agent is about 100 lines: one loop, one prompt template, one
   bash tool, a message list, and limits on steps and cost. Its self-reported benchmark scores land
   in the same range as much bigger systems (different models and dates, so only roughly
   comparable).
2. **Read the harness study** ("Harness Engineering", arXiv 2609.00006). It has the seven parts above,
   18 design recommendations and a 90-line minimum viable harness.
3. **Build M1 in small steps**, each runnable against the fake model server:
   1. one chat request, no tools: see the JSON;
   2. one tool (`read_file`), one turn by hand;
   3. the loop: repeat until no tool calls;
   4. `write_file` and `edit_file`, inside a worktree;
   5. `run` with the allow-list, then `finish` and the checks;
   6. limits, truncation, think stripping, malformed calls;
   7. events and the transcript, then point it at the real model on the GPU box.

Links for all of this: docs/landscape.md.
