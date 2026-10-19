// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use rusqlite::OptionalExtension;

use super::invitations::{refresh_initial, refresh_invitation_expiry};
use super::readmission::{readmission_current, readmission_target};
use super::*;
use crate::foundation::admin::key_hash;
use crate::foundation::certificate::VerifiedCsr;
use crate::foundation::{EnrollmentReceipt, IssuingAuthority, RedemptionRequest};

struct Invitation {
    id: Uuid,
    workspace: Uuid,
    role: DataRole,
    initial: bool,
    issuer: Actor,
    uses: i64,
    capacity: Option<i64>,
    revoked: bool,
}

struct PreparedIdentity {
    client_id: Uuid,
    credential_id: Uuid,
    certificate_pem: String,
    expires_at: i64,
}

enum Redemption {
    Accepted(EnrollmentReceipt),
    Denied,
}

impl AccessStore<'_> {
    /// Redeems a bearer invitation and a signed CSR. Existing-identity claims
    /// require a live credential with exactly the CSR's verified public key.
    /// Receipt replay returns committed public material, never another grant.
    pub fn redeem_invitation(
        &mut self,
        secret: &str,
        request: &RedemptionRequest,
        authority: &IssuingAuthority,
        now: i64,
    ) -> anyhow::Result<EnrollmentReceipt> {
        if request.existing_credential.is_some() {
            bail!("existing identity admission requires an authenticated client connection");
        }
        self.redeem_request(secret, request, authority, now)
    }

    /// The actor must come from a verified mTLS connection, never request JSON.
    /// A public signed CSR by itself cannot select an existing client identity.
    pub fn admit_with_invitation(
        &mut self,
        actor: Actor,
        secret: &str,
        request: &RedemptionRequest,
        authority: &IssuingAuthority,
        now: i64,
    ) -> anyhow::Result<EnrollmentReceipt> {
        if !matches!(actor, Actor::Credential(id) if request.existing_credential == Some(id)) {
            bail!("existing identity admission requires its authenticated credential");
        }
        self.redeem_request(secret, request, authority, now)
    }

    fn redeem_request(
        &mut self,
        secret: &str,
        request: &RedemptionRequest,
        authority: &IssuingAuthority,
        now: i64,
    ) -> anyhow::Result<EnrollmentReceipt> {
        let suffix = secret.strip_prefix("ccp_join_");
        if now < 0
            || !suffix.is_some_and(|value| {
                value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
            })
        {
            bail!("invalid invitation redemption");
        }
        let proof = VerifiedCsr::parse(&request.csr_pem)?;
        let result = self.mutate(|tx| redeem(tx, secret, request, &proof, authority, now))?;
        match result {
            Redemption::Accepted(receipt) => Ok(receipt),
            Redemption::Denied => bail!("invitation redemption is not authorized or is closed"),
        }
    }
}

fn redeem(
    tx: &Connection,
    secret: &str,
    request: &RedemptionRequest,
    proof: &VerifiedCsr,
    authority: &IssuingAuthority,
    now: i64,
) -> anyhow::Result<Redemption> {
    let Some(invite) = load_invitation(tx, secret)? else {
        return Ok(Redemption::Denied);
    };
    refresh_invitation_expiry(tx, invite.workspace, invite.id, now)?;
    if invite.revoked {
        return Ok(Redemption::Denied);
    }
    if let Some(replayed) = replay(tx, &invite, request, proof, now)? {
        return Ok(replayed);
    }
    if !permitted(
        tx,
        invite.issuer,
        Action::AdmitClient,
        Some(invite.workspace),
        now,
    )? || invitation_closed(tx, &invite)?
    {
        return Ok(Redemption::Denied);
    }
    let Some(identity) = prepare_invited_identity(tx, &invite, request, proof, authority, now)?
    else {
        return Ok(Redemption::Denied);
    };
    let grant_id = grant_and_bind(
        tx,
        invite.workspace,
        identity.client_id,
        identity.credential_id,
        invite.role,
    )?;
    let receipt = EnrollmentReceipt {
        client_id: identity.client_id,
        credential_id: identity.credential_id,
        grant_id,
        certificate_pem: identity.certificate_pem,
        expires_at: identity.expires_at,
    };
    require_changed(tx.execute(
        "UPDATE ccp_invitations SET uses = uses + 1 WHERE id = ?1
         AND uses < 9223372036854775807 AND (max_uses IS NULL OR uses < max_uses)",
        [invite.id.to_string()],
    )?)?;
    tx.execute(
        "INSERT INTO ccp_invitation_receipts(invitation_id, request_id, workspace_id, public_key_hash,
         previous_credential_id, client_id, credential_id, grant_id, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![invite.id.to_string(), request.request_id.to_string(), invite.workspace.to_string(), proof.key_hash,
            request.existing_credential.map(|id| id.to_string()), receipt.client_id.to_string(),
            receipt.credential_id.to_string(), receipt.grant_id.to_string(), now],
    )?;
    refresh_initial(tx, invite.workspace, now)?;
    audit(
        tx,
        Actor::Credential(receipt.credential_id),
        "redeem_invitation",
        Some(invite.workspace),
        invite.id,
        now,
    )?;
    Ok(Redemption::Accepted(receipt))
}

