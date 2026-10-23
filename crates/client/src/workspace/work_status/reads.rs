// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use std::collections::BTreeSet;

impl WorkspaceProfile {
    pub async fn status_message(&mut self, request: &wire::Get) -> anyhow::Result<wire::Status> {
        input::ids(&[&request.workspace_id, &request.status_id])?;
        self.authenticated_read(
            "api/v1/work/status/get",
            request,
            true,
            |_, value: &wire::Status| {
                status(value, &request.workspace_id, true)?;
                if value.status_id != request.status_id {
                    bail!("status response differs from requested slot");
                }
                Ok(())
            },
        )
        .await
    }
    pub async fn status_messages(&mut self, request: &wire::List) -> anyhow::Result<wire::Page> {
        input::ids(&[&request.workspace_id])?;
        if let Some(after) = &request.after {
            input::ids(&[after])?;
        }
        if request
            .limit
            .is_some_and(|limit| !(1..=32).contains(&limit))
        {
            bail!("status page limit must be1..32");
        }
        self.authenticated_read(
            "api/v1/work/status/list",
            request,
            true,
            |_, value: &wire::Page| page(value, request),
        )
        .await
    }
}
pub(super) fn status(
    value: &wire::Status,
    workspace: &str,
    allow_cleared: bool,
) -> anyhow::Result<()> {
    input::ids(&[&value.workspace_id, &value.status_id, &value.client_id])?;
    if value.workspace_id != workspace
        || value.client_name.is_empty()
        || value.revision == 0
        || value.revision > i64::MAX.unsigned_abs()
        || value.updated_at < 0
    {
        bail!("invalid status metadata or workspace target");
    }
    match &value.message {
        Some(message) if message.is_empty() || message.len() > 4096 || message.contains('\0') => {
            bail!("invalid status message bounds")
        }
        None if !allow_cleared || value.agent.is_some() || !value.stale => {
            bail!("invalid cleared status metadata")
        }
        _ => {}
    }
    if value
        .agent
        .as_deref()
        .is_some_and(|a| a.is_empty() || a.len() > 128 || a.contains('\0'))
    {
        bail!("invalid status attribution bounds");
    }
    Ok(())
}
pub(super) fn page(value: &wire::Page, request: &wire::List) -> anyhow::Result<()> {
    if value.items.len() > usize::from(request.limit.unwrap_or(32))
        || value.observed_at < 0
        || value.stale_after_seconds <= 0
    {
        bail!("invalid status page bounds or observation");
    }
    let mut seen = BTreeSet::new();
    let mut previous = request.after.as_deref();
    for item in &value.items {
        status(item, &request.workspace_id, false)?;
        if !seen.insert(&item.status_id) || previous.is_some_and(|id| id >= item.status_id.as_str())
        {
            bail!("status page IDs are duplicate or not after the original cursor");
        }
        let age = value.observed_at.checked_sub(item.updated_at);
        let stale = age.is_none_or(|n| !(0..value.stale_after_seconds).contains(&n));
        if item.stale != stale {
            bail!("status freshness differs from server observation");
        }
        previous = Some(&item.status_id);
    }
    if value
        .next_cursor
        .as_ref()
        .is_some_and(|id| value.items.last().is_none_or(|item| item.status_id != *id))
    {
        bail!("status cursor differs from last returned slot");
    }
    Ok(())
}
