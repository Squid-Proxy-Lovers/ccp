# Contributing to CCP

Submit changes as GitHub pull requests, including maintainer changes. For an
existing issue, coordinate with its owner before starting. Discuss new protocol
features before implementing them.

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
ARM64. These are development tools, not runtime dependencies.

Rustup selects the compiler in `rust-toolchain.toml`. Rust 1.88 is the supported
minimum. Builds and tests use `Cargo.lock` with `--locked`.

## Before opening a PR

```bash
make lint-fast
make test
make quality-tests
make installer-tests
make integration
make mcp-smoke
make compat
make security
```

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
| Rust | rustfmt; Clippy for every workspace target with warnings as errors |
| Python | Ruff correctness, imports, bugbear, modernization, simplification, security, and formatting |
| Shell | ShellCheck for Git-visible Bash and POSIX scripts, including extensionless scripts |
| Actions | actionlint and zizmor's regular policy; pinned actions and read-only default permissions |
| Secrets | Gitleaks CLI on working source and Git history, with redacted output |
| Dependencies | cargo-deny advisories, licenses, bans, and sources; pip-audit |

The Rust policy lives in the workspace manifest and is inherited by every crate.
It also rejects debug macros, TODO/unimplemented macros, and unsafe operations
implicitly performed inside unsafe functions. Python targets 3.10-compatible
syntax; test assertions are allowed.

Fix findings before submission. When an intentional invariant or API shape needs
an exception, keep it local and explain the reason. Prefer Rust `#[expect(...,
reason = "...")]`, which detects stale exemptions, and specific Python `noqa`
codes with nearby explanations. Avoid whole-file or whole-workspace suppressions.
Do not enable entire opinionated rule groups just to increase the rule count.

Dependency vulnerabilities fail the audit. Existing maintenance notices and
duplicate versions are handled through dependency review rather than forcing
unrelated migrations. The license allowlist records accepted dependencies; new
licenses or sources need explicit review. Bincode 1.3 is retained for wire-format
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
protocol migration. Weekly audits detect new advisories even without a source change.

CCP uses semver. Patch releases preserve protocol/API compatibility; new features
or incompatible pre-1.0 changes require an appropriate minor version. Track wire
compatibility with `PROTOCOL_VERSION` in `crates/protocol/src/lib.rs`, and database
compatibility with `SCHEMA_VERSION` in `crates/server/src/init.rs`. Change those
when the corresponding contract changes, and document migration requirements.

## Security reports

Report vulnerabilities privately according to [SECURITY.md](SECURITY.md), rather
than opening a public issue.
