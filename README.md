<div id="user-content-toc" align="center">
  <img src=".github/spl-logo.png" alt="SPL" width="200">
  <ul style="list-style: none;">
    <summary>
      <h1>Cephalopod Coordination Protocol</h1>
    </summary>
  </ul>
  <p>A Rust-based client-server coordination protocol for agentic systems.</p>
</div>

<p align="center">
  <a href="#install">Install</a> &middot;
  <a href="#quick-start">Quick Start</a> &middot;
  <a href="#droplets">Droplets</a> &middot;
  <a href="#use-cases">Use Cases</a> &middot;
  <a href="docs/">Docs</a> &middot;
    <a href="#quick-start">Sessions</a>
</p>

<p align="center">
  <a href="https://github.com/squid-proxy-lovers/ccp/actions"><img src="https://github.com/squid-proxy-lovers/ccp/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0-blue" alt="License"></a>
</p>

## What is this

When you have multiple agents working together they need somewhere to share context. One agent finds something, another agent needs to know about it. Right now most setups either pipe everything through the orchestrator or dump state into shared files. Both fall apart once you have more than a couple agents or need any kind of access control.

CCP is a dedicated coordination layer. One server hosts multiple isolated sessions over a plaintext HTTP endpoint. Clients subscribe only to the sessions they want to use. Entries are journaled and searchable, and graceful shutdown persists the complete state so agents can pick up where others left off.

This is useful if you're building multi-agent workflows where agents need to coordinate without going through a single bottleneck. Research agents can dump findings into shared entries. Planning agents can read those findings and write plans. Review agents can search across everything and flag issues. Each agent operates independently; the current HTTP transport shares a worker identity and records optional agent attribution. See [use cases](#use-cases) for real examples.

## Install

This draft targets `0.2.0` and introduces the HTTP protocol. Build from this checkout to test it; existing `0.1.x` releases on main use the older transport.

```bash
# Server + client (downloads prebuilt binaries)
curl -fsSL https://raw.githubusercontent.com/squid-proxy-lovers/ccp/main/install.sh | bash

# Client only (auto-configures MCP for Claude/Cursor/Codex)
curl -fsSL https://raw.githubusercontent.com/squid-proxy-lovers/ccp/main/install.sh | bash -s -- --client

# Docker
curl -fsSL https://raw.githubusercontent.com/squid-proxy-lovers/ccp/main/install.sh | bash -s -- --docker

# From source
curl -fsSL https://raw.githubusercontent.com/squid-proxy-lovers/ccp/main/install.sh | bash -s -- --from-source
```

Binaries go to `~/.local/bin`. Pass `--install-dir /usr/local/bin` to change that.

The `--client` flag is what most people want if someone else is running the server. It installs the client binary and auto-detects your Claude, Cursor, or Codex config files to register the MCP bridge.

## Build from a checkout

The repository selects Rust 1.88.0 automatically when used with `rustup`.

```bash
make release       # optimized server and client binaries in target/release/
make test          # full Rust test suite
make integration   # end-to-end CLI suite
```

Run `make help` for the available local build, lint, formatting, and Docker targets.

## Quick start

Start a server with an optional initial session:

```bash
export CCP_ADMIN_KEY="<configured-admin-key>"
export CCP_CLIENT_KEY="<configured-client-key>"
ccp-server my-session
```

Create more sessions while the server is running. Client setup automatically connects every open topic:

```bash
export CCP_SERVER_URL=http://127.0.0.1:1338
curl -fsS -X POST "$CCP_SERVER_URL/v1/admin/sessions" \
  -H "X-CCP-Admin-Key: $CCP_ADMIN_KEY" -H "Content-Type: application/json" \
  -d '{"session_name":"second-session"}'
ccp-client subscribe-all --server "$CCP_SERVER_URL"
```

Create some structure and write data:

```bash
ccp-client add-shelf my-session notes "project notes"
ccp-client add-book my-session --shelf notes standup "daily standups"
ccp-client add-entry my-session --shelf notes --book standup day1 "first standup" "discussed sprint goals and blockers"
```

Search across everything:

```bash
ccp-client search-context my-session "sprint"
```

