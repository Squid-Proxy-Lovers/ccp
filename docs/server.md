# CCP server

One server process hosts multiple sessions in one SQLite database and serves a plaintext JSON HTTP API.

## Deployment defaults

- public URL: `http://192.168.130.34:1338`
- listener: `0.0.0.0:1338`
- data directory: `data` or `CCP_SERVER_DATA_DIR`
- downloadable artifacts: `downloads` or `CCP_DOWNLOAD_DIR`

Start without or with an initial topic:

```sh
ccp-server
ccp-server initial-topic
```

## HTTP routes

- `GET /health`
- `GET /v1/sessions` — list open topics
- `POST /v1/subscribe` — resolve an open topic
- `POST /v1/request` — execute a protocol operation for explicitly subscribed session IDs
- `POST /v1/admin/sessions` — add a session
- `DELETE /v1/admin/sessions/{session}` — delete a session
- `GET /v1/admin/sessions/{session}/stats` — get session statistics
- `GET /setup-client.sh` and `GET /setup-client.ps1`
- `GET /ccp-manage` and `GET /ccp-manage.ps1`
- `GET /downloads/{artifact}`

## Keys

HTTP is intentionally plaintext. A shared client key is embedded in the client and setup scripts. A separate admin key protects the three management operations. Override them with `CCP_CLIENT_KEY` and `CCP_ADMIN_KEY`; if changed, regenerate the embedded scripts/binaries together.

## Management

The management scripts intentionally implement only:

```text
add SESSION
delete SESSION
stats SESSION
```

## Artifact publishing

Run `scripts/build-downloads.sh` to produce the current platform server/client binaries and the MCP source distribution in `downloads/`. Cross-compile or CI-build the other platform filenames expected by the installers:

```text
ccp-client-darwin-aarch64
ccp-client-darwin-x86_64
ccp-client-linux-aarch64
ccp-client-linux-x86_64
ccp-client-windows-x86_64.exe
ccp-mcp.tar.gz
```
