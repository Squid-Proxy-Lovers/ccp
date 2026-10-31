// SPDX-License-Identifier: AGPL-3.0-or-later

//! Local, observation-only inventory of a stopped native database. Never loads
//! native caches, initializes the source, interprets a journal or grants access.
use serde::Serialize;
use std::path::Path;

pub(crate) mod query;
pub(crate) mod snapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectionError {
    SourceUnavailable,
    PendingJournal,
    SidecarsPresent,
    TooLarge,
    SourceChanged,
    UnsupportedSchema,
    InvalidDatabase,
    ScratchUnavailable,
}

impl std::fmt::Display for InspectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::SourceUnavailable => "native inventory: requires an existing absolute directory and regular database, with no final symlinks",
            Self::PendingJournal => "native inventory: recover the journal with its producing software and stop it cleanly first; no journal was changed",
            Self::SidecarsPresent => "native inventory: SQLite WAL, SHM or rollback sidecar is present; stop the producer and preserve the complete source",
            Self::TooLarge => "native inventory: database exceeds this inspector's 2 GiB copy limit",
            Self::SourceChanged => "native inventory: source changed during inspection; no stable observation returned",
            Self::UnsupportedSchema => "native inventory: unsupported schema; HTTP holding schemas 3/4 and future formats are not accepted",
            Self::InvalidDatabase => "native inventory: SQLite integrity or required native layout could not be verified",
            Self::ScratchUnavailable => "native inventory: private inspection copy could not be created or written",
        })
    }
}
impl std::error::Error for InspectionError {}

type Result<T> = std::result::Result<T, InspectionError>;

#[derive(Debug, Serialize)]
pub struct NativeInventory {
    pub format_version: u32,
    pub source_schema: u32,
    pub database_sha256: String,
    pub database_bytes: u64,
    pub data_mode: &'static str,
    pub native: NativeCounts,
    pub workspace: WorkspaceCounts,
    pub observation_only: bool,
    pub conversion_performed: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct NativeCounts {
    pub sessions: u64,
    pub shelves: u64,
    pub books: u64,
    pub entries: u64,
    pub archives: u64,
    pub append_records: u64,
    pub archived_append_records: u64,
    pub certificates: u64,
    pub tokens: u64,
    pub transfer_records: u64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct WorkspaceCounts {
    pub workspaces: u64,
    pub principals: u64,
    pub keys: u64,
    pub clients: u64,
    pub grants: u64,
    pub entries: u64,
}

pub fn inspect(directory: &Path) -> Result<NativeInventory> {
    let snapshot = snapshot::Snapshot::capture(directory)?;
    let db = rusqlite::Connection::open_with_flags(
        snapshot.database(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|_error| InspectionError::InvalidDatabase)?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")
        .map_err(|_error| InspectionError::InvalidDatabase)?;
    let (source_schema, data_mode, native, workspace) = query::inventory(&db)?;
    snapshot.verify()?;
    Ok(NativeInventory {
        format_version: 1,
        source_schema,
        database_sha256: snapshot.fingerprint.sha256.clone(),
        database_bytes: snapshot.fingerprint.bytes,
        data_mode,
        native,
        workspace,
        observation_only: true,
        conversion_performed: false,
    })
}

#[cfg(test)]
mod tests;
