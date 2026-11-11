# Changelog

All notable changes to this project are documented in this file.

## [0.1.1]

### Security

- Fixed certificate revocation and rollback handling under concurrent requests (advisory pending).

### Fixed

- Hardened certificate revocation against concurrent and repeated requests.
- Hardened native state mutations (imports, restores, deletes) against concurrent interleaving.
- Routine checkpoints capture native state while excluding journal producers before retiring journal records.
- Fixed a bundle-import edge case that could leave the in-memory and persisted state inconsistent after a failed import.

### Known issues

- In rare crash or concurrency windows, native-mode journal recovery can replay a write twice or restore a just-deleted entry. Version 0.2.0 replaces the journal recovery design.
