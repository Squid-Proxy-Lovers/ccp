// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::{
    ContentError, EntryChange, EntryRetentionChange, EntryRetentionPolicy, TimedEntryPurge,
};
use rusqlite::OptionalExtension;

mod clocks;
mod policy;
mod purge;

pub(super) use clocks::{entry_changed, workspace_state};

impl AccessStore<'_> {
    /// Administrative metadata, checked against current read scope. No lease.
    pub fn entry_retention(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        now: i64,
    ) -> anyhow::Result<EntryRetentionPolicy> {
        if matches!(actor, Actor::Credential(_)) {
            return Err(ContentError::Forbidden.into());
        }
        let tx = self.connection.transaction()?;
        content::content_scope(&tx, actor, workspace, Action::ReadContext, now)?;
        let result = policy::current(&tx, workspace)?;
        tx.commit()?;
        Ok(result)
    }

    /// Changes only an active workspace's explicit retention configuration.
    /// Replays return historical policy receipts without resetting any clock.
    pub fn change_entry_retention(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        request: &EntryRetentionChange,
        now: i64,
    ) -> anyhow::Result<EntryRetentionPolicy> {
        policy::validate(request)?;
        self.mutate(|tx| {
            content::content_scope(tx, actor, workspace, Action::ManageRetention, now)?;
            if let Some(receipt) = policy::replay(tx, actor, workspace, request)? {
                return Ok(receipt);
            }
            let current = policy::current(tx, workspace)?;
            if current.revision != request.expected_revision
                || current.retention_seconds.is_some() == request.retention_seconds.is_some()
            {
                return Err(ContentError::Conflict.into());
            }
            let next = policy::record(tx, actor, workspace, request, now)?;
            clocks::configure(tx, &next, now)?;
            audit(
                tx,
                actor,
                "configure_entry_retention",
                Some(workspace),
                request.operation_id,
                now,
            )?;
            Ok(next)
        })
    }

    /// Trusted server worker ONLY, never a remote claimed identity or public
    /// admin override. The current persisted policy authorizes each due purge.
    /// No production worker is started by this internal storage API.
    pub fn purge_due_entry_archives(
        &mut self,
        workspace: Uuid,
        limit: usize,
        now: i64,
    ) -> anyhow::Result<Vec<TimedEntryPurge>> {
        if now < 0 || !(1..=32).contains(&limit) {
            return Err(ContentError::InvalidInput.into());
        }
        self.mutate(|tx| purge::due(tx, workspace, limit, now))
    }
}

fn exact_count(changed: usize, expected: usize) -> anyhow::Result<()> {
    if changed != expected {
        bail!("retention mutation did not affect its complete captured set");
    }
    Ok(())
}