Read it back:

```bash
ccp-client get my-session day1 --shelf notes --book standup
```

Append more content to an existing entry:

```bash
ccp-client append my-session day1 --shelf notes --book standup "follow-up: resolved the blocker"
```

## How subscriptions work

The server defaults to the loopback plaintext HTTP endpoint `http://127.0.0.1:1338`; configure `CCP_HTTP_LISTENER_ADDR` and `CCP_HTTP_BASE_URL` for other deployments. It can host any number of sessions in one database. The management script creates/deletes sessions, while `subscribe` saves a chosen server/session pair locally. Every request carries its selected session IDs. This is request-local selection, rather than a persistent server subscription or a per-agent authorization role. Session IDs are never reused after deletion; subscriptions are stored by server and session identity. Qualify ambiguous selectors as `session@http://host:1338`.

The active transport does not use certificate enrollment, TLS, or certificate access roles. Configure client/admin keys explicitly; public topics accept ordinary requests without a client key, private requests require `X-CCP-Client-Key`, and management requires `X-CCP-Admin-Key`. See [server configuration](docs/server.md) for deployment and upgrade requirements.

## Docker

```bash
curl -fsSL https://raw.githubusercontent.com/squid-proxy-lovers/ccp/main/install.sh | bash -s -- --docker --session my-session
```

This builds the image and starts the container:

```bash
docker compose logs -f ccp-server
```

Create and subscribe to another session:

```bash
ccp-manage add another-session
ccp-client subscribe --server http://127.0.0.1:1338 another-session
```

Override the session or advertised host:

```bash
CCP_SESSION_NAME=prod CCP_ADVERTISE_HOST=192.168.1.50 docker compose up -d
```

Stop:

```bash
docker compose down
```

## CLI reference

After installing, `ccp-client` and `ccp-server` are available in your PATH.

### Read operations

- `ccp-client health <session>` check the selected server
- `ccp-client remote-sessions --server <http-url>` discover open topics
- `ccp-client subscribe --server <http-url> <session>` subscribe by name or id
- `ccp-client sessions` list saved subscriptions
- `ccp-client delete-session <session>` forget a session locally
- `ccp-client list <session>` list all entries
- `ccp-client get <session> <name>` fetch an entry
- `ccp-client get <session> <name> --shelf <shelf> --book <book>`
- `ccp-client history <session> <name>` view append history
- `ccp-client search-entries <session> <query>` search names and descriptions
- `ccp-client search-shelves <session> <query>`
- `ccp-client search-books <session> <query>`
- `ccp-client search-context <session> <query>` full-text in entry content
- `ccp-client search-deleted <session> [query]` filter archived entries, or list all with no query
- `ccp-client team-status <session> --team <shelf>` list active work in a challenge team
- `ccp-client search-team-status <session> --team <shelf> <query>` search agent names and work in a team
- `ccp-client brief-me <session>` session overview in one call (structure, recent entries, labels)
- `ccp-client get-entry-at <session> <name> --at <timestamp>` entry content at a point in time
- `ccp-client export <session>` export full session to stdout
- `ccp-client export <session> --output bundle.json`
- `ccp-client export <session> --shelf <shelf>` export one shelf
- `ccp-client export <session> --shelf <shelf> --book <book>` export one book
- `ccp-client export <session> --shelf <shelf> --book <book> --entry <name>` export specific entries
- `ccp-client export <session> --no-history` omit append history from bundle

### Write operations

- `ccp-client add-shelf <session> <shelf-name> <description>`
- `ccp-client add-book <session> --shelf <shelf> <book-name> <description>`
- `ccp-client add-entry <session> --shelf <shelf> --book <book> <name> <desc> <data>`
- `ccp-client append <session> <name> <content>`
- `ccp-client set-status <session> --team <shelf> --agent <name> <status>` join a team or update current work
- `ccp-client clear-status <session> --team <shelf> --agent <name>` clear current work and leave the team
- `ccp-client delete <session> <name>` soft-delete entry (archived)
- `ccp-client delete-shelf <session> <shelf-name>` remove a shelf and everything in it
- `ccp-client restore <session> <entry-key>`
- `ccp-client import <session> bundle.json`
- `ccp-client import <session> bundle.json --policy overwrite|skip|merge-history|error`

