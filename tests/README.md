# Tests and verification

[Documentation index](../docs/README.md)

Start with [CONTRIBUTING.md](../CONTRIBUTING.md) to install Rust, the MCP package,
and the pinned quality tools. Each command runs against local isolated test data.

## Rust tests

```bash
make test
# List cases without running them:
cargo test --workspace --locked -- --list
```

Use `--list` for the current Rust case inventory as workspace features are added.
The quality suite includes tool/accounting regressions and workspace MCP config,
argument, child-process and selection-handle tests. The Docker installer suite is
separate. The nightly suite checks artifact integrity, publication recovery and
real-installer channel selection. The CLI suite separately checks 59 assertions.

The workspace contains server and client library tests plus the Rust integration
suite. Protocol and database compatibility are separate versioned contracts.
Tests that mutate process-global environment variables run serially.

Two integration load tests are excluded from normal CI. Run them explicitly:

```bash
cargo test -p ccp-tests --locked -- --ignored --test-threads=1
```

## CLI integration

```bash
make integration
# Reuse already-built release binaries:
bash tests/run.sh --skip-build
```

The script starts a real TLS server, issues tokens, enrolls clients, and checks
library CRUD, append/history, searches, deletion/restoration, droplets, briefing,
temporal reads, access control, revocation, health, permissions, and shutdown.
Command checks update counters in the parent shell and retain raw output for
content assertions. The script exits unsuccessfully if any assertion fails.
Run library tests separately through `make test`.

## MCP and dependency compatibility

```bash
make mcp-smoke
make compat
```

The MCP command runs both the native and workspace smoke suites using actual
FastMCP transport, CLI processes and TLS servers. The native suite exercises
discovery, help/session resources, enrollment, writes, append, read and briefing.
The workspace suite exercises the stdio entry point, issued selection handles,
independent callers even on one connection, explicit targets, scoped recovery,
current revocation/disable, process restart and private-state cleanup. It also
runs seven content read tools through actual stdio/CLI/TLS, including historical
and archived entry rules, large escaped snapshots and scoped recovery without
authority union. CI runs both across supported Python/FastMCP combinations.
It also runs private-file shelf/book creation through MCP, including pending
entry guards, exact explicit targets across default changes, current denials,
recovery isolation and temporary-file cleanup. Entry tests exercise all five
actions, safe metadata projections, exact guarded retry after local save failure,
read-only cancellation, large private payloads, recovered-profile resolution and
refusal of stale CLI guards before action. SDK input/error privacy checks cover
both creation and entry-intent envelopes.

The compatibility smoke enrolls clients, verifies the existing wire codec,
reads/appends context, and reopens persisted data. For a dependency update, test
both revisions:

```bash
python tests/dependency_compat.py \
  --server target/release/server --client target/release/client \
  --old-server /path/to/base/server --old-client /path/to/base/client
```

CI builds the PR base and performs this cross-version check when protocol and
schema versions match. A deliberate migration needs its own migration tests.

## Quality and security

```bash
make lint-fast
make installer-tests
make nightly-tests
make security
PATH="$PWD/.lint-tools/bin:$PATH" python -m unittest discover -s tests -p 'test_quality.py' -v
```

Source checks cover Rust, Python, shell, workflow security, and working-file
secrets. CI also scans Git history. Security checks audit known vulnerabilities
and dependency policies using current network data.

Quality regressions verify checksum failure preserves existing tools, archive
members cannot write outside the install directory, archive symlinks are refused,
untracked secrets and extensionless shell defects are detected, and CLI counts
stay in the parent shell. Deliberate bad fixtures must make the corresponding
gate fail.

The quality/security gate has ten check categories: Rust formatting, Clippy,
Python lint, Python formatting, shell lint, workflow correctness, workflow
security, secrets, Rust dependency policy, and Python dependency advisories.

Count executable test cases separately from CLI assertions, smoke suites, and
lint rules. Matrix repetitions validate compatibility environments; they do not
create new unique test cases. See test output for the current count.

## Benchmarks

```bash
cargo run --release -p ccp-tests --locked --bin benchmark -- --mode suite
cargo run --release -p ccp-tests --locked --bin benchmark -- --mode append --clients 16 --requests-per-client 1000
```

Supported modes: `list`, `get`, `search-entries-simple`, `search-entries-complex`,
`search-entries-miss`, `search-context-simple`, `search-context-complex`,
`search-context-miss`, `append`, `mixed`, `suite`, and `full-suite`.
Results go to `tests/benchmark-results/`. Historical published results are retained
in [recorded benchmarks](../docs/benchmarks.md).

Report machine, revision, build settings, concurrency, and workload when sharing
results. Interpret throughput using those run settings. Benchmarks and the opt-in
load tests are outside the quick quality gate.
