# Set up or recover workspace administration locally

These unreleased server commands run on the CCP host with write access to its
data directory. Stop every server using that directory first: setup, inspection
and reset take an exclusive maintenance lease. They do not stop processes for
you. The [workspace HTTPS server](workspace-http-api.md) uses this setup;
browser login remains subsequent work.
These commands provision its administrative foundation without starting a
listener, activating workspace mode or generating a CA.

## First instance setup

From a checkout, use `target/release/server`; an installed binary is named
`ccp-server`. Choose the intended directory explicitly:

```bash
ccp-server workspace-init --data-dir ./ccp-data --operator 'Alice' --workspace 'Main'
```

The first operator becomes an instance administrator and the first workspace's
single owner. The transaction creates that principal, role, workspace, two
independently random key verifiers and local-operator audits together. The
response returns the instance, workspace and principal UUIDs plus an
`instance_admin_key` and a `workspace_owner_key`, each with its key ID and secret.
Save the secrets securely when returned; the database retains only verifiers.
They are valid until revoked by default.

The scopes remain separate even though the principal is the same. An instance
key exercises global powers; it cannot use the principal's owner role. The
owner key exercises only its workspace powers. See
[scoped admin keys](workspace-admin-keys.md).

Any existing global-admin record prevents a second setup, including a disabled
one. A lost response is handled through inspection and reset below. Setup on
supported native data preserves its sessions and existing records; it creates a
new owned workspace without granting authority from legacy owner display names
or automatically assigning owners to old workspace mappings.

## Find the existing identities

```bash
ccp-server admin-info --data-dir ./ccp-data
```

This returns the immutable instance UUID, current runtime mode, global-admin
principal IDs/names/enabled state and workspace IDs/names/state/current owner IDs.
It does not return key secrets, verifiers or stored content. It reads the data
schema without upgrading it or creating a missing instance. A native database
without the workspace foundation needs explicit first setup instead.

Names help you recognize records; the UUID is the reset target. Replace
`INSTANCE_ADMIN_UUID` and `WORKSPACE_UUID` below with IDs from this response.

## Replace lost keys

Replace all instance-scope keys belonging to one existing enabled global admin:

```bash
ccp-server reset-instance-key --data-dir ./ccp-data --principal INSTANCE_ADMIN_UUID
```

Replace the current owner's keys only in one workspace:

```bash
ccp-server reset-owner-key --data-dir ./ccp-data --workspace WORKSPACE_UUID
```

The owner is captured from the current workspace record, not a name or supplied
principal claim. Active and archived workspaces are supported; purged, unowned
or missing workspaces are refused. Disabled principals and targets without the
required current global/owner role are refused. Reset does not enable an account,
appoint or transfer an owner, or add a global role.

Each operation atomically revokes the selected principal's nonrevoked keys in
that exact scope, records each revocation, issues a replacement and audits the
local operator. It returns the new `secret`, key/principal/workspace IDs,
`expires_at` and count of revoked keys. Randomness, database or audit
failure leaves old keys intact. Save the replacement when returned; secret
responses are not stored for later retrieval. If output is lost, inspection and
another explicit scoped reset recover access without creating a new principal.

Other administrators' keys, the same principal's other scopes, owner/team
records, data, client credentials/grants, archive state and closed enrollment
remain intact. Pending invitations still recheck their original issuer keys;
revoked issuer keys no longer authorize them. Reset never reopens bulk admission
or re-enrolls clients. Existing authorized client grants remain separate from
administrative key validity.

A replacement defaults to no expiry. To set one, add a positive TTL:

```bash
ccp-server reset-owner-key --data-dir ./ccp-data --workspace WORKSPACE_UUID --ttl-seconds 3600
```

Zero or overflowing TTLs are refused. Browser-session lifetime is separate from
underlying key validity; browser authentication is not implemented by these
commands.

## Storage and scope limits

These operations preserve the
[runtime mode markers](workspace-runtime-mode.md), including workspace activation.
They do not clear data, role restrictions or activation to regain access.
Unsupported/HTTP-draft schemas require the
[existing conversion and backup rules](workspace-foundation.md#database-upgrade).
Keep a complete protected backup and matching producing version.

The library's `local_admin` wrappers enforce exclusive directory maintenance;
the underlying access-store methods are trusted building blocks and must never
be wired to first-visitor signup or remote reset endpoints. A global bearer key
alone does not authorize these host operations. Additional-workspace first-owner
onboarding, client TLS, the browser panel and content migration remain separate
implementation work.

After administration setup, [provision instance TLS identity](instance-tls-identity.md)
with explicitly chosen server names. That is a separate local operation.
