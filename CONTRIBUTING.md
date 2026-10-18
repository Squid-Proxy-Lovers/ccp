# Contributing to CCP

Submit changes as GitHub pull requests, including maintainer changes. For an
existing issue, coordinate with its owner before starting. Discuss new protocol
features before implementing them.

[Documentation index](docs/README.md) · [Writing guide](docs/writing.md)

## Setup

Install Rust through rustup, Python 3.13, Git, and Make. The MCP package also
supports Python 3.10+; CI tests both the minimum and current supported environments.

```bash
git clone https://github.com/squid-proxy-lovers/ccp.git
cd ccp
python3 -m venv .venv
. .venv/bin/activate
python -m pip install -e ./mcp
make tools
```

`make tools` installs exact Python tool versions from `requirements-lint.txt` and
native binaries from `tools/lint-tools.json`. Native downloads are SHA-256 checked
before installation in `.lint-tools/bin`; Linux and macOS support x86_64 and
ARM64. These tools support development checks.

Rustup selects the compiler in `rust-toolchain.toml`. Rust 1.88 is the supported
minimum. Builds and tests use `Cargo.lock` with `--locked`.

## Build from a checkout

From the repository root:

```bash
make build         # optimized server/client in target/release/
cargo check --workspace --locked
make help          # available build, test, and quality commands
```

Cargo-built binaries are named `server` and `client`; the installer names them
`ccp-server` and `ccp-client`. See the [test guide](tests/README.md) to select suites.

## Before opening a PR

```bash
make check            # lint, types, dead code, coverage, CLI, test integrity, protocol API
make installer-tests  # needs Docker
make nightly-tests
make mcp-smoke
make compat
make security         # needs network
```

Run `make hooks` once to check formatting, Python, and secrets before each commit.
`make check` compares against `origin/main`; fetch first, or pass `BASE=<ref>`.
Coverage needs the `llvm-tools` component: `rustup component add llvm-tools`.

`make lint-fast` needs installed tools and cached Cargo dependencies, and performs
no advisory-network requests. Warm runs are quick; the initial Rust compilation
can take longer. `make security` needs network access to current advisory databases
and audits the active Python environment, so install the MCP package in that venv.
The audit verifies that CCP is installed from this checkout, then excludes that
local package from the advisory lookup. Every installed third-party version,
including transitive runtime and quality-tool dependencies, is audited strictly.
The audit does not resolve a different dependency set.

The [test guide](tests/README.md) explains individual suites, opt-in load tests,
and old/new compatibility checks. Use `make help` for targeted commands. CI runs
the source, security, test, MSRV, MCP compatibility, and container checks before
producing release artifacts.

## Quality policy

| Area | Required checks |
| --- | --- |
| Rust | rustfmt; Clippy for every workspace target with warnings as errors; rustdoc warnings; no unused dependencies (cargo-machete) |
| Rust safety | No `unwrap`, `expect`, `panic!`, `unreachable!`, or panicking index/slice outside tests; no discarded `Result`; no lossy casts; no `unsafe` without a reason |
| Size | Functions at most 100 lines and cognitive complexity 25 (Rust); complexity 10, five arguments, and statement/branch limits (Python) |
| Python | Ruff correctness, imports, bugbear, modernization, simplification, security, unused arguments, commented-out code, blind `except`, and formatting |
| Python types and dead code | `mypy --strict` for the MCP package and scripts; vulture for unused code |
| Tests | Rust line coverage may not fall below `tools/coverage-baseline.json`; no removed tests or assertions and no new skips without review |
| Protocol API | cargo-semver-checks on the `protocol` crate against `main` |
| Shell | ShellCheck for Git-visible Bash and POSIX scripts, including extensionless scripts |
| Actions | actionlint and zizmor's regular policy; pinned actions and read-only default permissions |
| Secrets | Gitleaks CLI on working source and Git history, with redacted output |
| Dependencies | cargo-deny advisories, licenses, bans, and sources; pip-audit |

