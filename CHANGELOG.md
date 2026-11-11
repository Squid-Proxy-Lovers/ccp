# Changelog

All notable changes to this project are documented in this file.

## [0.1.1]

### Security

- Fixed certificate revocation and rollback handling under concurrent requests (advisory: TBD).

### Fixed

- `revoke_client_cert` now checks and marks a certificate revoked under a single
  lock held for the whole operation, so a retried or concurrent revoke of the
  same certificate can no longer undo an earlier revocation.
- `import_bundle`, `restore_deleted_entry` and `delete_entry` now hold an
  exclusive mutation guard for their full duration, so their rollback path can
  no longer clobber another client's concurrent write.
