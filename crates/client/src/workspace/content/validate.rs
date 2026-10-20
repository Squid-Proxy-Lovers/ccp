// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

fn check(value: bool) -> anyhow::Result<()> {
    if !value {
        bail!("invalid or mismatched workspace content response");
    }
    Ok(())
}

pub(super) fn ids(values: &[&str]) -> anyhow::Result<()> {
    for value in values {
        crypto::canonical_id(value)
            .map_err(|_invalid_id| anyhow::anyhow!("invalid content identity UUID"))?;
    }
    Ok(())
}

pub(super) fn same(value: &str, expected: &str) -> anyhow::Result<()> {
    check(value == expected)
}

pub(super) fn page_input(after: Option<&str>, limit: Option<u16>) -> anyhow::Result<()> {
    check((1..=32).contains(&limit.unwrap_or(32)))?;
    if let Some(id) = after {
        ids(&[id])?;
    }
    Ok(())
}

fn name(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn node(value: &str, description: &str, created: i64) -> anyhow::Result<()> {
    check(name(value, 256) && value.trim() == value && description.len() <= 4096 && created >= 0)
}

pub(super) fn shelf(value: &Shelf, workspace: &str) -> anyhow::Result<()> {
    ids(&[&value.id, &value.workspace_id])?;
    same(&value.workspace_id, workspace)?;
    node(&value.name, &value.description, value.created_at)
}

pub(super) fn book(value: &Book, workspace: &str, parent: Option<&str>) -> anyhow::Result<()> {
    ids(&[&value.id, &value.workspace_id, &value.shelf_id])?;
    same(&value.workspace_id, workspace)?;
    if let Some(parent) = parent {
        same(&value.shelf_id, parent)?;
    }
    node(&value.name, &value.description, value.created_at)
}

pub(super) fn page<T>(
    value: &Page<T>,
    after: Option<&str>,
    limit: Option<u16>,
    id: impl Fn(&T) -> &str,
    validate: impl Fn(&T) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let limit = usize::from(limit.unwrap_or(32));
    check(value.items.len() <= limit)?;
    let mut previous = after;
    for item in &value.items {
        validate(item)?;
        let current = id(item);
        check(previous.is_none_or(|p| current > p))?;
        previous = Some(current);
    }
    if let Some(cursor) = &value.next_cursor {
        ids(&[cursor])?;
        check(value.items.len() == limit && previous == Some(cursor.as_str()))?;
    }
    Ok(())
}

pub(super) fn revision_number(value: u64) -> anyhow::Result<()> {
    check(value > 0 && value <= i64::MAX.unsigned_abs())
}

pub(super) fn summary(value: &EntrySummary, workspace: &str) -> anyhow::Result<()> {
    ids(&[&value.id, &value.workspace_id, &value.book_id])?;
    same(&value.workspace_id, workspace)?;
    if let Some(owner) = &value.owner_client_id {
        ids(&[owner])?;
    }
    revision_number(value.revision)?;
    node(&value.name, "", value.created_at)?;
    check(value.updated_at >= 0)
}

fn revision(value: &EntryRevision) -> anyhow::Result<()> {
    revision_number(value.revision)?;
    check(matches!(
        value.action.as_str(),
        "create" | "append" | "revise" | "archive" | "restore"
    ))?;
    check(value.recorded_at >= 0 && value.agent.as_deref().is_none_or(|v| name(v, 256)))?;
    match &value.actor {
        ContentActor::Credential(id) => {
            ids(&[
                id,
                value
                    .client_id
                    .as_deref()
                    .context("missing content client attribution")?,
            ])?;
            check(value.admin_principal_id.is_none())
        }
        ContentActor::Admin(id) => {
            ids(&[id])?;
            check(value.admin_principal_id.as_deref() == Some(id) && value.client_id.is_none())
        }
        ContentActor::AdminKey(id) => {
            ids(&[
                id,
                value
                    .admin_principal_id
                    .as_deref()
                    .context("missing content admin attribution")?,
            ])?;
            check(value.client_id.is_none())
        }
    }
}

pub(super) fn snapshot(value: &EntrySnapshot, request: &EntryTarget) -> anyhow::Result<()> {
    summary(&value.entry, &request.workspace_id)?;
    same(&value.entry.id, &request.entry_id)?;
    revision(&value.revision)?;
    check(!value.entry.archived && value.revision.revision <= value.entry.revision)?;
    check(value.revision.revision == request.revision.unwrap_or(value.entry.revision))?;
    let content = &value.content;
    check(
        content.text.len() <= 256 * 1024
            && content.description.len() <= 4096
            && content.labels.len() <= 32,
    )?;
    let labels = content
        .labels
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    check(labels.len() == content.labels.len() && content.labels.iter().all(|v| name(v, 128)))?;
    if value.revision.action == "append" {
        let delta = value
            .appended_text
            .as_deref()
            .context("missing original content contribution")?;
        check(!delta.is_empty() && delta.len() <= 256 * 1024 && content.text.ends_with(delta))?;
    } else {
        check(value.appended_text.is_none())?;
    }
    Ok(())
}

pub(super) fn history(value: &History, request: &HistoryPage) -> anyhow::Result<()> {
    let limit = usize::from(request.limit.unwrap_or(32));
    check(value.items.len() <= limit)?;
    let mut previous = request.after.unwrap_or(0);
    for item in &value.items {
        revision(item)?;
        check(item.revision > previous)?;
        previous = item.revision;
    }
    if let Some(cursor) = value.next_cursor {
        check(value.items.len() == limit && cursor == previous)?;
    }
    Ok(())
}
