// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

pub(super) fn load_recovery(tx: &Connection, id: Uuid) -> anyhow::Result<RecoveryInvitation> {
    let row = tx.query_row(
        "SELECT client_id, workspace_id, issuer_kind, issuer_id, expires_at,
         revoked_at IS NOT NULL, expired_at IS NOT NULL FROM ccp_recovery_invitations WHERE id = ?1",
        [id.to_string()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?, row.get::<_, bool>(5)?, row.get::<_, bool>(6)?)),
    )?;
    let issuer = parse_id(row.3)?;
    let issuer = match row.2.as_str() {
        "admin" => Actor::Admin(issuer),
        "admin_key" => Actor::AdminKey(issuer),
        _ => bail!("invalid persisted recovery issuer"),
    };
    Ok(RecoveryInvitation {
        id,
        client: parse_id(row.0)?,
        scope: match row.1 {
            Some(id) => RecoveryScope::Workspace(parse_id(id)?),
            None => RecoveryScope::Instance,
        },
        issuer,
        expires_at: row.4,
        revoked: row.5,
        expired: row.6,
    })
}

pub(super) fn replace_bindings(
    tx: &Connection,
    invite: &RecoveryInvitation,
    request: &RecoveryRedemptionRequest,
    proof: &VerifiedCsr,
    authority: &IssuingAuthority,
    now: i64,
) -> anyhow::Result<RecoveryReceipt> {
    require_fresh_recovery_key(tx, invite.client, &proof.key_hash)?;
    let grants = tx
        .prepare(
            "SELECT g.id FROM ccp_recovery_grants r
         JOIN ccp_grants g ON g.id = r.grant_id AND g.client_id = r.client_id
         JOIN ccp_workspaces w ON w.id = g.workspace_id
         WHERE r.invitation_id = ?1 AND g.revoked_at IS NULL AND w.state != 'purged'
         AND (?2 IS NULL OR g.workspace_id = ?2) ORDER BY g.id",
        )?
        .query_map(
            params![
                invite.id.to_string(),
                invite.scope.workspace().map(|id| id.to_string())
            ],
            |row| row.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if grants.is_empty() {
        bail!("recovery has no remaining captured authorized grants");
    }
    let credential = Uuid::new_v4();
    let instance = parse_id(tx.query_row(
        "SELECT instance_id FROM ccp_instance WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?)?;
    let signed = authority.issue(proof, instance, invite.client, credential, now)?;
    tx.execute(
        "INSERT INTO ccp_credentials(id, client_id, expires_at) VALUES (?1, ?2, ?3)",
        params![
            credential.to_string(),
            invite.client.to_string(),
            signed.expires_at
        ],
    )?;
    tx.execute("INSERT INTO ccp_credential_certificates(credential_id, public_key_hash, certificate_fingerprint, certificate_pem)
        VALUES (?1, ?2, ?3, ?4)", params![credential.to_string(), signed.key_hash, signed.fingerprint, signed.pem])?;
    for grant in &grants {
        tx.execute("INSERT INTO ccp_binding_revocations(credential_id, grant_id, revoked_at, recovery_invitation_id)
            SELECT credential_id, grant_id, ?1, ?2 FROM ccp_credential_grants WHERE grant_id = ?3
             ON CONFLICT(credential_id, grant_id) DO NOTHING",
            params![now, invite.id.to_string(), grant])?;
        tx.execute("INSERT INTO ccp_credential_grants(credential_id, grant_id, client_id) VALUES (?1, ?2, ?3)",
            params![credential.to_string(), grant, invite.client.to_string()])?;
    }
    tx.execute("INSERT INTO ccp_recovery_receipts(invitation_id, request_id, credential_id, client_id, public_key_hash, recorded_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![invite.id.to_string(), request.request_id.to_string(), credential.to_string(),
            invite.client.to_string(), proof.key_hash, now])?;
    for grant in grants {
        tx.execute("INSERT INTO ccp_recovery_receipt_grants(invitation_id, grant_id, credential_id) VALUES (?1, ?2, ?3)",
            params![invite.id.to_string(), grant, credential.to_string()])?;
    }
    recovered_receipt(tx, invite, credential)
}

pub(super) fn recovered_receipt(
    tx: &Connection,
    invite: &RecoveryInvitation,
    credential: Uuid,
) -> anyhow::Result<RecoveryReceipt> {
    let (certificate_pem, expires_at) = tx.query_row(
        "SELECT k.certificate_pem, c.expires_at FROM ccp_credentials c JOIN ccp_credential_certificates k ON k.credential_id = c.id
         WHERE c.id = ?1 AND c.client_id = ?2", params![credential.to_string(), invite.client.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let grant_ids = tx.prepare("SELECT grant_id FROM ccp_recovery_receipt_grants WHERE invitation_id = ?1 ORDER BY grant_id")?
        .query_map([invite.id.to_string()], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter().map(parse_id).collect::<anyhow::Result<Vec<_>>>()?;
    Ok(RecoveryReceipt {
        client_id: invite.client,
        credential_id: credential,
        grant_ids,
        certificate_pem,
        expires_at,
    })
}

pub(super) fn recovery_retry(
    tx: &Connection,
    invite: &RecoveryInvitation,
    request: &RecoveryRedemptionRequest,
    proof: &VerifiedCsr,
    now: i64,
) -> anyhow::Result<Option<RecoveryReceipt>> {
    let row = tx.query_row("SELECT request_id, credential_id, public_key_hash FROM ccp_recovery_receipts WHERE invitation_id = ?1",
        [invite.id.to_string()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Vec<u8>>(2)?))).optional()?;
    let Some((request_id, credential, hash)) = row else {
        return Ok(None);
    };
    if request_id != request.request_id.to_string() || hash != proof.key_hash {
        bail!("recovery invitation already redeemed with another request or key");
    }
    let credential = parse_id(credential)?;
    let client = authenticated_client(tx, credential, now)?;
    if client != invite.client {
        bail!("recovery receipt identity mismatch");
    }
    let scopes = tx
        .prepare(
            "SELECT g.workspace_id FROM ccp_recovery_receipt_grants r
         JOIN ccp_grants g ON g.id = r.grant_id JOIN ccp_credential_grants b
         ON b.grant_id = g.id AND b.credential_id = r.credential_id
         WHERE r.invitation_id = ?1 AND r.credential_id = ?2 AND g.revoked_at IS NULL
         AND NOT EXISTS(SELECT 1 FROM ccp_binding_revocations v
             WHERE v.credential_id = b.credential_id AND v.grant_id = b.grant_id)",
        )?
        .query_map(
            params![invite.id.to_string(), credential.to_string()],
            |row| row.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut readable = false;
    for scope in scopes {
        readable |= permitted(
            tx,
            Actor::Credential(credential),
            Action::ReadContext,
            Some(parse_id(scope)?),
            now,
        )?;
    }
    if !readable {
        bail!("recovery receipt has no current captured readable binding");
    }
    recovered_receipt(tx, invite, credential).map(Some)
}

fn require_fresh_recovery_key(
    tx: &Connection,
    client: Uuid,
    key_hash: &[u8],
) -> anyhow::Result<()> {
    // All previously registered target-client keys count, even when their
    // credentials expired, were revoked/superseded or retain only another
    // workspace. Otherwise recovery could reuse a known compromised key.
    let reused: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM ccp_credential_certificates k
         JOIN ccp_credentials c ON c.id = k.credential_id
         WHERE c.client_id = ?1 AND k.public_key_hash = ?2)",
        params![client.to_string(), key_hash],
        |row| row.get(0),
    )?;
    if reused {
        bail!("recovery requires a key not previously registered to this client");
    }
    Ok(())
}
