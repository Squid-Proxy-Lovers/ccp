// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Persistent foundations for the workspace redesign.
//!
//! These APIs are not external authentication endpoints. Callers must construct
//! actors from verified transport credentials, never a request's claimed ID.
//! Phase 1 leaves the existing native transport unchanged; new transport and
//! entry ownership enforcement are introduced by subsequent phases.

mod access;
mod admin;
mod certificate;
mod content;
mod entry;
mod guidance;
mod invitation;
mod model;
mod operation_ledger;
mod provisioning;
mod recovery;
mod retention;
mod store;
mod team;

pub use access::authorize;
pub(crate) use admin::AdminKeyIdentity;
pub use admin::{
    AdminKeyRequest, AdminKeyScope, BootstrappedInstance, IssuedAdminKey, LocalKeyReset,
    LocalKeyTarget,
};
pub(crate) use certificate::strict_pem_der;
pub use certificate::{
    EnrollmentReceipt, IssuingAuthority, RedemptionRequest, RenewalReceipt, RenewalRequest,
};
pub use content::{
    ArchivedShelf, HierarchyPage, NodeDraft, ShelfState, ShelfTransition, WorkspaceBook,
    WorkspaceShelf,
};
pub use entry::{
    ContentError, EntryChange, EntryContent, EntryMutation, EntryOperationOutcome,
    EntryOperationTarget, EntryPageRequest, EntryPurge, EntryPurgeReceipt, EntryReceipt,
    EntryRevision, EntrySnapshot, EntrySummary, RevisionPage, RevisionPageRequest,
};
pub use guidance::{
    GUIDANCE_TEXT_BYTES, GuidanceBoard, GuidanceBundle, GuidanceContext, GuidancePrecedence,
    GuidancePublisher, GuidanceRead, GuidanceReceipt, GuidanceSave, GuidanceScope,
};
pub use invitation::{
    IndividualInvitationRequest, InitialEnrollmentPhase, InitialEnrollmentStatus,
    InitialInvitationRequest, InvitationKind, InvitationPage, InvitationStatus, InvitationSummary,
    IssuedInvitation, ReadmissionInvitationRequest,
};
pub use model::{
    Action, Actor, Admission, ClientRegistration, DataRole, Enrollment, GrantPage, GrantSummary,
    WorkspaceAdministration, WorkspacePage, WorkspaceSummary,
};
pub use provisioning::{InitialOwner, OwnerProvisioning, ProvisionedWorkspace};
pub use recovery::{
    IssuedRecoveryInvitation, RecoveryInvitationPage, RecoveryInvitationRequest,
    RecoveryInvitationStatus, RecoveryInvitationSummary, RecoveryPageRequest, RecoveryReceipt,
    RecoveryRedemptionRequest, RecoveryScope,
};
pub use retention::{EntryRetentionChange, EntryRetentionPolicy, TimedEntryPurge};
pub use store::AccessStore;
pub use team::{
    AdminAppointment, AdminKeyPage, AdminKeySummary, AdminPage, AdminSummary, PrincipalSelection,
};

use anyhow::{Context, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

const SCHEMA: &str = include_str!("schema.sql");

pub(crate) fn migrate(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    transaction.execute_batch(SCHEMA)?;
    transaction.execute_batch(include_str!("content.sql"))?;
    transaction.execute_batch(include_str!("entry.sql"))?;
    operation_ledger::migrate(transaction)?;
    transaction.execute_batch(include_str!("operation_resolution.sql"))?;
    transaction.execute_batch(include_str!("shelf_lifecycle.sql"))?;
    transaction.execute_batch(include_str!("guidance.sql"))?;
    transaction.execute_batch(include_str!("entry_purge.sql"))?;
    transaction.execute_batch(include_str!("retention.sql"))?;
    let has_key_audit = transaction
        .prepare("PRAGMA table_info(ccp_access_audit)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == "admin_key_id");
    if !has_key_audit {
        transaction.execute_batch(
            "ALTER TABLE ccp_access_audit ADD COLUMN admin_key_id TEXT
             REFERENCES ccp_admin_keys(id) ON DELETE RESTRICT;",
        )?;
    }
    transaction.execute(
        "INSERT OR IGNORE INTO ccp_instance(singleton, instance_id) VALUES (1, ?1)",
        [Uuid::new_v4().to_string()],
    )?;
    parse_id(transaction.query_row(
        "SELECT instance_id FROM ccp_instance WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?)?;
    let sessions = transaction
        .prepare("SELECT id FROM sessions ORDER BY id")?
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for session in sessions {
        map_legacy_session(transaction, session)?;
    }
    Ok(())
}

pub(crate) fn map_legacy_session(connection: &Connection, session_id: i64) -> anyhow::Result<Uuid> {
    if let Some(id) = connection
        .query_row(
            "SELECT id FROM ccp_workspaces WHERE legacy_session_id = ?1",
            [session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return parse_id(id);
    }
    let name: String = connection.query_row(
        "SELECT name FROM sessions WHERE id = ?1",
        [session_id],
        |row| row.get(0),
    )?;
    let id = Uuid::new_v4();
    connection.execute(
        "INSERT INTO ccp_workspaces(id, legacy_session_id, name) VALUES (?1, ?2, ?3)",
        params![id.to_string(), session_id, name],
    )?;
    Ok(id)
}

fn parse_id(id: String) -> anyhow::Result<Uuid> {
    Uuid::parse_str(&id).context("invalid persisted foundation identity")
}

fn validate_name(name: &str) -> anyhow::Result<&str> {
    let name = name.trim();
    if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        bail!("name must contain 1..256 bytes without control characters");
    }
    Ok(name)
}

#[cfg(test)]
mod tests;
