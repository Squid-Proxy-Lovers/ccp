# Protocol crate

Shared Rust request/response types and Bincode encode/decode functions for the
client and server. The crate also defines entries, summaries, history, session
metadata, droplets, and conflict policies.

[Documentation index](../../docs/README.md) · [Server wire protocol](../../docs/server.md#wire-protocol)

## Compatibility

`PROTOCOL_VERSION` identifies the wire contract. `Handshake`, `HandshakeOk`, and
`HandshakeRejected` negotiate compatibility when a client connects. Changes to
request/response enums or their encoding can require a version bump; see
[versioning requirements](../../CONTRIBUTING.md#dependencies-and-versions).

Both binaries compile against these types. Independently deployed versions still
need compatible wire contracts.

## Use in Rust

From a sibling crate, add the dependency to `Cargo.toml`:

```toml
[dependencies]
protocol = { path = "../protocol" }
```

Import the request/response enums, domain types, or encode/decode functions from
[lib.rs](src/lib.rs). This crate builds as a library.

## Build and test

From the repository root:

```bash
cargo build --locked -p protocol
cargo test --locked -p protocol
```
