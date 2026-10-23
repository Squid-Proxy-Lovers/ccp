// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

pub(super) fn ids(values: &[&str]) -> anyhow::Result<()> {
    for id in values {
        crypto::canonical_id(id)?;
        if Uuid::parse_str(id)?.is_nil() {
            bail!("status IDs must be non-nil canonical UUIDs");
        }
    }
    Ok(())
}
pub(super) fn target(value: &WorkStatusTarget) -> anyhow::Result<()> {
    ids(&[
        &value.workspace_id,
        &value.status_id,
        &value.operation_id,
        &value.credential_id,
    ])?;
    if value.request_hash.len() != 64
        || !value
            .request_hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        bail!("invalid status request fingerprint");
    }
    Ok(())
}
pub(super) fn mutation(value: &wire::Mutation) -> anyhow::Result<String> {
    ids(&[&value.workspace_id, &value.status_id, &value.operation_id])?;
    if value.expected_revision >= i64::MAX.unsigned_abs() {
        bail!("status revision cannot be incremented");
    }
    if let wire::Change::Set { agent, message } = &value.change
        && (message.trim().is_empty()
            || message.len() > 4096
            || message.contains('\0')
            || agent
                .as_deref()
                .is_some_and(|a| a.trim().is_empty() || a.len() > 128 || a.contains('\0')))
    {
        bail!("invalid status message or attribution bounds");
    }
    let encoded = serde_json::to_vec(value)?;
    if encoded.len() > MAX_REQUEST {
        bail!("encoded status request exceeds64KiB");
    }
    Ok(crypto::hash(&encoded))
}
pub(super) fn binding(
    value: &Binding,
    trust: &WorkspaceTrust,
    credential: &Credential,
) -> anyhow::Result<()> {
    target(&value.target)?;
    ids(&[&value.instance_id, &value.client_id])?;
    if value.instance_id != trust.instance_id
        || value.client_id != credential.receipt.client_id
        || value.expected_revision >= i64::MAX.unsigned_abs()
    {
        bail!("saved status binding differs from profile identity");
    }
    Ok(())
}
pub(super) fn receipt(binding: &Binding, value: &wire::Receipt) -> anyhow::Result<()> {
    if value.workspace_id != binding.target.workspace_id
        || value.status_id != binding.target.status_id
        || value.operation_id != binding.target.operation_id
        || value.client_id != binding.client_id
        || value.revision != binding.expected_revision + 1
        || value.cleared != binding.cleared
        || value.updated_at < 0
    {
        bail!("status receipt differs from the original request; preserve pending intent");
    }
    Ok(())
}
