// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use std::collections::BTreeSet;

#[derive(clap::Args)]
pub(crate) struct Args {
    #[arg(long)]
    workspace_id: String,
    #[arg(long)]
    entry_id: String,
    #[arg(long)]
    after: Option<u64>,
    #[arg(long, default_value_t = 32)]
    limit: u16,
}

pub(crate) async fn run(path: &std::path::Path, args: Args) -> anyhow::Result<()> {
    let value = WorkspaceProfile::open(path)?
        .content_native_history(&HistoryPage {
            workspace_id: args.workspace_id,
            entry_id: args.entry_id,
            after: args.after,
            limit: Some(args.limit),
        })
        .await?;
    println!("{}", serde_json::to_string(&value)?);
    Ok(())
}

impl WorkspaceProfile {
    pub async fn content_native_history(
        &mut self,
        request: &HistoryPage,
    ) -> anyhow::Result<NativeHistory> {
        validate::ids(&[&request.workspace_id, &request.entry_id])?;
        validate::page_input(None, request.limit)?;
        ensure(request.after.is_none_or(|n| n <= i64::MAX.unsigned_abs()))?;
        self.content_read(
            "entries/native-history",
            request,
            |value: &NativeHistory| check(value, request),
        )
        .await
    }
}

fn ensure(value: bool) -> anyhow::Result<()> {
    if !value {
        bail!("invalid or mismatched native source-history response");
    }
    Ok(())
}

fn check(value: &NativeHistory, request: &HistoryPage) -> anyhow::Result<()> {
    ensure(value.workspace_id == request.workspace_id && value.entry_id == request.entry_id)?;
    ensure(value.items.len() <= usize::from(request.limit.unwrap_or(32)))?;
    let Some(source) = &value.source else {
        return ensure(value.items.is_empty() && value.next_cursor.is_none());
    };
    ensure(source.converted_at >= 0 && lower_hex(&source.database_sha256, 64))?;
    let mut previous = request.after.unwrap_or(0);
    for item in &value.items {
        ensure(item.ordinal > previous && item.ordinal <= i64::MAX.unsigned_abs())?;
        record(item)?;
        previous = item.ordinal;
    }
    if let Some(cursor) = value.next_cursor {
        // The byte budget can produce fewer rows than the count limit.
        ensure(!value.items.is_empty() && cursor == previous)?;
    }
    Ok(())
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn integer(value: &str) -> bool {
    value.parse::<i64>().is_ok_and(|n| n.to_string() == value)
}

fn record(value: &NativeRecord) -> anyhow::Result<()> {
    ensure(match value.source_table.as_str() {
        "message_packs" | "message_history" | "deleted_message_history" => {
            integer(&value.source_key)
        }
        "deleted_message_packs" => !value.source_key.is_empty() && value.source_key.len() <= 8192,
        _ => false,
    })?;
    let row = &value.row;
    ensure(
        !row.columns.is_empty()
            && row.columns.len() <= 256
            && row.columns.len() == row.values.len(),
    )?;
    ensure(row.columns.iter().collect::<BTreeSet<_>>().len() == row.columns.len())?;
    ensure(serde_json::to_vec(row)?.len() <= 2 * 1024 * 1024)?;
    for value in &row.values {
        ensure(match value {
            NativeValue::Integer(v) => integer(v),
            NativeValue::Real(v) => lower_hex(v, 16),
            _ => true,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
