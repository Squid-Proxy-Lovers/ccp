# Tests

## Integration tests

Builds the binaries, boots a real server, enrolls clients, and exercises every CLI operation end to end. Tears everything down when done.

```bash
bash tests/run.sh
```

Use `--skip-build` if you already have binaries in `target/release/`.

Covers these: token issuance, enrollment, shelf/book/entry CRUD, append, search (entries, context, shelves, books), delete, restore, export, import, access control enforcement, certificate revocation, file permissions, git secret tracking, server health, and clean shutdown.

## Rust end-to-end tests

Same coverage but through the Rust test harness instead of the CLI. Runs a real server in-process.

```bash
cargo test -p ccp-tests
```

The ignored tests are load tests. Run them separately:

```bash
cargo test -p ccp-tests -- --ignored
```

## Benchmarks

```bash
cargo run -p ccp-tests --bin benchmark -- --mode suite
cargo run -p ccp-tests --bin benchmark -- --mode full-suite
```

Single scenario:

```bash
cargo run -p ccp-tests --bin benchmark -- --mode append --clients 16 --requests-per-client 1000
```

Supported modes: `list`, `get`, `search-entries-simple`, `search-entries-complex`, `search-entries-miss`, `search-context-simple`, `search-context-complex`, `search-context-miss`, `append`, `mixed`, `suite`, `full-suite`.

Results go to `tests/benchmark-results/`.
