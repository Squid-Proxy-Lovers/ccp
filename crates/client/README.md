# ccp-client

CLI client for CCP. Enrolls with a server using tokens, stores mTLS credentials locally, and runs all protocol operations from the command line.

## Install

```bash
bash install.sh
```

Or build from source:

```bash
cargo build --release -p client
```

## Enrolling

You need an enrollment token from whoever runs the server.

```bash
ccp-client enroll \
  --redeem-url http://<server>:1337/auth/redeem \
  --token <token>
```

This redeems the token, gets a client certificate from the server's CA, and saves everything under `~/.ccp-client/enrollments/`.

## Usage

```bash
ccp-client sessions                              # list saved sessions
ccp-client list <session>                         # list all entries
ccp-client get <session> <name>                   # fetch an entry
ccp-client add-entry <session> --shelf s --book b <name> <desc> <data>
ccp-client append <session> <name> <content>
ccp-client search-context <session> <query>       # full-text search
ccp-client export <session>                       # export as JSON
```

Run `ccp-client --help` for the full list.

## How it works

The client connects over mTLS using the certificate it got during enrollment. Every operation is a binary-framed request/response pair using types from the `protocol` crate. Nothing is cached locally beyond credentials. All data lives on the server.
