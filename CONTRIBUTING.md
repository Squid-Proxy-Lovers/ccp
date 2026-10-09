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

## Dependency updates

Dependabot opens weekly update PRs for Cargo, the MCP package, GitHub Actions,
Docker images, and the Rust toolchain. Cargo updates are limited to the existing
manifest ranges; Python major upgrades require a separate review. Updates are
reviewed PRs and do not merge automatically.

Rust 1.88 is the supported minimum. The pinned development/CI compiler is in
`rust-toolchain.toml`; when changing it, update the Docker builder version too.
Cargo's resolver prefers dependencies compatible with the minimum compiler.
Build with `--locked` so validation and release artifacts use the reviewed lockfile.

Before accepting dependency updates, run the existing unit/CLI tests, the MCP
smoke test, and the old/new client and persisted-data compatibility check. CI also
checks the minimum compiler and audits Rust/Python dependencies; the weekly audit
detects newly published advisories even when there is no source change.

Keep the protocol, database schema, CLI commands and public response shapes
compatible in maintenance updates. Bincode 1.3 remains in use to preserve the
version-1 wire format. RustSec reports it as unmaintained, without a known
vulnerability; replacing the serializer requires an explicitly versioned protocol
migration, rather than a routine dependency update.

## Versioning

CCP follows [semver](https://semver.org/). We're at `0.x.y` which means the protocol and API can still change between minor versions.

- `0.1.x` patch: bug fixes, doc corrections, no protocol changes
- `0.2.0` minor: new features, new protocol messages, new CLI commands
- `1.0.0` major: protocol and API are stable, backward compatibility is guaranteed from that point

`PROTOCOL_VERSION` in `crates/protocol/src/lib.rs` and `SCHEMA_VERSION` in `crates/server/src/init.rs` track wire format and database compatibility separately from the crate version. Bump those when your change affects what goes over the wire or what's stored in SQLite.

## Security

If you find a security vulnerability, do not open a public issue. See [SECURITY.md](SECURITY.md) for reporting instructions.
