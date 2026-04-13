# Contributing to CCP

Contributions should be made as GitHub pull requests. Each PR gets reviewed by a maintainer and either merged or given feedback. This applies to everyone, including maintainers.

If you want to work on an open issue, comment on it first so nobody else picks it up at the same time.

## Setting up

```bash
git clone https://github.com/squid-proxy-lovers/ccp.git
cd ccp
cargo build --release
```

Run the test suite before submitting anything:

```bash
cargo test -p server --lib -- --test-threads=1
cargo test -p client --lib
bash tests/run.sh --skip-build
```

## Codebase layout

```text
crates/protocol/     shared wire format types (client + server depend on this)
crates/server/       the CCP server (Rust, SQLite, mTLS)
crates/client/       CLI client (Rust)
mcp/                 FastMCP bridge for Claude/Cursor/Codex (Python)
tests/               integration tests + benchmarks
docs/                design docs and format specs
```

## Pull requests

- Branch from `main`. Rebase onto current `main` before submitting if your branch has fallen behind.
- Please keep commits small. Each one should compile and pass tests on its own.
- Add tests for new functionality or bug fixes.
- CI runs tests, clippy, and format checks on every PR. Make sure those pass before requesting review.
- Run `cargo fmt --all` and `cargo clippy --workspace` before pushing.

## What we're looking for

- Bug fixes with a test that proves the fix
- Performance improvements with benchmark numbers
- New protocol features (open an issue first to discuss the design)
- Documentation fixes
- Test coverage for untested paths

## What we're not looking for

- Cosmetic refactors with no functional change
- Dependencies we don't need
- Features that break backward compatibility without discussion

## Versioning

CCP follows [semver](https://semver.org/). We're at `0.x.y` which means the protocol and API can still change between minor versions.

- `0.1.x` patch: bug fixes, doc corrections, no protocol changes
- `0.2.0` minor: new features, new protocol messages, new CLI commands
- `1.0.0` major: protocol and API are stable, backward compatibility is guaranteed from that point

`PROTOCOL_VERSION` in `crates/protocol/src/lib.rs` and `SCHEMA_VERSION` in `crates/server/src/init.rs` track wire format and database compatibility separately from the crate version. Bump those when your change affects what goes over the wire or what's stored in SQLite.

## Security

If you find a security vulnerability, do not open a public issue. See [SECURITY.md](SECURITY.md) for reporting instructions.