fn load_invitation(tx: &Connection, secret: &str) -> anyhow::Result<Option<Invitation>> {
    let row = tx.query_row(
        "SELECT id, workspace_id, role, kind, issuer_kind, issuer_id, uses, max_uses, revoked_at IS NOT NULL
         FROM ccp_invitations WHERE token_hash = ?1", [key_hash(secret)],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
            row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?,
            row.get::<_, i64>(6)?, row.get::<_, Option<i64>>(7)?, row.get::<_, bool>(8)?)),
    ).optional()?;
    let Some(row) = row else {
        return Ok(None);
    };
    let issuer = parse_id(row.5)?;
    let issuer = match row.4.as_str() {
        "admin" => Actor::Admin(issuer),
        "admin_key" => Actor::AdminKey(issuer),
        _ => bail!("invalid persisted invitation issuer"),
    };
    let role = match row.2.as_str() {
        "read" => DataRole::Read,
        "read_write" => DataRole::ReadWrite,
        _ => bail!("invalid persisted invitation role"),
    };
    Ok(Some(Invitation {
        id: parse_id(row.0)?,
        workspace: parse_id(row.1)?,
        role,
        initial: match row.3.as_str() {
            "initial_group" => true,
            "individual" => false,
            _ => bail!("invalid persisted invitation kind"),
        },
        issuer,
        uses: row.6,
        capacity: row.7,
        revoked: row.8,
    }))
}

fn invitation_closed(tx: &Connection, invite: &Invitation) -> anyhow::Result<bool> {
    if invite
        .capacity
        .is_some_and(|capacity| invite.uses >= capacity)
    {
        return Ok(true);
    }
    let expired: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM ccp_invitation_closures WHERE invitation_id = ?1)",
        [invite.id.to_string()],
        |row| row.get(0),
    )?;
    if expired {
        return Ok(true);
    }
    if !invite.initial {
        return Ok(false);
    }
    let open: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM ccp_initial_enrollment WHERE workspace_id = ?1
         AND invitation_id = ?2 AND closed_at IS NULL)",
        params![invite.workspace.to_string(), invite.id.to_string()],
        |row| row.get(0),
    )?;
    Ok(!open)
}

fn replay(
    tx: &Connection,
    invite: &Invitation,
    request: &RedemptionRequest,
    proof: &VerifiedCsr,
    now: i64,
) -> anyhow::Result<Option<Redemption>> {
    let row = tx
        .query_row(
            "SELECT client_id, credential_id, grant_id, public_key_hash, previous_credential_id
         FROM ccp_invitation_receipts WHERE invitation_id = ?1 AND request_id = ?2",
            params![invite.id.to_string(), request.request_id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(None);
    };
    let client = parse_id(row.0)?;
    let credential = parse_id(row.1)?;
    let grant = parse_id(row.2)?;
    let target = readmission_target(tx, invite.id, invite.workspace)?;
    if target.is_some_and(|id| id != client || request.existing_credential.is_some())
        || row.3 != proof.key_hash
        || row.4 != request.existing_credential.map(|id| id.to_string())
        || !permitted(
            tx,
            invite.issuer,
            Action::RecoverWorkspaceClient,
            Some(invite.workspace),
            now,
        )?
        || !permitted(
            tx,
            Actor::Credential(credential),
            Action::ReadContext,
            Some(invite.workspace),
            now,
        )?
    {
        return Ok(Some(Redemption::Denied));
    }
    let current: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM ccp_credential_grants b JOIN ccp_grants g ON g.id = b.grant_id
         WHERE b.credential_id = ?1 AND g.id = ?2 AND g.workspace_id = ?3 AND g.revoked_at IS NULL
         AND NOT EXISTS(SELECT 1 FROM ccp_binding_revocations v
             WHERE v.credential_id = b.credential_id AND v.grant_id = b.grant_id))",
        params![
            credential.to_string(),
            grant.to_string(),
            invite.workspace.to_string()
        ],
        |row| row.get(0),
    )?;
    if !current {
        return Ok(Some(Redemption::Denied));
    }
    let (certificate_pem, expires_at) = stored_certificate(tx, credential)?;
    Ok(Some(Redemption::Accepted(EnrollmentReceipt {
        client_id: client,
        credential_id: credential,
        grant_id: grant,
        certificate_pem,
        expires_at,
    })))
}

