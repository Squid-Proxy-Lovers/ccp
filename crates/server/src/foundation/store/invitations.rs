// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use rusqlite::OptionalExtension;

use super::*;
use crate::foundation::admin::key_hash;
use crate::foundation::{
    IndividualInvitationRequest, InitialInvitationRequest, InvitationStatus, IssuedInvitation,
};

impl AccessStore<'_> {
    /// Opens the one initial group window. Existing admission records, including
    /// revoked grants, prevent starting a later bulk-enrollment phase.
    pub fn issue_initial_invitation(
        &mut self,
        actor: Actor,
        request: InitialInvitationRequest,
        now: i64,
    ) -> anyhow::Result<IssuedInvitation> {
        validate_bounds(request.max_clients.map(u64::from), request.expires_at, now)?;
        self.mutate(|tx| {
            authorize(
                tx,
                actor,
                Action::AdmitClient,
                Some(request.workspace_id),
                now,
            )?;
            let started: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM ccp_initial_enrollment WHERE workspace_id = ?1)
                 OR EXISTS(SELECT 1 FROM ccp_grants WHERE workspace_id = ?1)",
                [request.workspace_id.to_string()],
                |row| row.get(0),
            )?;
            if started {
                bail!("initial enrollment has already started or closed");
            }
            let issued = insert_invitation(
                tx,
                actor,
                InvitationInsert {
                    workspace: request.workspace_id,
                    role: request.role,
                    kind: "initial_group",
                    max_uses: request.max_clients.map(u64::from),
                    expires_at: request.expires_at,
                },
            )?;
            tx.execute(
                "INSERT INTO ccp_initial_enrollment(workspace_id, invitation_id) VALUES (?1, ?2)",
                params![request.workspace_id.to_string(), issued.id.to_string()],
            )?;
            enrollment_event(tx, request.workspace_id, "opened", now)?;
            audit(
                tx,
                actor,
                "issue_initial_invitation",
                Some(request.workspace_id),
                issued.id,
                now,
            )?;
            Ok(issued)
        })
    }

    /// Starting individual admission seals an unused initial window. An active
    /// group window must first be explicitly closed; issuance never resets it.
    pub fn issue_individual_invitation(
        &mut self,
        actor: Actor,
        request: IndividualInvitationRequest,
        now: i64,
    ) -> anyhow::Result<IssuedInvitation> {
        validate_bounds(Some(1), Some(request.expires_at), now)?;
        self.mutate(|tx| {
            authorize(
                tx,
                actor,
                Action::AdmitClient,
                Some(request.workspace_id),
                now,
            )?;
            refresh_initial(tx, request.workspace_id, now)?;
            let open: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM ccp_initial_enrollment
                 WHERE workspace_id = ?1 AND closed_at IS NULL)",
                [request.workspace_id.to_string()],
                |row| row.get(0),
            )?;
            if open {
                bail!("close initial group enrollment before individual admission");
            }
            close_initial(tx, request.workspace_id, "manual_close", now)?;
            let issued = insert_invitation(
                tx,
                actor,
                InvitationInsert {
                    workspace: request.workspace_id,
                    role: request.role,
                    kind: "individual",
                    max_uses: Some(1),
                    expires_at: Some(request.expires_at),
                },
            )?;
            audit(
                tx,
                actor,
                "issue_individual_invitation",
                Some(request.workspace_id),
                issued.id,
                now,
            )?;
            Ok(issued)
        })
    }

    pub fn close_initial_enrollment(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        now: i64,
    ) -> anyhow::Result<()> {
        self.mutate(|tx| {
            authorize(tx, actor, Action::RevokeGrant, Some(workspace), now)?;
            if close_initial(tx, workspace, "manual_close", now)? {
                audit(
                    tx,
                    actor,
                    "close_initial_enrollment",
                    Some(workspace),
                    workspace,
                    now,
                )?;
            }
            Ok(())
        })
    }

    pub fn revoke_invitation(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        invitation: Uuid,
        now: i64,
    ) -> anyhow::Result<()> {
        self.mutate(|tx| {
            authorize(tx, actor, Action::RevokeGrant, Some(workspace), now)?;
            require_changed(tx.execute(
                "UPDATE ccp_invitations SET revoked_at = ?1 WHERE id = ?2
                 AND workspace_id = ?3 AND revoked_at IS NULL",
                params![now, invitation.to_string(), workspace.to_string()],
            )?)?;
            let initial: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM ccp_initial_enrollment WHERE workspace_id = ?1 AND invitation_id = ?2)",
                params![workspace.to_string(), invitation.to_string()], |row| row.get(0),
            )?;
            if initial {
                close_initial(tx, workspace, "manual_close", now)?;
            }
            audit(tx, actor, "revoke_invitation", Some(workspace), invitation, now)
        })
    }

    /// Administrative metadata only, never a redemption authorization lease.
    /// Observed expiry/capacity closure is persisted even across clock rollback.
    pub fn invitation_status(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        invitation: Uuid,
        now: i64,
    ) -> anyhow::Result<InvitationStatus> {
        self.mutate(|tx| {
            authorize(tx, actor, Action::RevokeGrant, Some(workspace), now)?;
            refresh_initial(tx, workspace, now)?;
            let row = tx.query_row(
                "SELECT uses, max_uses, expires_at, revoked_at, issuer_kind, issuer_id,
                 EXISTS(SELECT 1 FROM ccp_initial_enrollment w WHERE w.workspace_id = i.workspace_id
                 AND w.invitation_id = i.id AND w.closed_at IS NOT NULL)
                 FROM ccp_invitations i WHERE id = ?1 AND workspace_id = ?2",
                params![invitation.to_string(), workspace.to_string()],
                |row| {
                    Ok((
                        row.get::<_, u64>(0)?,
                        row.get::<_, Option<u64>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, bool>(6)?,
                    ))
                },
            )?;
            let issuer = parse_id(row.5)?;
            let issuer = match row.4.as_str() {
                "admin" => Actor::Admin(issuer),
                "admin_key" => Actor::AdminKey(issuer),
                _ => bail!("invalid persisted invitation issuer"),
            };
            Ok(InvitationStatus {
                id: invitation,
                uses: row.0,
                max_uses: row.1,
                expires_at: row.2,
                revoked: row.3.is_some(),
                initial_closed: row.6,
                issuer_authorized: permitted(
                    tx,
                    issuer,
                    Action::AdmitClient,
                    Some(workspace),
                    now,
                )?,
            })
        })
    }
}

