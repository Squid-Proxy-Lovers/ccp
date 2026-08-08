# CCP client

The Rust client talks to the server using JSON over plaintext HTTP. The deployed endpoint is `http://192.168.130.34:1338`.

## Install

macOS and Linux:

```sh
curl -fsSL http://192.168.130.34:1338/setup-client.sh | sh
```

Windows PowerShell:

```powershell
irm http://192.168.130.34:1338/setup-client.ps1 | iex
```

The installer detects the OS/CPU, downloads the matching Rust client, installs the MCP bridge in an isolated Python virtual environment, and configures Codex and Claude Code when installed.

## Topics and subscriptions

```sh
ccp-client remote-sessions --server http://192.168.130.34:1338
ccp-client subscribe --server http://192.168.130.34:1338 <topic-name-or-id>
ccp-client sessions
```

Only public sessions appear in remote discovery. A saved subscription contains the session metadata and HTTP endpoint. Normal operations select one of these saved topics by name or ID.

## Authentication

There is no TLS, certificate enrollment, or per-client certificate. The setup scripts embed the shared client API key and server URL. Public topics can be discovered and selected openly. The separate admin key is never installed by the client setup script.

## Codex and Claude Code

The local MCP bridge exposes `open_topics` and `subscribe` so an agent can select an open topic itself. It also exposes the existing entry, shelf, book, search, export, and import tools.
