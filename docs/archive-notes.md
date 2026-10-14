# Archived README guide

The [October 8 Defcon archive](https://github.com/Squid-Proxy-Lovers/ccp/tree/archive/defcon2026-main-2026-10-08)
preserves revision `6aa65df2dd5e8250594ac3b4797abaa6af09a99a`. Its
[README](https://github.com/Squid-Proxy-Lovers/ccp/blob/6aa65df2dd5e8250594ac3b4797abaa6af09a99a/README.md)
describes a different transport and feature set from current main.

[Documentation index](README.md) · [Current quick start](../README.md#quick-start)

## Where the useful material lives

| Archived README topic | Current documentation |
| --- | --- |
| Purpose and agent coordination | [README overview](../README.md#what-ccp-does) and [collaboration examples](use-cases.md) |
| Binary, client/MCP, Docker, and source installs | [Install](../README.md#install), [MCP setup](../mcp/README.md), and [server deployment](server.md#deployment) |
| Build from a checkout | [Build commands](../CONTRIBUTING.md#build-from-a-checkout) |
| Create, append, search, and read entries | [Quick start](../README.md#quick-start) and [CLI reference](client.md#cli-reference) |
| Data hierarchy, labels, and history | [Entry model](client.md#entry-model) and [search/history](client.md#search-and-history) |
| Session selection and access | [Enrollment](client.md#enroll-and-select-a-session) and [permissions](client.md#credentials-and-permissions) |
| Docker configuration, logs, and shutdown | [Deployment](server.md#deployment) and [configuration](server.md#configuration-surface) |
| MCP tools and agent instructions | [MCP bridge](../mcp/README.md) and [complete tool reference](tool-call-api.md) |
| Droplet selectors, conflict policies, and checksum | [Droplet guide](droplet-format.md) |
| CTF, code review, research, and feature coordination | [Collaboration examples](use-cases.md) |
| Tests and performance table | [Tests](../tests/README.md) and [recorded benchmarks](benchmarks.md) |
| Architecture and wire contracts | [Server](server.md), [client](client.md), and [protocol crate](../crates/protocol/README.md) |
| Contributions, disclosure, contacts, and license | [Contributing](../CONTRIBUTING.md), [security](../SECURITY.md), and [README footer](../README.md#contributing-and-security) |

## Branch-specific features

These features remain documented in the archive. Current main uses token enrollment
and a framed binary protocol over mTLS. It does not expose the archived commands
or hosted-script endpoints listed below.

| Archive feature | Behavior and archived reference |
| --- | --- |
| HTTP subscriptions | `remote-sessions`, `subscribe`, and `subscribe-all` discover/select sessions and save local subscription records. Requests use the archived HTTP endpoint. See [archived client design](https://github.com/Squid-Proxy-Lovers/ccp/blob/6aa65df2dd5e8250594ac3b4797abaa6af09a99a/docs/client.md). |
| Multiple sessions in one HTTP service | The archive hosts sessions together and exposes `ccp-manage add`, `delete`, and `stats`. See [archived server design](https://github.com/Squid-Proxy-Lovers/ccp/blob/6aa65df2dd5e8250594ac3b4797abaa6af09a99a/docs/server.md) and [management script](https://github.com/Squid-Proxy-Lovers/ccp/blob/6aa65df2dd5e8250594ac3b4797abaa6af09a99a/scripts/ccp-manage). |
| Team status | Shelves act as teams. `set-status`, `clear-status`, `team-status`, and `search-team-status` publish or query agent work. Unrefreshed status expires after three hours. See [archived MCP guide](https://github.com/Squid-Proxy-Lovers/ccp/blob/6aa65df2dd5e8250594ac3b4797abaa6af09a99a/mcp/README.md). |
| Server-hosted client setup | HTTP endpoints serve client/MCP downloads and setup scripts that configure agent hosts. See the archived [shell installer](https://github.com/Squid-Proxy-Lovers/ccp/blob/6aa65df2dd5e8250594ac3b4797abaa6af09a99a/scripts/setup-client.sh) and [PowerShell installer](https://github.com/Squid-Proxy-Lovers/ccp/blob/6aa65df2dd5e8250594ac3b4797abaa6af09a99a/scripts/setup-client.ps1). |
| Shared-key management | The archived service and management scripts use their own HTTP/key rules. The README contains conflicting descriptions of key requirements; consult the pinned [server source](https://github.com/Squid-Proxy-Lovers/ccp/blob/6aa65df2dd5e8250594ac3b4797abaa6af09a99a/crates/server/src/lib.rs) for that revision. |

The archive's LAN address is an example from that deployment. Use the endpoint
provided by your server operator. Current installers support Linux and macOS;
the archived PowerShell workflow has its own branch-specific setup.

## Reading older claims

The archived droplet checksum detects changed entry data, but a sender can recompute
it after editing a bundle. Sender authenticity requires a separate trust mechanism.
The preserved benchmark table records historical machines and workload settings;
it has no source revision and needs a fresh run for a current comparison.

Use the guides for the version you run. This page maps documentation and preserves
historical context; it does not establish data or protocol compatibility between
the archived branch and main.
