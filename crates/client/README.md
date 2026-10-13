# Client crate

Rust CLI and library for enrollment, saved mTLS credentials, and CCP data operations.
See the [client guide](../../docs/client.md) for enrollment, all commands, permissions,
and an SDK example. The [project README](../../README.md#install) covers installation.

## Build

From the repository root:

```bash
cargo build --release --locked -p client
./target/release/client --help
```

Installed binaries are named `ccp-client`; Cargo builds `target/release/client`.
For tests and contribution requirements, see [CONTRIBUTING.md](../../CONTRIBUTING.md).

[Documentation index](../../docs/README.md) · [Wire protocol](../protocol/README.md)