fn stored_certificate(tx: &Connection, credential: Uuid) -> anyhow::Result<(String, i64)> {
    Ok(tx.query_row(
        "SELECT k.certificate_pem, c.expires_at FROM ccp_credentials c
         JOIN ccp_credential_certificates k ON k.credential_id = c.id WHERE c.id = ?1",
        [credential.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?)
}

fn prepare_identity(
    tx: &Connection,
    request: &RedemptionRequest,
    target: Option<Uuid>,
    proof: &VerifiedCsr,
    authority: &IssuingAuthority,
    now: i64,
) -> anyhow::Result<Option<PreparedIdentity>> {
    if let Some(credential) = request.existing_credential {
        let row = tx.query_row(
            "SELECT c.client_id, k.certificate_pem, c.expires_at FROM ccp_credentials c
             JOIN ccp_clients p ON p.id = c.client_id JOIN ccp_credential_certificates k ON k.credential_id = c.id
             WHERE c.id = ?1 AND c.expires_at > ?2 AND c.revoked_at IS NULL AND p.disabled = 0 AND k.public_key_hash = ?3
             AND NOT EXISTS(SELECT 1 FROM ccp_credential_renewals r WHERE r.previous_credential_id = c.id)",
            params![credential.to_string(), now, proof.key_hash],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)),
        ).optional()?;
        return row
            .map(|row| {
                Ok(PreparedIdentity {
                    client_id: parse_id(row.0)?,
                    credential_id: credential,
                    certificate_pem: row.1,
                    expires_at: row.2,
                })
            })
            .transpose();
    }
    let instance = parse_id(tx.query_row(
        "SELECT instance_id FROM ccp_instance WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?)?;
    let client = target.unwrap_or_else(Uuid::new_v4);
    if target.is_some() {
        require_fresh_client_key(tx, client, &proof.key_hash)?;
    }
    let credential = Uuid::new_v4();
    let signed = authority.issue(proof, instance, client, credential, now)?;
    if target.is_none() {
        tx.execute(
            "INSERT INTO ccp_clients(id, name) VALUES (?1, ?2)",
            params![client.to_string(), format!("client-{}", client.simple())],
        )?;
    }
    tx.execute(
        "INSERT INTO ccp_credentials(id, client_id, expires_at) VALUES (?1, ?2, ?3)",
        params![
            credential.to_string(),
            client.to_string(),
            signed.expires_at
        ],
    )?;
    tx.execute(
        "INSERT INTO ccp_credential_certificates(credential_id, public_key_hash, certificate_fingerprint, certificate_pem)
         VALUES (?1, ?2, ?3, ?4)", params![credential.to_string(), signed.key_hash, signed.fingerprint, signed.pem],
    )?;
    Ok(Some(PreparedIdentity {
        client_id: client,
        credential_id: credential,
        certificate_pem: signed.pem,
        expires_at: signed.expires_at,
    }))
}

fn prepare_invited_identity(
    tx: &Connection,
    invite: &Invitation,
    request: &RedemptionRequest,
    proof: &VerifiedCsr,
    authority: &IssuingAuthority,
    now: i64,
) -> anyhow::Result<Option<PreparedIdentity>> {
    let target = readmission_target(tx, invite.id, invite.workspace)?;
    if let Some(client) = target
        && (invite.initial
            || request.existing_credential.is_some()
            || !readmission_current(tx, invite.id, client, invite.workspace)?)
    {
        return Ok(None);
    }
    let Some(identity) = prepare_identity(tx, request, target, proof, authority, now)? else {
        return Ok(None);
    };
    if target.is_none() {
        let existing: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM ccp_grants WHERE client_id = ?1 AND workspace_id = ?2)",
            params![identity.client_id.to_string(), invite.workspace.to_string()],
            |row| row.get(0),
        )?;
        if existing {
            return Ok(None);
        }
    }
    Ok(Some(identity))
}
