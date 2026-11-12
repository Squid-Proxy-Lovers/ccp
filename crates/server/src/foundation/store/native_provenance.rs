// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use crate::foundation::{
    ContentError, NativeEntryHistory, NativeEntryRecord, NativeHistoryRequest, NativeImportInfo,
    NativeRow,
};
use rusqlite::OptionalExtension;

const PAGE_BYTES: u64 = 4 * 1024 * 1024;

struct RecordMetadata {
    ordinal: u64,
    table: String,
    key: String,
    bytes: u64,
}

impl AccessStore<'_> {
    /// Raw history follows the same active-entry visibility as normal snapshots.
    /// This does not expose workspace security records or reconstruct revisions.
    pub fn native_entry_history(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        request: &NativeHistoryRequest,
        now: i64,
    ) -> anyhow::Result<Option<NativeEntryHistory>> {
        if !(1..=100).contains(&request.limit) {
            return Err(ContentError::InvalidInput.into());
        }
        let after = request
            .after
            .map(i64::try_from)
            .transpose()
            .map_err(|_error| ContentError::InvalidInput)?;
        let tx = self.connection.transaction()?;
        content::content_scope(&tx, actor, workspace, Action::ReadContext, now)?;
        entries::active_head(&tx, workspace, request.entry_id)?;
        let Some(source) = source(&tx, workspace, request.entry_id)? else {
            tx.commit()?;
            return Ok(None);
        };
        let metadata = metadata(&tx, workspace, request, after)?;
        let mut bytes = 1024_u64; // Import header and outer JSON structure.
        let mut items = Vec::new();
        for row in &metadata {
            let overhead = u64::try_from(row.key.len().saturating_add(row.table.len()))?
                .saturating_mul(6)
                .saturating_add(256);
            bytes = bytes.saturating_add(row.bytes).saturating_add(overhead);
            if items.len() == request.limit || bytes > PAGE_BYTES {
                break;
            }
            items.push(load(&tx, workspace, request.entry_id, row)?);
        }
        if items.is_empty() && !metadata.is_empty() {
            return Err(ContentError::InvalidInput.into());
        }
        let next_cursor = if items.len() < metadata.len() {
            items.last().map(|row| row.ordinal)
        } else {
            None
        };
        tx.commit()?;
        Ok(Some(NativeEntryHistory {
            source,
            items,
            next_cursor,
        }))
    }
}

fn source(
    db: &Connection,
    workspace: Uuid,
    entry: Uuid,
) -> anyhow::Result<Option<NativeImportInfo>> {
    let raw = db.query_row("SELECT p.source_sha256,p.source_schema,p.legacy_session_id,p.recorded_by,p.recorded_at FROM ccp_native_imports p WHERE p.workspace_id=?1 AND p.sealed=1 AND EXISTS(SELECT 1 FROM ccp_native_entry_rows r WHERE r.workspace_id=p.workspace_id AND r.entry_id=?2)",
        params![workspace.to_string(),entry.to_string()], |r| Ok((r.get::<_,String>(0)?,r.get::<_,u32>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?))).optional()?;
    raw.map(
        |(source_sha256, source_schema, legacy_session_id, admin, recorded_at)| {
            Ok(NativeImportInfo {
                source_sha256,
                source_schema,
                legacy_session_id,
                recorded_by: parse_id(admin)?,
                recorded_at,
            })
        },
    )
    .transpose()
}

fn metadata(
    db: &Connection,
    workspace: Uuid,
    request: &NativeHistoryRequest,
    after: Option<i64>,
) -> anyhow::Result<Vec<RecordMetadata>> {
    Ok(db.prepare("SELECT ordinal,source_table,source_key,length(CAST(row_json AS BLOB)) FROM ccp_native_entry_rows WHERE workspace_id=?1 AND entry_id=?2 AND ordinal>?3 ORDER BY ordinal LIMIT ?4")?
        .query_map(params![workspace.to_string(),request.entry_id.to_string(),after.unwrap_or(0),i64::try_from(request.limit+1)?], |r| Ok(RecordMetadata {
            ordinal:r.get(0)?, table:r.get(1)?, key:r.get(2)?, bytes:r.get(3)?,
        }))?.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load(
    db: &Connection,
    workspace: Uuid,
    entry: Uuid,
    record: &RecordMetadata,
) -> anyhow::Result<NativeEntryRecord> {
    let json: String = db.query_row("SELECT row_json FROM ccp_native_entry_rows WHERE workspace_id=?1 AND entry_id=?2 AND ordinal=?3",
        params![workspace.to_string(),entry.to_string(),i64::try_from(record.ordinal)?], |r| r.get(0))?;
    let row: NativeRow = serde_json::from_str(&json)
        .map_err(|_error| anyhow::anyhow!("stored native provenance is invalid"))?;
    if !row.is_valid() {
        bail!("stored native provenance is invalid");
    }
    Ok(NativeEntryRecord {
        ordinal: record.ordinal,
        source_table: record.table.clone(),
        source_key: record.key.clone(),
        row,
    })
}
