# Tests

Install Rust 1.88.0, Node.js 22 or newer, and Python 3.12 or newer, then install the MCP package in a virtual environment:

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install -e ./mcp
make test
make integration
python tests/mcp_live.py
```

`make test` runs the complete Rust workspace, Python MCP tests, isolated operations-script tests, and deterministic dashboard regressions. `make integration` builds release binaries and runs the real CLI against a temporary loopback HTTP server. The live MCP test exercises FastMCP tools through the compiled CLI, HTTP server, and persistent data, including graceful restart.

## CLI integration

```sh
bash tests/run.sh
bash tests/run.sh --skip-build
```

The runner covers session discovery/subscription, shelf/book/entry CRUD, append/history, search, agent status, delete/restore, export/import, briefing, master instructions, temporal reads, and process shutdown. It asserts command exit status and JSON results. It creates isolated client/server directories and removes them on exit. Unit tests run separately via `make test`.

HTTP subscription metadata replaces certificate enrollment in the current transport. This suite does not claim TLS enrollment, read-only certificate access control, or certificate revocation coverage.

## Rust HTTP tests

```sh
cargo test -p ccp-tests --locked -- --test-threads=1
```

These use an in-process HTTP server and configured test keys. They cover administrator authorization, multi-session administration, instructions/activity, and subscription enforcement. They also verify the benchmark's HTTP transport with a small CRUD/load smoke test. Server shutdown is awaited before cleaning storage or restoring environment.

## Benchmarks

```sh
cargo run --release -p ccp-tests --locked --bin benchmark -- --mode suite
cargo run --release -p ccp-tests --locked --bin benchmark -- --mode append --clients 16 --requests-per-client 1000
```

Supported modes: `list`, `get`, `search-entries-simple`, `search-entries-complex`, `search-entries-miss`, `search-context-simple`, `search-context-complex`, `search-context-miss`, `append`, `mixed`, `suite`, `full-suite`. `suite` and `full-suite` select the same scenarios. Results go to `tests/benchmark-results/`.

Each client reuses an HTTP connection pool and sends sequential JSON requests. There is no persistent authenticated TLS session or request pipelining. Timings exclude startup, seeding, subscription, and connection warmup. Scenarios share seeded state, so append growth carries into later scenarios. Latencies are collected in memory; throughput measures a fixed concurrency workload rather than a fixed arrival rate. An error stops the scenario; successful results do not imply an error-rate measurement or production capacity estimate.
