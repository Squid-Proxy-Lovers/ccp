# Renew the instance server certificate

This unreleased local command renews the server's TLS certificate and private
key while preserving its exact instance CA. It can also explicitly change the
DNS/IP names served by that certificate. It does not start a listener or install
trust in a client, browser or OS.

[Documentation index](README.md) · [Instance TLS setup](instance-tls-identity.md)

## Renew or change names

Stop the server and keep a complete protected backup. Run on the trusted host
with the same data directory used for initial setup:

```bash
ccp-server workspace-tls-renew --data-dir ./ccp-data
ccp-server workspace-tls-info --data-dir ./ccp-data
```

From a checkout, replace `ccp-server` with `cargo run -p server --`. Without any
`--name` flags, renewal keeps the selected certificate's names. To replace the
complete name list explicitly:

```bash
ccp-server workspace-tls-renew --data-dir ./ccp-data \
  --name ccp.example.com --name localhost --name 127.0.0.1
```

Names follow the existing bounded DNS/IP validation. Supply every name clients
will use; omitted names disappear from the new certificate. The command takes
exclusive maintenance access and refuses while a native runtime, workspace
runtime, retained TLS configuration or connection holds the directory. It never
stops a process automatically. Restart using the renewed selected identity.

Renewal returns public trust JSON only. The instance UUID, CA certificate,
CA private key and CA fingerprint remain identical. A fresh server key and
ServerAuth-only certificate replace the server leaf, for up to 365 days and
never beyond the CA's expiry. Existing client certificates, workspace/admin
identities, keys, grants, closed enrollment and runtime mode remain unchanged.
Clients that trust the CA and use a retained valid name can verify the renewed
certificate. A separately pinned server-leaf fingerprint needs an explicit update.

Newly issued workspace server and client certificates include the issuer's key
identifier for strict X.509 verification. If an earlier draft generated your
server certificate without that extension, run the renewal command above and
restart the server. This keeps the same CA and instance identity. Existing client
certificates gain the extension when renewed or replaced through authorized
recovery; installing new software alone does not rewrite stored certificates.

An expired server leaf can be renewed if the pinned bundle is otherwise valid
and the CA is currently valid. The old certificate/key/profile/chain are checked
at the leaf's valid historical interval; CA validity is also checked at the
actual renewal time. A future-dated leaf, invalid key, corrupt bundle, missing
pin or expired CA refuses renewal. This command cannot rotate or repair an
expired/compromised CA and does not revoke earlier server certificates. Those
certificates remain cryptographically valid through their own validity windows.

## Interrupted renewal and recovery

The database pin selects one exact private bundle hash. Renewal writes and
synchronizes a complete immutable generation before committing its new pin and
local-operator audit together with FULL SQLite synchronization. On Unix, bundle
directory entries are also synchronized. Unsupported hard links fail closed;
non-Unix directory synchronization and ACL limitations remain as documented in
[initial setup](instance-tls-identity.md#storage-interruption-and-backup).

If publication, the pin update or the audit fails, the previous pinned identity
remains selected. A complete but unreferenced private generation may remain;
it is never discovered or automatically activated. If only the response is
lost, inspect `workspace-tls-info`: it reads the committed selection. Running
renewal again is another explicit server-key renewal, not an idempotent receipt
replay.

A missing or corrupt selected generation refuses loading, setup and renewal.
The server never selects an older valid bundle or another unreferenced file to
recover automatically. Restore the exact selected generation from the complete
matching backup. Do not edit the database pin or delete files to force a new CA.

## Disk layout and version compatibility

Schema 14 records the generation-selection contract; schema 13's public pin
columns stay unchanged. Private bundle format 1 and native wire protocol 1 stay
unchanged. Explicit renewal upgrades supported older databases before the
operation; a later refusal may still leave that normal schema upgrade committed.
The info command can read the original schema-13 layout without upgrading it.
Older schema-aware servers refuse schema 14; use matching whole backups and
producing software versions for rollback.

The original `ccp-instance-tls.json` remains untouched. Renewals publish
`ccp-instance-tls-SHA256.json`, where `SHA256` is the selected bundle's lowercase
64-character hash. Loaders use only the pinned filename. They accept the original
file as the initial-layout fallback only when its bytes match that exact pin.
There is no directory scan, newest-file rule or stale fallback.

All generations contain CA and server private keys and must remain protected.
Unix files start with mode `0600`, and loading applies the same bounded regular
file/permission checks to every generation. Old and unreferenced generations are
retained; automatic private-material cleanup is not included. Include the
selected generation with its matching database/WAL, mode ledger, journal and
other private material in protected backups. Keep retained historical copies
private as well.
