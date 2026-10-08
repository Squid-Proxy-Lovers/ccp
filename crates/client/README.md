# CCP client

The CLI and Rust library discover public sessions, save local subscriptions, and send JSON requests over plaintext HTTP. Server data is not cached locally.

## Build and install

```sh
cargo build --release -p client
bash install.sh
```

Cargo produces `target/release/client`; installation names the executable `ccp-client`.

## Connect and use

```sh
export CCP_SERVER_URL=http://127.0.0.1:1338
ccp-client remote-sessions
ccp-client subscribe <session-name-or-id>
ccp-client subscribe-all
ccp-client sessions
ccp-client health <session>
ccp-client master-instructions <session>
ccp-client list <session>
ccp-client get <session> <name> --shelf s --book b
ccp-client add-entry <session> --shelf s --book b <name> <description> <data>
ccp-client append <session> <name> --shelf s --book b -- <content>
ccp-client search-context <session> -- <query>
ccp-client export <session> --output context.droplet
```

Run `ccp-client --help` and individual command help for all commands. Use `--` after options when trailing content starts with a dash.

Discovery commands accept `--server`; otherwise they use `CCP_SERVER_URL`, then `http://127.0.0.1:1338`. Base paths are supported. URLs must use `http://` and contain no credentials, query, or fragment. Requests have a 10-second connection limit and 60-second overall limit; redirects are rejected.

Subscriptions live in `~/.ccp-client/enrollments/` (override with `CCP_CLIENT_HOME`). Each record identifies a server and session ID. Existing metadata remains readable. Selection accepts a session name or ID and uses `CCP_SERVER_URL` when configured. If a selector matches distinct sessions, selection fails with an ambiguity error. An explicit selector such as `42@http://localhost:1338` selects that server regardless of the environment. `delete-session` removes matching local records; it does not delete server data, and ambiguous selectors fail without removing records.

Public discovery and public-session requests are open. For private-session requests, configure `CCP_CLIENT_KEY` to match the server. There is no certificate enrollment in the active HTTP client. Legacy certificate-shaped metadata fields remain for local compatibility.

## Rust library

Depend on `client` via a path dependency, construct `CcpClient`, and call `subscribe(server_url, selector)`, `session(selector)`, or `writable_session(selector)`. `SessionClient` provides typed entry/library CRUD, searches, history, and bundle transfer. Mutations reject read-only saved subscriptions even if selected with `session`. The convenience API covers fewer operations than the CLI; the shared `protocol` crate contains the full wire types.

`add_entry` preserves its existing return type. Use `add_entry_with_warning` to receive the optional duplicate-entry warning along with the added entry. CLI warnings go to stderr while stdout remains the entry JSON.

See [the client guide](../../docs/client.md) for hosted installation and MCP setup.
