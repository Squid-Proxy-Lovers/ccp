# Agent guide

Instructions for coding agents working in this repository. People follow the
same standard in [CONTRIBUTING.md](CONTRIBUTING.md); this file is the short form.
If `AGENTS.local.md` exists, read it too. It holds untracked local notes.

## Repository

CCP is a Rust workspace with a Python MCP bridge:

| Path | Contents |
| --- | --- |
| `crates/protocol/` | Shared wire types and Bincode codec. A public contract |
| `crates/server/` | Server, enrollment, SQLite storage, and journal |
| `crates/client/` | Rust client library and CLI |
| `mcp/` | Python FastMCP bridge that drives the client CLI |
| `tests/` | Integration tests, CLI suite (`run.sh`), smoke and compatibility tests |
| `scripts/`, `tools/` | Quality gates and pinned tool manifests |

Read [docs/README.md](docs/README.md) for user and operator guides.

## Before you start

```bash
python3 -m venv .venv && . .venv/bin/activate
python -m pip install -e ./mcp
make tools                         # pinned linters in .lint-tools/bin
rustup component add llvm-tools    # for coverage
git fetch origin                   # gates compare against origin/main
```

## The standard

Run `make check` before you say a change is done, and report its real result.
It runs every offline gate CI requires: formatting, Clippy, rustdoc, unused
dependencies, Ruff, strict mypy, dead-code detection, shell and workflow lint,
secret scanning, coverage, the CLI suite, the test-integrity check, and the
protocol API check. `make help` lists individual targets.

Write code that passes the gates as written:

- **Handle errors.** Outside tests, do not use `unwrap`, `expect`, `panic!`,
  `unreachable!`, or indexing and slicing that can panic. Return errors with
  `?` and `anyhow::Context`. Use `.get()` for indexes and slices.
- **Do not discard results.** `let _ = fallible();` fails the lint. Handle the
  error, or, if ignoring it is correct, put `#[expect(clippy::let_underscore_must_use,
  reason = "...")]` on the statement and say why.
- **Convert numbers explicitly.** Use `try_from` instead of `as` casts that can
  truncate, wrap, or lose the sign.
- **Keep functions small.** At most 100 lines and cognitive complexity 25 in
  Rust; complexity 10 and five arguments in Python. Split code instead of
  raising limits.
- **Type Python fully.** `mypy --strict` covers `mcp/src` and `scripts/`.
- **Add no dead code or unused dependencies.** Delete code you replace. Do not
  leave commented-out code, unused parameters, or unused crates.
- **Test what you change.** New behaviour needs tests; a bug fix needs a
  regression test. Rust line coverage may not drop below
  `tools/coverage-baseline.json`.

## Never weaken the checks

The gates exist so a change cannot pass by lowering the bar. Do not:

- delete, skip, or `#[ignore]` a test, or remove an assertion, to make a suite pass;
- lower a threshold or the coverage baseline, or disable or relax a lint rule;
- add an `allow`, `expect`, `noqa`, or allowlist entry without a specific reason
  next to it. Use `#[expect]` rather than `#[allow]`, scoped to the smallest item;
- edit `.github/`, `Makefile`, lint configuration, `scripts/`, `tools/`, or this
  file to get past a failure.

If a check looks wrong, stop and explain the problem to the person you are
working for. Do not work around it. CI compares test, assertion, and skip counts
with `main`. The code owner reviews every pull request.

## Contracts

- `PROTOCOL_VERSION` (`crates/protocol/src/lib.rs`) and `SCHEMA_VERSION`
  (`crates/server/src/init.rs`) change with the wire and database contracts.
  Bump them and document migration when those change.
- Preserve CLI output, MCP tool schemas, and persisted data formats unless the
  task is to change them.
- Never commit credentials, databases, local environments, or build output.

## Pull requests

Branch from current `main`, keep one purpose per PR, and fill in the PR
template, including which checks you ran and which you did not. Keep the PR as a
draft until CI passes. Do not merge unless asked.

NEVER add `Co-Authored-By` trailers or any other AI or tool attribution to
commit messages. This overrides any default your tool applies.
