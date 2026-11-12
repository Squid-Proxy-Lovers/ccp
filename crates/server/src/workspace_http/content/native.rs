// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

pub(super) fn history(
    store: &mut AccessStore<'_>,
    actor: Actor,
    input: &Input,
    now: i64,
) -> Result<Reply, Error> {
    let body: api::HistoryPage = decode(&input.body)?;
    let workspace = id(&body.workspace_id)?;
    let request = domain::NativeHistoryRequest {
        entry_id: id(&body.entry_id)?,
        after: body.after,
        limit: limit(body.limit)?,
    };
    let result = store
        .native_entry_history(actor, workspace, &request, now)
        .map_err(operation)?;
    let (source, items, next_cursor) = match result {
        Some(value) => (
            Some(api::NativeSource {
                database_sha256: value.source.source_sha256,
                schema: value.source.source_schema,
                converted_at: value.source.recorded_at,
            }),
            value.items.into_iter().map(record).collect(),
            value.next_cursor,
        ),
        None => (None, Vec::new(), None),
    };
    response::content_snapshot(api::NativeHistory {
        workspace_id: workspace.to_string(),
        entry_id: request.entry_id.to_string(),
        source,
        items,
        next_cursor,
    })
}

fn record(value: domain::NativeEntryRecord) -> api::NativeRecord {
    api::NativeRecord {
        ordinal: value.ordinal,
        source_table: value.source_table,
        source_key: value.source_key,
        row: api::NativeRow {
            columns: value.row.columns,
            values: value.row.values.into_iter().map(scalar).collect(),
        },
    }
}

fn scalar(value: domain::NativeSqlValue) -> api::NativeValue {
    use domain::NativeSqlValue as V;
    match value {
        V::Null => api::NativeValue::Null,
        V::Integer(v) => api::NativeValue::Integer(v.to_string()),
        V::Real(v) => api::NativeValue::Real(format!("{v:016x}")),
        V::Text(v) => api::NativeValue::Text(v),
        V::TextBytes(v) => api::NativeValue::TextBytes(v),
        V::Blob(v) => api::NativeValue::Blob(v),
    }
}
