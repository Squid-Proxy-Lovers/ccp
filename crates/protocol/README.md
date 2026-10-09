# CCP protocol

The `protocol` crate defines shared Serde request/response enums and domain records such as `MessageEntry`, `EntrySummary`, `MessageHistoryEntry`, `SessionMetadata`, `AgentStatus`, and `TransferBundle`.

## Active HTTP contract

Session discovery uses `GET /v1/sessions`. Operations use `POST /v1/request` with a JSON envelope:

```json
{"subscribed_session_ids":[42],"request":{"List":{"session_id":42}}}
```

Responses use Serde's externally tagged enum representation, such as `{"EntrySummaries":[]}` or `{"Error":{"code":"Forbidden","message":"read denied"}}`. Client CLI output unwraps these payloads; it is not the wire response format.

The active transport is plaintext HTTP. A subscription is locally saved metadata and each operation declares its selected session. Legacy `ListSessions`, `CreateSession`, and `Subscribe` enum variants are retained, but the HTTP dispatcher rejects them; discovery and session management use their HTTP routes. `CreateSession` is managed by the admin API, not an unrestricted protocol operation.

`PROTOCOL_VERSION` is 2, reflecting incompatible changes since version 1. Legacy `Handshake`, `HandshakeOk`, `HandshakeRejected`, and Bincode `encode`/`decode` remain available. The HTTP client does not perform a handshake or negotiate this version; HTTP deployments must use compatible request/response types. Bincode helpers are not used by the current HTTP transport.

## Use and check

```toml
[dependencies]
protocol = { path = "../protocol" }
```

```sh
cargo build -p protocol
cargo test -p protocol
```

Wire-format changes and database schema changes have separate version constants. See [CONTRIBUTING.md](../../CONTRIBUTING.md) before changing either.