### Client setup

```bash
# macOS and Linux
export CCP_SERVER_URL=http://127.0.0.1:1338
export CCP_CLIENT_KEY="<configured-client-key>"
curl -fsS "$CCP_SERVER_URL/setup-client.sh" -o /tmp/ccp-setup.sh
sh /tmp/ccp-setup.sh

# Windows PowerShell
$env:CCP_SERVER_URL = "http://127.0.0.1:1338"
$env:CCP_CLIENT_KEY = "<configured-client-key>"
irm "$env:CCP_SERVER_URL/setup-client.ps1" | iex
```

The installers download the platform client, install the MCP bridge, and pass the configured endpoint/key and installed client path to Codex and Claude Code when their CLIs are present. Hosted scripts contain the advertised URL, and never embed configured server credentials. Keep the client key configured for subsequent CLI commands against private sessions.

### Management

The management script exposes only `add`, `delete`, and `stats`:

```bash
export CCP_SERVER_URL=http://127.0.0.1:1338
export CCP_ADMIN_KEY="<configured-admin-key>"
curl -fsS "$CCP_SERVER_URL/ccp-manage" -o ccp-manage
chmod +x ccp-manage
./ccp-manage add topic-name
./ccp-manage stats topic-name
./ccp-manage delete topic-name
```

## MCP tools

Run `bash install.sh --client` to set up the FastMCP bridge. Agents get tools for reading, searching, creating entries, appending content, and publishing temporary team status. Entry deletion, import, and restoration are CLI-only; server management uses the admin HTTP API, scripts, or dashboard. Certificate enrollment and revocation are outside the active HTTP/MCP workflow.

Challenge teams use existing shelves. An agent sets its status when starting work, updates it when the task changes, and clears it when finished; unrefreshed statuses expire after three hours so stale workers disappear automatically.

Agents automatically learn how to use CCP through the MCP instructions and the `ccp://help` resource. No extra prompting needed. See [mcp/README.md](mcp/README.md) for the full tool list and details.

## Data model

```text
Session
  └── Shelf (e.g. "research", "logs", "shared-context")
        └── Book (e.g. "findings", "errors", "agent-notes")
              └── Entry (e.g. "day1-summary")
                    ├── content (appendable text)
                    ├── description
                    ├── labels
                    └── history (who appended what, when, why)
```

Entries are the core unit. Each entry lives at a unique path: shelf/book/name. Content is append-only with full history tracking. Deleted entries are archived and can be restored.

## Access and management

Open topics are discoverable and subscribable by agents. Client setup forwards the configured shared key to the local MCP host. Management scripts require the admin key from the caller; the admin API also supports activity, statistics, and global/session instruction boards. The dashboard is served at `/admin`.

## Droplets

Droplets are shareable CCP bundles. You export a shelf, book, or entire session as a `.droplet` file and hand it to someone else. They import it into their own session and their agents have instant access to everything in it.

Think of it as packaged agent memory. Your team spent a week doing recon on an API. Export that shelf as a droplet. Another team imports it and their agents pick up where yours left off without re-doing the work.

Export a droplet:

```bash
# full session
ccp-client export my-session --output research.droplet

# just one shelf
ccp-client export my-session --shelf recon --output recon.droplet

# specific book
ccp-client export my-session --shelf recon --book endpoints --output endpoints.droplet

# without history (smaller file, just the content)
ccp-client export my-session --shelf recon --no-history --output recon-clean.droplet
```

Import a droplet:

```bash
# import into your session (fails if entries already exist)
ccp-client import my-session research.droplet

# overwrite existing entries
ccp-client import my-session research.droplet --policy overwrite

# skip entries that already exist
ccp-client import my-session research.droplet --policy skip

# merge history from the droplet into existing entries
ccp-client import my-session research.droplet --policy merge-history
```

Every droplet includes a SHA-256 hash over the entries. The server verifies it on import so you know the content hasn't been tampered with. See [docs/droplet-format.md](docs/droplet-format.md) for the full file format spec.

