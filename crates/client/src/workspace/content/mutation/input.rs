// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

fn plain(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
fn content(value: &wire::EntryContent) -> anyhow::Result<()> {
    if value.text.len() > 256 * 1024
        || value.description.len() > 4096
        || value.labels.len() > 32
        || !value.labels.iter().all(|v| plain(v, 128))
        || value
            .labels
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != value.labels.len()
    {
        bail!("invalid entry content bounds or labels");
    }
    Ok(())
}

pub(super) fn validate(request: &wire::EntryMutation) -> anyhow::Result<String> {
    validate::ids(&[&request.workspace_id, &request.operation_id])?;
    if Uuid::parse_str(&request.operation_id)?.is_nil()
        || request.agent.as_deref().is_some_and(|v| !plain(v, 256))
    {
        bail!("invalid entry operation ID or agent attribution");
    }
    match &request.change {
        wire::EntryChange::Create {
            book_id,
            name,
            content: value,
        } => {
            validate::ids(&[book_id])?;
            if !plain(name.trim(), 256) {
                bail!("invalid entry name");
            }
            content(value)?;
        }
        wire::EntryChange::Append {
            entry_id,
            expected_revision,
            text,
        } => {
            target(entry_id, *expected_revision)?;
            if text.is_empty() || text.len() > 256 * 1024 {
                bail!("invalid entry append size");
            }
        }
        wire::EntryChange::Revise {
            entry_id,
            expected_revision,
            content: value,
        } => {
            target(entry_id, *expected_revision)?;
            content(value)?;
        }
        wire::EntryChange::Archive {
            entry_id,
            expected_revision,
        }
        | wire::EntryChange::Restore {
            entry_id,
            expected_revision,
        } => target(entry_id, *expected_revision)?,
    }
    if serde_json::to_vec(request)?.len() > MAX_REQUEST {
        bail!("encoded entry request exceeds 2 MiB");
    }
    Ok(crypto::hash(&wire::fingerprint_bytes(
        &request.change,
        request.agent.as_deref(),
    )?))
}

fn target(id: &str, revision: u64) -> anyhow::Result<()> {
    validate::ids(&[id])?;
    validate::revision_number(revision)?;
    if revision == i64::MAX.unsigned_abs() {
        bail!("entry revision cannot be incremented");
    }
    Ok(())
}

pub(super) fn receipt(
    request: &wire::EntryMutation,
    value: &wire::EntryReceipt,
) -> anyhow::Result<()> {
    validate::ids(&[&value.entry_id])?;
    validate::revision_number(value.revision)?;
    let (id, revision) = match &request.change {
        wire::EntryChange::Create { .. } => (None, 1),
        wire::EntryChange::Append {
            entry_id,
            expected_revision,
            ..
        }
        | wire::EntryChange::Revise {
            entry_id,
            expected_revision,
            ..
        }
        | wire::EntryChange::Archive {
            entry_id,
            expected_revision,
        }
        | wire::EntryChange::Restore {
            entry_id,
            expected_revision,
        } => (
            Some(entry_id),
            expected_revision
                .checked_add(1)
                .context("invalid entry revision")?,
        ),
    };
    if value.revision != revision || id.is_some_and(|id| *id != value.entry_id) {
        bail!("entry receipt differs from the original intent");
    }
    Ok(())
}

pub(super) fn binding(
    value: &Binding,
    trust: &WorkspaceTrust,
    credential: &Credential,
) -> anyhow::Result<()> {
    validate::ids(&[
        &value.instance_id,
        &value.client_id,
        &value.credential_id,
        &value.workspace_id,
        &value.operation_id,
    ])?;
    if value.instance_id != trust.instance_id
        || value.client_id != credential.receipt.client_id
        || Uuid::parse_str(&value.operation_id)?.is_nil()
        || value.request_hash.len() != 64
        || !value
            .request_hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        bail!("invalid saved entry operation binding");
    }
    Ok(())
}
