# ccp-server

The CCP server hosts multiple sessions in one SQLite database and serves HTTP/JSON on one listener. Operations include shelf/book/entry CRUD, append/search, deleted-entry restoration, import/export, history, team status, and instruction boards. The dashboard and administration API use the same listener.

```sh
cargo build --locked --release -p server
./target/release/server my-session
```

Omit the session argument to start without creating a topic. To create a topic offline while the server is stopped, use `server create-session <session-name>`; running clients discover and subscribe with the client CLI.

See [server deployment and API documentation](../../docs/server.md) for HTTP routes, configuration, keys, Docker, and downloadable artifacts. Certificate enrollment, `issue-token`, and `server health` are absent from the current CLI.
