// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Explicit current guidance reads. Automatic tool delivery is separate.
use super::*;
pub use protocol::workspace::guidance::{
    Board, Bundle, Context as DeliveryContext, Delivery, Precedence, Publisher, PublisherKind,
    Scope,
};

mod delivery;

const TEXT_BYTES: usize = 8192;

impl WorkspaceProfile {
    /// Omission requests global guidance only and never changes caller selection.
    /// A global-only result cannot by itself initiate certificate renewal.
    pub async fn guidance(&mut self, workspace: Option<&str>) -> anyhow::Result<Bundle> {
        if let Some(workspace) = workspace {
            identity(workspace)?;
        }
        let request = api::guidance::Get {
            workspace_id: workspace.map(str::to_string),
        };
        self.authenticated_read(
            "api/v1/guidance/get",
            &request,
            workspace.is_some(),
            |_, value| validate(value, workspace),
        )
        .await
    }
    /// Return current authenticated binding for future caller emission cursors.
    /// It is metadata, not a permission lease or proof of delivery or obedience.
    pub async fn guidance_delivery(&mut self, workspace: Option<&str>) -> anyhow::Result<Delivery> {
        if let Some(workspace) = workspace {
            identity(workspace)?;
        }
        let request = api::guidance::Get {
            workspace_id: workspace.map(str::to_string),
        };
        self.authenticated_read(
            "api/v1/guidance/context",
            &request,
            workspace.is_some(),
            |profile, value| delivery::validate(profile, value, workspace),
        )
        .await
    }
}

fn check(valid: bool) -> anyhow::Result<()> {
    if !valid {
        bail!("invalid or mismatched workspace guidance response");
    }
    Ok(())
}
fn identity(value: &str) -> anyhow::Result<()> {
    let id = crypto::canonical_id(value)
        .map_err(|_invalid_id| anyhow::anyhow!("invalid guidance identity UUID"))?;
    check(!id.is_nil())
}
fn validate(value: &Bundle, workspace: Option<&str>) -> anyhow::Result<()> {
    match value.precedence {
        Precedence::GlobalFirst => {}
    }
    board(&value.global, None)?;
    match (&value.workspace, workspace) {
        (Some(value), Some(workspace)) => board(value, Some(workspace)),
        (None, None) => Ok(()),
        _ => check(false),
    }
}
fn board(value: &Board, workspace: Option<&str>) -> anyhow::Result<()> {
    match (&value.scope, workspace) {
        (Scope::Global {}, None) => {}
        (Scope::Workspace { workspace_id }, Some(expected)) => {
            identity(workspace_id)?;
            check(workspace_id == expected)?;
        }
        _ => return check(false),
    }
    check(value.revision <= i64::MAX.unsigned_abs() && value.text.len() <= TEXT_BYTES)?;
    match (&value.publisher, value.revision) {
        (None, 0) => check(value.text.is_empty()),
        (Some(publisher), revision) if revision > 0 => attribution(publisher),
        _ => check(false),
    }
}
fn attribution(value: &Publisher) -> anyhow::Result<()> {
    identity(&value.actor_id)?;
    identity(&value.principal_id)?;
    check(value.recorded_at >= 0)?;
    match value.actor_kind {
        PublisherKind::Admin => check(value.actor_id == value.principal_id),
        PublisherKind::Key => Ok(()),
    }
}

#[cfg(test)]
mod tests;
