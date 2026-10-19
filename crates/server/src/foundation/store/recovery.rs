// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use rusqlite::OptionalExtension;

use super::*;
use crate::foundation::admin::key_hash;
use crate::foundation::certificate::VerifiedCsr;
use crate::foundation::{
    IssuedRecoveryInvitation, IssuingAuthority, RecoveryInvitationRequest,
    RecoveryInvitationStatus, RecoveryReceipt, RecoveryRedemptionRequest, RecoveryScope,
};

mod records;
use records::{load_recovery, recovery_retry, replace_bindings};

struct RecoveryInvitation {
    id: Uuid,
    client: Uuid,
    scope: RecoveryScope,
    issuer: Actor,
    expires_at: i64,
    revoked: bool,
    expired: bool,
}

enum RecoveryResult {
    Accepted(RecoveryReceipt),
    Expired,
}

impl AccessStore<'_> {
    /// Issues fresh, one-use recovery of server-captured existing grant IDs.
    /// Does not admit a new member, restore a revoked grant or open enrollment.
    pub fn issue_recovery_invitation(
        &mut self,
        actor: Actor,
        request: RecoveryInvitationRequest,
        now: i64,
    ) -> anyhow::Result<IssuedRecoveryInvitation> {
        if now < 0
            || request
                .expires_at
                .checked_sub(now)
                .is_none_or(|ttl| ttl <= 0 || ttl > 7 * 24 * 60 * 60)
        {
            bail!("recovery invitation needs a future deadline within seven days");
        }
        self.mutate(|tx| {
            authorize(tx, actor, request.scope.action(), request.scope.workspace(), now)?;
            require_recoverable_client(tx, request.client_id)?;
            let grants = tx.prepare(
                "SELECT g.id FROM ccp_grants g JOIN ccp_workspaces w ON w.id = g.workspace_id
                 WHERE g.client_id = ?1 AND g.revoked_at IS NULL AND w.state != 'purged'
                 AND (?2 IS NULL OR g.workspace_id = ?2) ORDER BY g.id",
            )?.query_map(params![request.client_id.to_string(), request.scope.workspace().map(|id| id.to_string())], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if grants.is_empty() {
                bail!("recovery requires a current grant in the authorized scope");
            }
            let (kind, issuer) = match actor {
                Actor::Admin(id) => ("admin", id),
                Actor::AdminKey(id) => ("admin_key", id),
                Actor::Credential(_) => bail!("client credentials cannot issue recovery invitations"),
            };
            let issued = IssuedRecoveryInvitation::generate()?;
            tx.execute(
                "INSERT INTO ccp_recovery_invitations(id, client_id, workspace_id, token_hash,
                 issuer_kind, issuer_id, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![issued.id.to_string(), request.client_id.to_string(), request.scope.workspace().map(|id| id.to_string()),
                    key_hash(issued.secret()), kind, issuer.to_string(), request.expires_at],
            )?;
            for grant in grants {
                tx.execute("INSERT INTO ccp_recovery_grants(invitation_id, grant_id, client_id) VALUES (?1, ?2, ?3)",
                    params![issued.id.to_string(), grant, request.client_id.to_string()])?;
            }
            audit(tx, actor, "issue_client_recovery", request.scope.workspace(), issued.id, now)?;
            Ok(issued)
        })
    }

    /// Replaces only covered old bindings. A fresh bearer secret authorizes the
    /// captured identity; no old/expired certificate or claimed ID is accepted.
    pub fn redeem_recovery_invitation(
        &mut self,
        secret: &str,
        request: &RecoveryRedemptionRequest,
        authority: &IssuingAuthority,
        now: i64,
    ) -> anyhow::Result<RecoveryReceipt> {
        validate_recovery_secret(secret, now)?;
        let proof = VerifiedCsr::parse(&request.csr_pem)?;
        let outcome = self.mutate(|tx| {
            let id: String = tx
                .query_row(
                    "SELECT id FROM ccp_recovery_invitations WHERE token_hash = ?1",
                    [key_hash(secret)],
                    |row| row.get(0),
                )
                .optional()?
                .context("recovery invitation is not valid")?;
            let invite = load_recovery(tx, parse_id(id)?)?;
            if invite.revoked {
                bail!("recovery invitation is not valid");
            }
            authorize(
                tx,
                invite.issuer,
                invite.scope.action(),
                invite.scope.workspace(),
                now,
            )?;
            require_recoverable_client(tx, invite.client)?;
            if let Some(receipt) = recovery_retry(tx, &invite, request, &proof, now)? {
                return Ok(RecoveryResult::Accepted(receipt));
            }
            if observe_expiry(tx, &invite, now)? {
                return Ok(RecoveryResult::Expired);
            }
            let receipt = replace_bindings(tx, &invite, request, &proof, authority, now)?;
            audit(
                tx,
                Actor::Credential(receipt.credential_id),
                "redeem_client_recovery",
                invite.scope.workspace(),
                invite.id,
                now,
            )?;
            Ok(RecoveryResult::Accepted(receipt))
        })?;
        match outcome {
            RecoveryResult::Accepted(receipt) => Ok(receipt),
            RecoveryResult::Expired => bail!("recovery invitation has expired"),
        }
    }

    pub fn revoke_recovery_invitation(
        &mut self,
        actor: Actor,
        invitation: Uuid,
        now: i64,
    ) -> anyhow::Result<()> {
        self.mutate(|tx| {
            let invite = load_recovery(tx, invitation)?;
            authorize(tx, actor, invite.scope.action(), invite.scope.workspace(), now)?;
            observe_expiry(tx, &invite, now)?;
            require_changed(tx.execute("UPDATE ccp_recovery_invitations SET revoked_at = ?1 WHERE id = ?2 AND revoked_at IS NULL",
                params![now, invitation.to_string()])?)?;
            audit(tx, actor, "revoke_client_recovery", invite.scope.workspace(), invitation, now)
        })
    }

    pub fn recovery_invitation_status(
        &mut self,
        actor: Actor,
        invitation: Uuid,
        now: i64,
    ) -> anyhow::Result<RecoveryInvitationStatus> {
        self.mutate(|tx| {
            let invite = load_recovery(tx, invitation)?;
            authorize(
                tx,
                actor,
                invite.scope.action(),
                invite.scope.workspace(),
                now,
            )?;
            let expired = observe_expiry(tx, &invite, now)?;
            let redeemed = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM ccp_recovery_receipts WHERE invitation_id = ?1)",
                [invitation.to_string()],
                |row| row.get(0),
            )?;
            Ok(RecoveryInvitationStatus {
                client_id: invite.client,
                scope: invite.scope,
                expires_at: invite.expires_at,
                revoked: invite.revoked,
                expired,
                redeemed,
                issuer_authorized: permitted(
                    tx,
                    invite.issuer,
                    invite.scope.action(),
                    invite.scope.workspace(),
                    now,
                )?,
            })
        })
    }
}

fn require_recoverable_client(tx: &Connection, client: Uuid) -> anyhow::Result<()> {
    let enabled: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM ccp_clients WHERE id = ?1 AND disabled = 0)",
        [client.to_string()],
        |row| row.get(0),
    )?;
    if !enabled {
        bail!("client is unknown or globally disabled");
    }
    Ok(())
}

fn validate_recovery_secret(secret: &str, now: i64) -> anyhow::Result<()> {
    let Some(hex) = secret.strip_prefix("ccp_recover_") else {
        bail!("recovery invitation is not valid");
    };
    if now < 0 || hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("recovery invitation is not valid");
    }
    Ok(())
}

fn observe_expiry(tx: &Connection, invite: &RecoveryInvitation, now: i64) -> anyhow::Result<bool> {
    if invite.expired || now >= invite.expires_at {
        tx.execute("UPDATE ccp_recovery_invitations SET expired_at = ?1 WHERE id = ?2 AND expired_at IS NULL",
            params![now, invite.id.to_string()])?;
        return Ok(true);
    }
    Ok(false)
}
