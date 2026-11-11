# Changelog

All notable changes to this project are documented in this file.

## [0.1.1]

### Security

- Fixed certificate revocation and rollback handling under concurrent requests (advisory pending).

### Fixed

- Hardened certificate revocation against concurrent and repeated requests.
- Hardened native state mutations (imports, restores, deletes) against concurrent interleaving.
- Native state is now persisted before a routine checkpoint retires journal records, so pending writes are no longer dropped by that checkpoint.
- Fixed a bundle-import edge case that could leave the in-memory and persisted state inconsistent after a failed import.
