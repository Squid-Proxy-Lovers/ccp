# ccp-protocol

This is the shared wire-format crate for the Cephalopod Coordination Protocol. It defines every type that travels between client and server, and it handles serialization so neither side has to think about it.

## What's in here

- `PROTOCOL_VERSION` constant for wire compatibility checks between client and server.
- `Handshake`/`HandshakeOk`/`HandshakeRejected` for version negotiation on connect.
- Request and response enums that map to every command the protocol supports (add shelf, add book, add entry, search, list, delete, restore, export, import, etc.).
- Bincode serialization for all of these types. The client and server both depend on this crate and use the same codec, so they can't drift out of sync.
- Shared domain types like `Entry`, `Shelf`, `Book`, `HistoryRecord`, and access-level enums.

## How to use it

You don't run this crate directly. Add it as a dependency in your `Cargo.toml`:

```toml
[dependencies]
ccp-protocol = { path = "../protocol" }
```

Then import whatever you need. The public API is the set of request/response enums and the encode/decode functions.

## Building

```bash
cargo build -p protocol
```

Or from this directory:

```bash
cargo build
```

Tests:

```bash
cargo test -p protocol
```
