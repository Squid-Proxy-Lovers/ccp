# CCP server

One process hosts multiple sessions in SQLite and serves a plaintext HTTP/JSON API. Start with `server` or `server initial-topic`; installed release binaries use the name `ccp-server`.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `CCP_HTTP_LISTENER_ADDR` | `127.0.0.1:1338` | HTTP bind address |
| `CCP_HTTP_BASE_URL` | `http://127.0.0.1:1338` | Advertised URL and hosted-script default |
| `CCP_SERVER_DATA_DIR` | `data` | SQLite, journal, and session storage |
| `CCP_DOWNLOAD_DIR` | `downloads` | Directory served by the artifact route |
| `CCP_CLIENT_KEY` | Built-in compatibility key | Shared access key for private session requests |
| `CCP_ADMIN_KEY` | Built-in compatibility key | Key for every administration API route |

Configure deployment keys explicitly. Public session operations are available to subscribers without a client key; keys are carried over plaintext HTTP. Restrict the listener/network or use a trusted local forwarding tunnel; native clients accept only `http://` URLs. HTTPS-aware callers can use a TLS reverse proxy in front of the server. Current transport does not enroll certificates or enforce separate certificate-based read/write roles.

The Docker entrypoint binds `0.0.0.0:${CCP_HTTP_PORT:-1338}` inside the container. Compose publishes on host loopback by default. Set `CCP_HTTP_BASE_URL` to the address reachable by clients and `CCP_PUBLISH_HOST` for the intended host interface. `compose.sdcl.yml` uses the same HTTP transport, requires configured keys, and probes `/health`.

## HTTP routes

| Route | Purpose |
| --- | --- |
| `GET /health` | Process health |
| `GET /v1/sessions` | Discover public topics |
| `POST /v1/subscribe` | Resolve a public topic |
| `POST /v1/request` | Execute a protocol operation with subscribed session IDs |
| `POST /v1/admin/sessions` | Create a session |
| `DELETE /v1/admin/sessions/{session}` | Delete a session |
| `GET /v1/admin/overview` | Session statistics and global instructions |
| `GET /v1/admin/sessions/{session}/stats` | Session statistics |
| `GET /v1/admin/activity?session={session}&limit=100` | Recent activity |
| `GET|PUT /v1/admin/master` | Global master instructions |
| `GET|PUT /v1/admin/sessions/{session}/master` | Session master instructions |
| `GET /admin` | Management dashboard; API requires admin key |
| `GET /setup-client.sh`, `GET /setup-client.ps1` | Client installers |
| `GET /ccp-manage`, `GET /ccp-manage.ps1` | Management scripts |
| `GET /ccp-update`, `GET /ccp-update.ps1` | Client updaters |
| `GET /downloads/{artifact}` | Published files |

Admin requests carry `X-CCP-Admin-Key`. Client keys use `X-CCP-Client-Key`. URL-encode session selectors in route paths. A subscription is local metadata, supplied with each request; it is not a persistent server connection.

## Hosted client setup

The server supplies its configured base URL in hosted scripts. `CCP_SERVER_URL` overrides that URL; scripts require `CCP_CLIENT_KEY` from the caller and do not publish server credentials. Unix setup requires curl and Python 3; Windows setup requires the Python launcher (`py`) and supports x86_64.

```sh
export CCP_SERVER_URL=http://127.0.0.1:1338
export CCP_CLIENT_KEY='<configured-client-key>'
curl -fsS "$CCP_SERVER_URL/setup-client.sh" -o /tmp/ccp-setup.sh
sh /tmp/ccp-setup.sh
```

Setup installs the client and updater in `CCP_INSTALL_DIR` or `~/.local/bin`, subscribes to open topics, installs the MCP package, and configures Codex/Claude when present. Their MCP configuration receives the exact installed client path, endpoint, and client key. Keep `CCP_CLIENT_KEY` in the environment when running CLI commands or updates against a server with a custom key. Restart MCP hosts after updating.

## Management

Set `CCP_SERVER_URL` and `CCP_ADMIN_KEY`, then run:

```text
ccp-manage add SESSION
ccp-manage delete SESSION
ccp-manage stats SESSION
```

With no arguments, the script opens `/admin`. Enter the admin key in the dashboard to load sessions, activity, and instruction boards.

## Artifact publishing

Run `scripts/build-downloads.sh` to build the native client/server and a fresh MCP source distribution. Other platform binaries require the corresponding CI artifacts. Generated binaries are revision-dependent; `downloads/provenance.json` records the verified source revision, CI run and hashes for checked-in downloads. Refresh the relevant files and provenance after changing source. Local builds do not automatically refresh provenance for other platforms.

Tagged CI releases include Unix server/client binaries, a Windows x86_64 client, and `ccp-mcp.tar.gz`. The separate client-artifacts workflow also produces workflow artifacts for manual deployment. Download the matching release/workflow artifacts into `CCP_DOWNLOAD_DIR` to refresh a live server; GitHub uploads do not update its mounted directory automatically.

Hosted filenames:

```text
ccp-client-darwin-aarch64
ccp-client-darwin-x86_64
ccp-client-linux-aarch64
ccp-client-linux-x86_64
ccp-client-windows-x86_64.exe
ccp-mcp.tar.gz
```

## Recovery and upgrades

Stop the producing server cleanly and back up its database and journal before upgrading. Databases with a newer schema are rejected. A legacy headerless journal alongside persisted library data has no trustworthy snapshot boundary; startup refuses that ambiguous state before changing the schema. Recover with the version that wrote the journal and shut it down cleanly before retrying the upgrade.

Schema 4 records a journal generation and byte offset in the same SQLite transaction as each full snapshot. Recovery replays only subsequent records; checkpoint rotation preserves that boundary across restarts. Session IDs are allocated transactionally and never reused after deletion. Only one server process should use a data directory at a time.

Ordinary mutations acknowledge journal queue acceptance, rather than an individual disk sync. Graceful SIGINT/SIGTERM drains and syncs the journal and checkpoints the full state. The mutation/checkpoint gate serializes writes to keep snapshots and rollback consistent; the benchmark suite verifies workloads, but no production capacity guarantee is inferred.