## Use cases

### CTF collaboration

Six of us played a 48-hour CTF. Everyone had their own agent. Solved challenges, recon'ed data, and exploit chains each got their own shelf. Someone found half a flag at 3am and dropped it into CCP. The rest of the team's agents picked it up through search and kept working with it. Nobody had to ping anyone on Discord.

### Multi-agent code review

We pointed three agents at a codebase: one for security audit, one for architecture review, one for test coverage gaps. Each wrote findings to CCP entries with labels like `severity:high` or `area:auth`. A fourth agent searched across all of them and produced a prioritized report. The whole thing ran in parallel because each agent had its own mTLS connection. There was no bottleneck.

### Persistent research across sessions

An agent spent two hours mapping out an API surface and wrote everything to CCP. Three days later we subscribed a new agent into the same session. It searched the old entries, found the endpoint map, and picked up where the first one left off. The data persists across server restarts so there wasn't a need for re-prompting and no context window issues.

### Two agents, one feature

Two people working on the same feature with separate Claude sessions. Both are enrolled in the same CCP session, so one agent makes notes about the approach it's taking, the other would be reading them before going off in a different direction. When one hits a roadblock, it writes what went wrong. The other sees it and skips that path entirely.

## Running tests

```bash
cargo test -p server --lib -- --test-threads=1
cargo test -p client --lib
cargo test -p protocol
```

## Architecture

See [docs/](docs/) for design documents:

- [docs/server.md](docs/server.md) HTTP server, sessions, management, and artifact hosting
- [docs/client.md](docs/client.md) subscriptions and cross-platform setup
- [docs/tool-call-api.md](docs/tool-call-api.md) MCP tool reference and response schemas
- [docs/droplet-format.md](docs/droplet-format.md) `.droplet` file format specification

## Benchmarks

The historical benchmark table predates the JSON-over-HTTP transport. Run the benchmark on your deployment for current numbers.

Below are some of the machines we benchmarked on:

- **M2**: Apple M2, 8GB RAM, macOS
- **EPYC**: AMD EPYC 12-core, 48GB RAM, Linux (cloud VPS)

| Operation | M2 (macOS) | EPYC 12-core (Linux) | P50 (M2) | P50 (EPYC) |
| --- | --- | --- | --- | --- |
| list entries | 36,714 req/s | 65,451 req/s | 0.33ms | 0.23ms |
| get entry | 44,520 req/s | 33,993 req/s | 0.26ms | 0.36ms |
| search (simple) | 34,091 req/s | 47,301 req/s | 0.37ms | 0.29ms |
| search (complex) | 32,393 req/s | 42,844 req/s | 0.38ms | 0.31ms |
| search (miss) | 44,783 req/s | 59,543 req/s | 0.19ms | 0.24ms |
| context search (simple) | 33,287 req/s | 51,201 req/s | 0.38ms | 0.27ms |
| context search (complex) | 28,308 req/s | 51,262 req/s | 0.44ms | 0.29ms |
| context search (miss) | 44,300 req/s | 66,111 req/s | 0.22ms | 0.22ms |
| append | 28,828 req/s | 19,285 req/s | 0.25ms | 0.80ms |
| mixed (all ops) | 2,695 req/s | 2,678 req/s | 3.64ms | 1.19ms |

Run your own: `cargo run --release -p ccp-tests --bin benchmark -- --mode full-suite`

## How CCP compares to MemPalace

This section was added to clear up confusion on a different yet quite popular project, MemPalace. Initially, we found out about MemPalace when it came out on April 5th on Twitter/X, however, CCP has been in development since March 11th. The key thing here is they're built for different things.

MemPalace is single-agent memory for one AI recalling past conversations. CCP is multi-agent coordination: multiple agents subscribe to topics on a shared network server and exchange structured, searchable context.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

For security vulnerabilities, see [SECURITY.md](SECURITY.md).

## Maintainers

- Vipin <vipin@spl.team>
- Tanush <dudcom@spl.team>
- General: <oss@spl.team>

## License

AGPL-3.0-or-later. See [LICENSE](LICENSE).
