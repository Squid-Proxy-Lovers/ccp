# CCP documentation

Start with the [quick start](../README.md#quick-start). Choose a guide by the task
you need to complete. Guides marked unreleased cover the workspace redesign;
the other guides describe the supported native interface.

## Use CCP

| Task | Guide |
| --- | --- |
| Install and try the CLI | [Project README](../README.md) |
| Enroll, select a session, and use commands or the Rust SDK | [Client guide](client.md) |
| Configure an agent host | [MCP bridge](../mcp/README.md) |
| Find tool arguments, responses, and resources | [MCP tool-call reference](tool-call-api.md) |
| Share context with another session | [Droplet guide and format](droplet-format.md) |
| Organize research, reviews, or handoffs | [Collaboration examples](use-cases.md) |

## Operate a server

| Task | Guide |
| --- | --- |
| Deploy, issue tokens, inspect health, and manage persistence | [Server guide](server.md) |
| Install or pin a main nightly | [Nightly builds](nightly-builds.md) |
| Set up unreleased workspace administration or replace lost admin keys locally | [Local administration](local-workspace-administration.md) |
| Provision unreleased instance certificates and export public trust | [Instance TLS identity](instance-tls-identity.md) |
| Report a vulnerability or read bounty terms | [Security policy](../SECURITY.md) |
| Find disclosure credits | [Hall of Fame](../HALL-OF-FAME.md) |

## Develop CCP

| Task | Guide |
| --- | --- |
| Set up the toolchain, run required checks, and submit changes | [Contributing](../CONTRIBUTING.md) |
| Brief a coding agent on the repository standard | [Agent guide](../AGENTS.md) |
| Select tests or run benchmarks | [Tests and verification](../tests/README.md) |
| Read historical performance measurements | [Recorded benchmarks](benchmarks.md) |
| Build the client crate | [Client crate](../crates/client/README.md) |
| Build the server crate | [Server crate](../crates/server/README.md) |
| Work with wire types and compatibility | [Protocol crate](../crates/protocol/README.md) |
| Understand the unreleased workspace migration and permission foundation | [Workspace access foundation](workspace-foundation.md) |
| Understand atomic first-owner appointment and initial key delivery | [Initial workspace owners](initial-workspace-owners.md) |
| Understand unreleased scoped-key authentication | [Workspace admin keys](workspace-admin-keys.md) |
| Understand unreleased initial enrollment policy | [Invitation state](workspace-invitation-state.md) |
| Understand unreleased atomic invitation redemption | [Certificate enrollment](workspace-enrollment.md) |
| Understand unreleased scoped/global client recovery | [Client recovery](workspace-recovery.md) |
| Understand explicit readmission of a revoked workspace member | [Targeted readmission](workspace-readmission.md) |
| Understand native/workspace data-directory exclusion and activation | [Runtime mode isolation](workspace-runtime-mode.md) |
| Understand registered mTLS peers and renewal-only retries | [Registered TLS peers](registered-tls-peers.md) |
| Edit documentation | [Writing guide](writing.md) |

## Earlier versions

The [archived README guide](archive-notes.md) maps the October 8 Defcon README to
current guides and records branch-specific features. Check it when looking for
HTTP subscriptions, team status, or hosted client setup from that version.