fn validate_bounds(max_uses: Option<u64>, expires_at: Option<i64>, now: i64) -> anyhow::Result<()> {
    if now < 0
        || max_uses == Some(0)
        || (max_uses.is_none() && expires_at.is_none())
        || expires_at.is_some_and(|expiry| expiry <= now)
    {
        bail!("invitation needs positive capacity or a future deadline");
    }
    Ok(())
}

struct InvitationInsert {
    workspace: Uuid,
    role: super::super::DataRole,
    kind: &'static str,
    max_uses: Option<u64>,
    expires_at: Option<i64>,
}

fn insert_invitation(
    tx: &Connection,
    actor: Actor,
    request: InvitationInsert,
) -> anyhow::Result<IssuedInvitation> {
    let (kind, issuer) = match actor {
        Actor::Admin(id) => ("admin", id),
        Actor::AdminKey(id) => ("admin_key", id),
        Actor::Credential(_) => bail!("client credentials cannot issue invitations"),
    };
    let issued = IssuedInvitation::generate()?;
    let capacity = request.max_uses.map(i64::try_from).transpose()?;
    tx.execute(
        "INSERT INTO ccp_invitations(id, workspace_id, kind, role, token_hash, issuer_kind, issuer_id, max_uses, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![issued.id.to_string(), request.workspace.to_string(), request.kind, request.role.as_str(),
            key_hash(issued.secret()), kind, issuer.to_string(), capacity, request.expires_at],
    )?;
    Ok(issued)
}

fn enrollment_event(tx: &Connection, workspace: Uuid, event: &str, now: i64) -> anyhow::Result<()> {
    tx.execute(
        "INSERT INTO ccp_enrollment_events(workspace_id, event, recorded_at) VALUES (?1, ?2, ?3)",
        params![workspace.to_string(), event, now],
    )?;
    Ok(())
}

fn close_initial(tx: &Connection, workspace: Uuid, event: &str, now: i64) -> anyhow::Result<bool> {
    let changed = tx.execute(
        "INSERT INTO ccp_initial_enrollment(workspace_id, closed_at) VALUES (?1, ?2)
         ON CONFLICT(workspace_id) DO UPDATE SET closed_at = excluded.closed_at
         WHERE ccp_initial_enrollment.closed_at IS NULL",
        params![workspace.to_string(), now],
    )?;
    if changed > 0 {
        enrollment_event(tx, workspace, event, now)?;
    }
    Ok(changed > 0)
}

fn refresh_initial(tx: &Connection, workspace: Uuid, now: i64) -> anyhow::Result<()> {
    let boundary: Option<(bool, bool)> = tx
        .query_row(
            "SELECT i.expires_at IS NOT NULL AND i.expires_at <= ?2,
         i.max_uses IS NOT NULL AND i.uses >= i.max_uses FROM ccp_initial_enrollment w
         JOIN ccp_invitations i ON i.id = w.invitation_id
         WHERE w.workspace_id = ?1 AND w.closed_at IS NULL",
            params![workspace.to_string(), now],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match boundary {
        Some((true, _)) => {
            close_initial(tx, workspace, "deadline_close", now)?;
        }
        Some((_, true)) => {
            close_initial(tx, workspace, "capacity_close", now)?;
        }
        _ => {}
    }
    Ok(())
}
