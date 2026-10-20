# Enroll a workspace client

This unreleased client supports trusted workspace profiles, initial enrollment
and paged workspace discovery over HTTPS/mTLS. It also accepts a fresh targeted
readmission invitation into an unenrolled profile. Joining another workspace
with an active registration, renewal, recovery, context selection and MCP
integration remain subsequent work. Existing native commands and their saved
session files keep their current behavior.

[Documentation index](README.md) · [Server HTTPS setup](workspace-http-api.md) ·
[Enrollment contract](workspace-enrollment.md)

## Establish server trust first

Obtain the instance UUID and public CA certificate from the host operator through
a trusted channel. Verify the CA file's SHA-256 fingerprint against the operator's
public trust output before sending an invitation. Do not download trust from an
invitation's unverified server response or install the CA as an OS/browser root.
The operator must already have started the workspace HTTPS and mTLS listeners.

Choose an explicit private profile directory. Replace the UUID and origins below
with the operator's actual values:

```bash
ccp-client workspace trust \
  --profile "$HOME/.ccp-client/workspaces/office" \
  --instance-id 00000000-0000-4000-8000-000000000001 \
  --https-origin https://ccp.example:8443 \
  --mtls-origin https://ccp.example:8444 \
  --ca-file ./ccp-ca.pem
```

The origins must use HTTPS, with no credentials, non-root path, query or fragment.
The client uses only the supplied root, verifies the server name and checks the
intended instance UUID through API version 1 health before publishing the profile.
It disables proxies, redirects, early data and plaintext fallback. HTTP/1.1 is
explicit. On Unix a new profile directory is 0700 and its SQLite file is 0600;
an existing directory/file with group or other access is refused. Windows uses
inherited ACLs: choose a local directory accessible only to your account. This
implementation does not inspect or install Windows ACLs.

Trust setup prints public profile metadata, including `ca_sha256`. An existing
profile is never replaced, even when setup is repeated with different settings.
There is no implicit default workspace profile or fallback to a native session.
`CCP_CLIENT_HOME` does not override the explicit workspace profile path.

## Redeem an invitation

Ask the workspace owner/admin for a current initial or individual invitation.
Store it in an owner-only regular file using a private editor or secure transfer;
do not put the bearer value in command-line arguments. On Unix, create an empty
private file before entering the invitation:

```bash
install -m 600 /dev/null ./workspace-invitation.txt
```

After saving the invitation in that file:

```bash
ccp-client workspace enroll \
  --profile "$HOME/.ccp-client/workspaces/office" \
  --invitation-file ./workspace-invitation.txt
```

The client durably saves a fresh request UUID, private key and signed CSR before
sending the enrollment request. It stores an invitation hash, not the bearer
value. It rechecks HTTPS health and the intended instance before sending that
value or CSR. The response must match the pinned CA, instance UUID, pending key,
client/credential UUID URI identities, serial, validity and ClientAuth-only
certificate profile. Only then does the client atomically activate it and clear
the pending request. No private key or invitation is printed in metadata/errors.

After a command error, inspect `workspace info` first. If a client UUID is already
present, activation committed and the saved profile can be used. Otherwise, if
a response was lost or activation failed, retain the profile and original
invitation file and run the same enrollment command again. The same UUID, CSR
and key retrieve the original committed public certificate without another
registration or quota use. A different invitation cannot silently replace a
pending request. If that invitation is no longer usable, keep the directory and
ask the administrator to inspect the admission; do not discard its pending key
on the assumption that enrollment failed at the server.

An enrolled profile refuses another initial enrollment. Reuse its registration
for later workspace access; do not create another registration merely to join a
second workspace on the same instance. The join command remains subsequent work.
A separately authorized [targeted readmission](workspace-readmission.md) can use
an otherwise unenrolled profile with a fresh key; the server captures the
existing client identity and creates a new scoped grant.

## Inspect and discover

```bash
ccp-client workspace info --profile "$HOME/.ccp-client/workspaces/office"
ccp-client workspace workspaces --profile "$HOME/.ccp-client/workspaces/office"
```

Info is local metadata: format, pinned origins/instance/CA fingerprint, pending
request UUID and optional client/credential UUIDs/expiry. It can show expired
credentials as expired, but is not proof of current server authorization.
Online discovery revalidates the certificate and uses actual mTLS; the server
checks current disable, credential and grant state. A denial never triggers
implicit registration or a trust downgrade.

Discovery returns `workspaces` and optional `next_cursor`. The default limit is
100, maximum 500. Pass `--limit` and the returned opaque string as `--after` for
the next page. Listings expose only workspaces authorized to that credential.
They do not select workspaces for routine context reads. Each page is a current
snapshot; concurrent changes can change ordering. Response bodies are limited
to 1 MiB, connect timeout 5s and total request/body timeout 20s. Errors include only
local descriptions and HTTP status, without echoing an untrusted server body.

## Profile storage and recovery limits

Workspace profile format 1 uses `workspace-profile.sqlite3` with a separate
application ID, DELETE journaling and FULL synchronization. Open profiles hold
an exclusive SQLite lease across commits and network calls. Another invocation
can fail busy after 2s; close the first profile before retrying. This serializes
profile changes without stale PID lock files. Missing, corrupt, foreign or future
formats fail closed; they do not regenerate keys or trust.

The private key is stored locally in the owner-protected SQLite file. It is not
encrypted by a separate keychain, and key erasure from free pages/backups is not
promised. Stop clients and close every profile lease before copying the entire
private profile directory for backup. Keep backups private. The file system
must support SQLite locks, hard links and the required local file operations.
Host operators remain trusted against path/file replacement races. Unix
publication syncs the directory; Windows directory-sync and ACL behavior have
not been independently verified locally.

Automatic renewal and recovery commands are not connected in this batch. A
client whose certificate has expired or been revoked cannot use that old
certificate to regain access. The selected [fresh targeted recovery](workspace-recovery.md)
workflow remains separate integration work. This draft is not a production
rollout or a throughput/power-loss guarantee.