The Rust policy lives in the workspace manifest and `clippy.toml`, and every
crate inherits it. Tests may unwrap, expect, panic, and index. Python targets
3.10-compatible syntax; test assertions are allowed.

Fix findings before submission. When an intentional invariant or API shape needs
an exception, keep it local and explain the reason. An exception without a reason
fails review. Prefer Rust `#[expect(...,
reason = "...")]`, which detects stale exemptions, and specific Python `noqa`
codes with nearby explanations. Avoid whole-file or whole-workspace suppressions.
Do not enable entire opinionated rule groups just to increase the rule count.

Functions marked as legacy for size or complexity predate the limits. Split them
before adding to them; the expectation then fails and must be removed. Raise the
coverage baseline when coverage improves; never lower it.

### Definition of done

A change is ready for review when:

- `make check` and the CI checks for the PR pass.
- New behaviour has tests, and fixed bugs have a regression test.
- No check was weakened to pass: no removed or skipped tests or assertions, no
  lowered thresholds, and no relaxed lint configuration. A reviewed reduction
  needs the `allow-test-reduction` label from a maintainer.
- Every exception states its reason next to the code.
- The PR description states what was verified, including what was not run.

Changes to the standard itself (lint and tool configuration, workflows,
`scripts/`, `tools/`, and agent instructions) and to the protocol, storage, and
authentication contracts need a code-owner review; see `.github/CODEOWNERS`.

Dependency vulnerabilities fail the audit. Existing maintenance notices and
duplicate versions receive separate dependency review. The license allowlist records
accepted dependencies; new licenses or sources need explicit review. Bincode 1.3 is retained for wire-format
compatibility, with its maintenance status tracked separately.

## Repository layout

```text
crates/protocol/   shared wire types and Bincode codec
crates/server/     mTLS server, enrollment, SQLite, and journal
crates/client/     Rust library and CLI
mcp/              Python FastMCP bridge
tests/            unit/integration support, smoke tests, and benchmarks
scripts/          development quality helpers
docs/             architecture and format contracts
```

## Pull requests

- Branch from current `main`; keep one coherent purpose and reviewable commits.
- Describe the problem, resulting behavior, and checks actually run.
- Include a regression test for a behavioral fix. Documentation-only edits need relevant link/command checks.
- Preserve public CLI, response, wire, and persisted-data contracts in maintenance changes.
- Keep generated credentials, databases, local environments, and build outputs out of Git.
- Keep PRs as drafts while required checks or design decisions are outstanding.

## Dependencies and versions

Dependabot opens weekly reviewed PRs for Cargo, Python, Actions, Docker, and the
Rust toolchain. Updates do not merge automatically. Cargo updates remain within
manifest ranges; Python major upgrades require separate review. When changing
the development compiler, update the Docker builder and verify the minimum compiler.

For dependency maintenance, run the old/new client and persistence checks as well
as the current CLI/MCP suites. Replacing Bincode requires an explicitly versioned
protocol migration. Daily nightly checks detect new advisories even without a source change.

CCP uses semver. Patch releases preserve protocol/API compatibility; new features
or incompatible pre-1.0 changes require an appropriate minor version. Track wire
compatibility with `PROTOCOL_VERSION` in `crates/protocol/src/lib.rs`, and database
compatibility with `SCHEMA_VERSION` in `crates/server/src/init.rs`. Change those
when the corresponding contract changes, and document migration requirements.

## Documentation changes

Follow the [writing guide](docs/writing.md): task-first structure, clear headings,
plain language, concrete examples, and links to canonical references. Check links,
anchors, example syntax, command flags, and tool signatures. Keep every public
Markdown document reachable from the [index](docs/README.md), and account for useful
material when shortening or moving it. Label archived behavior with its revision.

## Security reports

Report vulnerabilities privately according to [SECURITY.md](SECURITY.md), rather
than opening a public issue.
