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
| Read and edit workspace context with guarded retries through configured MCP profiles | [Workspace MCP](workspace-mcp.md) |
| Find tool arguments, responses, and resources | [MCP tool-call reference](tool-call-api.md) |
| Share context with another session | [Droplet guide and format](droplet-format.md) |
| Organize research, reviews, or handoffs | [Collaboration examples](use-cases.md) |

## Operate a server

| Task | Guide |
| --- | --- |
| Deploy, issue tokens, inspect health, and manage persistence | [Server guide](server.md) |
| Understand native source provenance, visibility and deletion scope | [Native provenance storage](native-provenance.md) |
| Convert an offline native schema-2 instance into a fresh workspace instance | [Native conversion](native-conversion.md) |
| Install or pin a main nightly | [Nightly builds](nightly-builds.md) |
| Set up unreleased workspace administration or replace lost admin keys locally | [Local administration](local-workspace-administration.md) |
| Provision unreleased instance certificates and export public trust | [Instance TLS identity](instance-tls-identity.md) |
| Renew the unreleased instance server certificate or change its names | [Server certificate renewal](server-certificate-renewal.md) |
| Run the unreleased TLS workspace API and inspect its request contract | [Workspace HTTPS API](workspace-http-api.md) |
| Pin instance trust and enroll an unreleased workspace client | [Workspace client](workspace-client.md) |
| Select workspace targets independently for each caller | [Workspace selection](workspace-selection.md) |
| Appoint workspace admins, rotate keys or transfer ownership | [Owner administration](workspace-owner-administration.md) |
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
| Permanently purge an archived entry through scoped admin HTTPS | [Confirmed purge request](workspace-content-http.md#permanently-purge-an-archived-entry) |
| Understand explicit archived-entry purge and retained proof | [Entry purge core](workspace-content.md#permanently-purge-an-archived-entry) |
| Use scoped browser-session login, cookies and CSRF | [Browser-session API](workspace-browser-session.md) |
| Permanently delete an already archived workspace with explicit confirmation | [Workspace deletion](workspace-deletion.md) |
| Configure optional timed entry cleanup and understand paused clocks | [Entry retention](workspace-retention.md) |
| Inspect authenticated guidance-delivery context | [Client context output](workspace-client.md#retrieve-a-guidance-delivery-context) |
| Read automatic guidance beside MCP results | [MCP guidance sidebands](workspace-mcp.md#read-automatic-guidance-alongside-results) |
| Refresh current guidance through MCP | [Explicit MCP guidance](workspace-mcp.md#refresh-guidance-explicitly) |
| Retrieve guidance with a registered client | [Client guidance reader](workspace-client.md#read-current-guidance) |
| Read or edit live guidance over TLS | [Guidance HTTP API](workspace-http-api.md#read-and-save-guidance) |
| Manage verified client/MCP bundles and optional automatic updates | [Managed client bundles](managed-client.md) |
| Verify matching official client/MCP bytes and publisher provenance | [Release bundle verification](client-release-bundles.md) |
| Read reported work and use explicit client status messages | [Work-status storage](workspace-status.md) |
| Inspect a stopped native database without conversion or source changes | [Native conversion inventory](native-conversion-inventory.md) |
| Start a workspace instance and complete first-boot setup | [One-command setup](workspace-foundation.md#start-an-instance-in-one-command) |
| Understand the unreleased workspace migration, permissions and guidance foundation | [Workspace access foundation](workspace-foundation.md) |
| Create shelves/books and read workspace context and shelf lifecycle metadata through the normal client | [Workspace content client](workspace-content-client.md) |
| Edit entries and resume uncertain client mutations | [Workspace entry mutations](workspace-content-mutations.md) |
| Resolve an uncertain client entry operation after renewal or recovery | [Operation resolution](workspace-operation-resolution.md) |
| Read, edit and archive unreleased workspace content through authenticated HTTPS | [Workspace content HTTP](workspace-content-http.md) |
| Understand workspace hierarchy, ownership, revisions, retained retry proofs and shelf recovery | [Workspace content foundation](workspace-content.md) |
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
