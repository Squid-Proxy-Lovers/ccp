# Permanently delete an archived workspace

Unreleased workspace API1, server schema24. The desktop deletion flow is not yet
implemented. [Archive/reactivate](workspace-http-api.md) preserves context for
reading; permanent deletion is a separate, irreversible action available only to
that workspace's owner or an instance administrator. Workspace admins and clients
cannot delete a workspace.

## Prepare and confirm

Use the HTTPS admin listener with a current Bearer admin key, or a
[browser session](workspace-browser-session.md) with its exact Origin and CSRF
header. These routes accept strict JSON POST bodies; the registered-client mTLS
listener does not serve them. Target IDs never grant authority.

1. Archive the workspace using `/api/v1/admin/workspaces/archive`.
2. POST `/api/v1/admin/workspaces/deletion/preview` with its `workspace_id`.
   The response contains the workspace's name, ID and `lifecycle_revision`.
3. Review the exact target and retain a new operation UUID. POST
   `/api/v1/admin/workspaces/deletion/start` with that revision and a separately
   confirmed workspace ID:

```json
{
  "workspace_id": "d7ea11fe-7495-494f-95b7-37fe2462d4cf",
  "operation_id": "7cb22923-89a5-4780-b749-353f81225a24",
  "expected_revision": 2,
  "confirm_workspace_id": "d7ea11fe-7495-494f-95b7-37fe2462d4cf"
}
```

A reactivation, rearchive, rename or ownership change advances the lifecycle
revision. A stale confirmation conflicts and cannot delete the later lifecycle.
Unconverted workspaces linked to native sessions refuse with
`content_conversion_required`; this API cannot remove their retained source data.

The successful start response contains `workspace_id`, `operation_id`, the new
terminal `lifecycle_revision`, `started_at` and nullable `completed_at`. Start
commits an irreversible namespace fence and cleanup job together. It blocks all
workspace access and admission immediately. A null completion time means cleanup
is still pending. Do not display “deleted completely” from a start receipt.

## Track completion or an uncertain response

A current instance-admin key can POST
`/api/v1/admin/workspaces/deletion/status` with the exact `workspace_id` and
`operation_id`. The response contains only deletion metadata. A non-null
`completed_at` confirms that live content/history rows have been removed.

**The deleting workspace's scoped keys and browser sessions lose their role
immediately at start.** They cannot check the receipt or retry after the fence,
even before their stored keys are marked revoked. Save the returned receipt
before refreshing session state. If its response is lost, keep the original
workspace/operation IDs and use a current instance administrator to inspect them.
A permission failure or missing receipt does not prove that deletion failed.
There is no revoked-key exception and no automatic new operation.

An instance key that started the operation may retry the exact original request
while it remains authorized; it returns the observed receipt without applying
another deletion. Another actor or changed operation/revision conflicts. A
revoked, expired or demoted key cannot retrieve old success through this API.

## Cleanup and retained records

The existing workspace cleanup worker advances only explicitly confirmed deletion
jobs in bounded transactions. It shares the cleanup concurrency budget and holds
the data-directory lease through blocking SQL and shutdown. Failures roll back a
batch and retry later; restarting the workspace server resumes pending jobs.
Ordinary archived workspaces are never selected for whole-workspace deletion.
Native mode does not run this worker.

Cleanup removes shelves, books, entries and their archives/revision history,
workspace guidance and publication history, work-status text, retention state,
per-operation content/status/guidance proofs and workspace-specific audit and
enrollment history. It clears the workspace name, removes workspace admin roles,
revokes its scoped grants/keys/invitations/recovery invitations and redacts scoped
key labels. Global guidance and other workspaces remain intact.

The terminal workspace UUID and a minimal immutable deletion receipt/audit remain,
along with necessary revoked identity/security bindings. Registered clients,
credentials, certificates and admin principals are shared identities and remain;
access granted in other workspaces is preserved. The terminal UUID cannot be
reactivated or reused, and late operations cannot recreate its contents after
per-operation receipts are removed. A new workspace may reuse the old name with a
new UUID; saved selections continue to target the deleted UUID.

Deletion removes logical live database rows. It does not securely erase SQLite
pages, journals, filesystem snapshots or backups; backup retention is separate.

## Upgrade and verification

Back up the complete stopped data directory before upgrading schema23 to24. The
migration adds lifecycle counters and empty deletion-job storage, installs terminal
identity guards and indexes, and preserves existing rows. It starts no deletion.
Older binaries refuse schema24; rollback requires the complete matching old backup
and its producing binary. Never lower a schema marker or copy selected tables to
downgrade.

Foundation tests cover authority, lifecycle confirmation, exact retries, payload
cleanup, shared identity/access preservation, transaction faults and reopen. TLS
and cookie tests cover the routes, current authentication and worker restart.
These checks do not establish physical secure erasure or hosted/cross-platform
acceptance. See the [test guide](../tests/README.md) for repository gates.
