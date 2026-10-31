# Provision instance TLS identity

This unreleased local setup step creates persistent certificates for the workspace
redesign. It does not start a listener, activate workspace mode, install trust in
your OS/browser, or change native session certificates. The limited [workspace HTTPS API](workspace-http-api.md) can use this material;
[client trust and profile commands](workspace-client.md) configure connections
using that identity. Internal
[registered-peer adapters](registered-tls-peers.md) bind actual TLS peers to
current credential records.

[Documentation index](README.md) · [Local administration](local-workspace-administration.md)

## Create and inspect certificates

Stop the server and run from the host that controls its data directory. Complete
[local workspace setup](local-workspace-administration.md) first. With installed
binaries, supply every DNS name or IP address clients will use:

```bash
ccp-server workspace-tls-init --data-dir ./ccp-data \
  --name localhost --name 127.0.0.1 --name ::1 --name ccp.example.com
ccp-server workspace-tls-info --data-dir ./ccp-data
```

From a checkout, use `cargo run -p server --` in place of `ccp-server`. Both
commands require exclusive maintenance access; they refuse while a native or
workspace runtime holds the directory. They never stop a process automatically.
The info command opens the data database read-only without upgrading its schema;
the separate coordination ledger may reconcile mode and advance its revision.

Supply 1–32 explicit ASCII DNS/IP names. DNS names are lowercased, IP addresses
canonicalized, and duplicates removed. URLs, wildcards, trailing dots, brackets
around IPv6, and empty labels are refused. No names come from environment
variables, session names, or automatic hostname discovery. Rerunning setup with
the same normalized names returns the same identity. Different names require the explicit [renewal command](server-certificate-renewal.md);
setup never replaces them.

The JSON output contains only the instance UUID, CA certificate PEM, CA and server
certificate SHA-256 fingerprints, configured names and expiration timestamps.
Private keys and admin keys are not output. A new CA lasts 3,650 days and the
server certificate 365 days, with a 60-second clock allowance at issuance.
Loading rejects certificates outside their validity interval. Use the explicit
[server certificate renewal](server-certificate-renewal.md) command for expiry or
name changes; setup never renews or rotates automatically.

## Establish trust before sending secrets

Convey the CA certificate or its SHA-256 fingerprint through an independently
trusted channel. A client must verify its intended instance, the CA and the
server DNS/IP name before transmitting an invitation or admin key. Fetching a
fingerprint from an unverified remote server does not establish trust. This local
export does not yet configure clients or browser trust, and never disables TLS
verification to bootstrap enrollment.

The internal HTTPS configuration uses server authentication. Its separate mTLS
configuration requires a CA-issued client certificate. Both negotiate
HTTP/1.1. Cryptographic certificate validation alone does not establish a
registered CCP identity: request handlers must use the [registered-peer adapters](registered-tls-peers.md)
to bind the actual TLS peer certificate to its current instance/client/credential
records and permissions.
No endpoint exposes the private signing material.

## Storage, interruption and backup

Schema 13 introduced `ccp_instance_tls`, a public pin for the instance UUID, private
bundle hash and CA certificate fingerprint. Existing schema-12 roles, keys,
grants, enrollment and mode state remain unchanged; earlier supported schemas
follow the [normal upgrade rules](workspace-foundation.md#database-upgrade).
Schema 14 adds immutable renewal generations selected by that pin; the
[renewal guide](server-certificate-renewal.md#disk-layout-and-version-compatibility)
describes compatibility. The native wire protocol remains version 1. Older schema-aware servers refuse
schema 14; host operators must stop older unaware binaries and use a complete
matching backup for rollback.

The initial `ccp-instance-tls.json` and later content-addressed renewal generations
use private disk format 1. It contains the instance UUID,
names, CA certificate/key and server certificate/key together. The CA is
independent of every native session's `ccp_ca_cert.pem` and `ccp_ca_key.pem`.
The bundle is created with mode `0600` on Unix; loading refuses group/other
permissions, symlinks and nonregular or oversized files. Windows operators must
restrict the data directory with suitable ACLs; this code does not install ACLs.
The host operator and filesystem are trusted.

Setup writes a random private staging file in the same directory, synchronizes
its contents, then publishes it through a hard link that cannot overwrite an
existing destination. Unix also synchronizes directory entries. Filesystems
without hard-link support refuse publication. Non-Unix directory synchronization
is not provided, so power-loss durability depends on the platform/filesystem.
A crash may leave a private `.ccp-instance-tls-*.tmp` file; setup never discovers
staging files as the active identity.

Only after publication does setup commit the public pin and a local-operator
audit with full SQLite synchronization. If that transaction or the response is
lost, rerun explicit setup with the same names to validate and pin the same
bundle. Existing pin plus missing bundle, changed bytes, foreign instance UUID,
invalid certificates or mismatched keys fail closed. Setup does not recover by
generating another CA, and a missing selected renewal generation does not fall
back to the original file unless its bytes match the exact current pin. Restore the complete matching backup; do not delete the
pin or edit identity files to force startup.

Back up the private bundle together with the matching data database and any WAL,
mode ledger, journals and other private material under the existing protected
[backup rules](workspace-runtime-mode.md#storage-and-compatibility). Treat backup copies
and any crash staging files as private signing material. Native session files,
workspace owners, roles, keys, grants and closed enrollment windows are preserved.

Implementation references: Rust's [hard-link publication semantics](https://doc.rust-lang.org/std/fs/fn.hard_link.html),
[file synchronization](https://doc.rust-lang.org/std/fs/struct.File.html#method.sync_all),
and Rustls [server verification](https://docs.rs/rustls/0.23.46/rustls/client/struct.WebPkiServerVerifier.html)
and [mandatory client-certificate verification](https://docs.rs/rustls/0.23.46/rustls/server/struct.WebPkiClientVerifier.html).
