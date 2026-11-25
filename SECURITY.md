# Security

## Reporting a problem

Please report security problems privately, through
[GitHub's private vulnerability reporting](https://github.com/ivanwe2/orkestr8R/security/advisories/new),
not as a public issue. You'll get an answer within a few days. orkestr8R is pre-1.0: fixes go into
`main` and the next release.

## What orkestr8R protects, and what it doesn't

orkestr8R runs a model's tool calls and your checks on your machine, so it's worth being precise.

**It does:**
- Run every job in its own git worktree, on its own branch. Nothing reaches your branch until you
  run `ork apply`.
- Resolve every path the model names, refuse anything outside the worktree or inside `.git` (links
  are judged by where they really lead), and refuse writes outside the order's `write_scope`.
- Run commands without a shell, only when they're on the `allow_commands` list, with a timeout, and
  kill the whole process tree when it's over.
- Pass commands a scrubbed environment: a short list of named variables (PATH, HOME, toolchain
  paths), no API keys or tokens.
- Send an API key only to a server whose URL and key variable your own user config names, so a
  cloned repo's `ork.json` can't redirect it.
- Check job ids and job files before they reach a path or a git argument, and escape control
  characters in everything shown in your terminal.

**It doesn't:**
- **Sandbox anything.** The command allow-list stops accidents (`git push`, `rm`), not a hostile
  model: allowed commands have flags that run other programs, and every check runs code the grunt
  wrote. A process started by a check can do whatever your user account can.
- Make an untrusted repo safe. Its build and its tests run, and so does its `ork.json`'s
  allow-list.

So: use orkestr8R on code whose tests you'd run yourself. OS-level isolation (bubblewrap on Linux)
is planned for v0.3.
