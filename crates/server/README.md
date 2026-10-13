# Server crate

Rust server with HTTP enrollment, client-certificate-authenticated TLS, SQLite,
and a mutation journal. See the [server guide](../../docs/server.md) for deployment,
configuration defaults, token issuance, health checks, and architecture.

## Build

From the repository root:

```bash
cargo build --release --locked -p server
./target/release/server my-session
```

Installed binaries are named `ccp-server`; Cargo builds `target/release/server`.
For tests and contribution requirements, see [CONTRIBUTING.md](../../CONTRIBUTING.md).

[Documentation index](../../docs/README.md) · [Wire protocol](../protocol/README.md)
