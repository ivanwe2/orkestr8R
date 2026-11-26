# Contributing

Thanks for helping. orkestr8R is built by people and by agents working from the same notes, so the
rules are written down:

- **[AGENTS.md](AGENTS.md)** has the conventions, and they apply to people too: what to work on, the
  merge policy, the CLI contract, tests.
- **[docs/plan-v0.1.md](docs/plan-v0.1.md)** is the build plan: tasks, contracts, acceptance
  commands. **[docs/decisions.md](docs/decisions.md)** says what was decided and why.

## Build and test

You need Rust (the toolchain in `rust-toolchain.toml` installs itself through rustup) and git.

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

`--all-features` builds `ork-testtool`, the helper the process tests run. Tests never need a GPU or
a network: the grunt loop runs against a fake OpenAI-compatible server that replies from a script.

**On Windows without the MSVC build tools**, Rust can't link natively. Run cargo in Docker instead,
with the same arguments: `scripts/dcargo.sh test --all-features`. To get a native `ork.exe` anyway,
`scripts/build-windows.sh` cross-compiles one into `target-win/`.

## Changes

- One branch per change (`feat/…`, `fix/…`, `docs/…`, `chore/…`), conventional commits, a PR into
  `main`. PRs are squash-merged, so the PR title is the commit.
- A test comes with each fix.
- **The CLI is a contract** ([docs/cli.md](docs/cli.md)): with `--json`, stdout carries JSON Lines
  and the last line is always `result`. If a command, an event, the order or the report changes,
  docs/cli.md and `ork guide` change in the same PR; a test checks they name the same commands.
- Never commit keys, `.env` files, or transcripts of private code.

## Issues

Bugs and ideas go in [GitHub issues](https://github.com/ivanwe2/orkestr8R/issues). Security problems
don't: see [SECURITY.md](SECURITY.md).
