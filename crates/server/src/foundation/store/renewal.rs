// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use rusqlite::OptionalExtension;

use super::*;
use crate::foundation::certificate::VerifiedCsr;
use crate::foundation::{IssuingAuthority, RenewalReceipt, RenewalRequest};

impl AccessStore<'_> {
    /// Rotate a currently authorized credential, or retrieve its exact committed
    /// receipt. `actor` must identify the actual verified mTLS peer. A future
    /// retry-only TLS adapter may accept a superseded but unexpired peer here;
    /// it must never make that peer an ordinary authorized data caller.
    pub fn renew_credential(
        &mut self,
        actor: Actor,
        request: &RenewalRequest,
        authority: &IssuingAuthority,
        now: i64,
    ) -> anyhow::Result<RenewalReceipt> {
        let Actor::Credential(previous) = actor else {
            bail!("renewal requires an authenticated client credential");
        };
        if now < 0 {
            bail!("invalid server time");
        }
        let proof = VerifiedCsr::parse(&request.csr_pem)?;
        self.mutate(|tx| {
            if let Some(receipt) = renewal_retry(tx, previous, request, &proof, now)? {
                return Ok(receipt);
            }
            let client = authenticated_client(tx, previous, now)?;
            // Legacy internal registration IDs without a certified transport
            // key are not an enrollment mechanism for the new TLS interface.
            tx.query_row(
                "SELECT credential_id FROM ccp_credential_certificates WHERE credential_id = ?1",
                [previous.to_string()],
                |row| row.get::<_, String>(0),
            )?;
            let grants = bound_grants(tx, previous)?;
            let mut current = Vec::new();
            for (grant, workspace) in grants {
                if permitted(tx, actor, Action::ReadContext, Some(workspace), now)? {
                    current.push(grant);
                }
            }
            if current.is_empty() {
                bail!("renewal requires a currently readable bound workspace grant");
            }
            let credential = Uuid::new_v4();
            let instance = parse_id(tx.query_row(
                "SELECT instance_id FROM ccp_instance WHERE singleton = 1",
                [],
                |row| row.get(0),
            )?)?;
            let signed = authority.issue(&proof, instance, client, credential, now)?;
            tx.execute(
                "INSERT INTO ccp_credentials(id, client_id, expires_at) VALUES (?1, ?2, ?3)",
                params![
                    credential.to_string(),
                    client.to_string(),
                    signed.expires_at
                ],
            )?;
            tx.execute(
                "INSERT INTO ccp_credential_certificates
                 (credential_id, public_key_hash, certificate_fingerprint, certificate_pem)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    credential.to_string(),
                    signed.key_hash,
                    signed.fingerprint,
                    signed.pem
                ],
            )?;
            for grant in &current {
                tx.execute(
                    "INSERT INTO ccp_credential_grants(credential_id, grant_id, client_id)
                     VALUES (?1, ?2, ?3)",
                    params![
                        credential.to_string(),
                        grant.to_string(),
                        client.to_string()
                    ],
                )?;
            }
            tx.execute(
                "INSERT INTO ccp_credential_renewals(previous_credential_id, request_id,
                 credential_id, client_id, public_key_hash, recorded_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    previous.to_string(),
                    request.request_id.to_string(),
                    credential.to_string(),
                    client.to_string(),
                    proof.key_hash,
                    now
                ],
            )?;
            for grant in &current {
                tx.execute(
                    "INSERT INTO ccp_renewal_grants(previous_credential_id, grant_id, client_id)
                     VALUES (?1, ?2, ?3)",
                    params![previous.to_string(), grant.to_string(), client.to_string()],
                )?;
            }
            audit(tx, actor, "renew_credential", None, credential, now)?;
            Ok(RenewalReceipt {
                client_id: client,
                credential_id: credential,
                grant_ids: current,
                certificate_pem: signed.pem,
                expires_at: signed.expires_at,
            })
        })
    }
}

fn bound_grants(connection: &Connection, credential: Uuid) -> anyhow::Result<Vec<(Uuid, Uuid)>> {
    connection
        .prepare(
            "SELECT g.id, g.workspace_id FROM ccp_credential_grants b
             JOIN ccp_grants g ON g.id = b.grant_id AND g.client_id = b.client_id
             WHERE b.credential_id = ?1 AND g.revoked_at IS NULL ORDER BY g.id",
        )?
        .query_map([credential.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|(grant, workspace)| Ok((parse_id(grant)?, parse_id(workspace)?)))
        .collect()
}

fn renewal_retry(
    tx: &Connection,
    previous: Uuid,
    request: &RenewalRequest,
    proof: &VerifiedCsr,
    now: i64,
) -> anyhow::Result<Option<RenewalReceipt>> {
    let row = tx
        .query_row(
            "SELECT r.request_id, r.credential_id, r.public_key_hash, c.revoked_at,
         c.expires_at FROM ccp_credential_renewals r
         JOIN ccp_credentials c ON c.id = r.previous_credential_id
         WHERE r.previous_credential_id = ?1",
            [previous.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((request_id, credential, hash, revoked, expires)) = row else {
        return Ok(None);
    };
    if request_id != request.request_id.to_string()
        || hash != proof.key_hash
        || revoked.is_some()
        || expires <= now
    {
        bail!("renewal retry does not match a current supersession receipt");
    }
    let credential = parse_id(credential)?;
    let client = authenticated_client(tx, credential, now)?;
    // At least one captured binding must remain readable. Returning the public
    // certificate cannot revive another revoked grant or change its role.
    let captured = tx
        .prepare(
            "SELECT g.id, g.workspace_id, g.revoked_at IS NULL FROM ccp_renewal_grants r
             JOIN ccp_grants g ON g.id = r.grant_id AND g.client_id = r.client_id
             WHERE r.previous_credential_id = ?1 ORDER BY g.id",
        )?
        .query_map([previous.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut readable = false;
    let mut grant_ids = Vec::new();
    for (grant, workspace, live) in captured {
        grant_ids.push(parse_id(grant)?);
        if live {
            readable |= permitted(
                tx,
                Actor::Credential(credential),
                Action::ReadContext,
                Some(parse_id(workspace)?),
                now,
            )?;
        }
    }
    if !readable {
        bail!("renewed credential no longer has a readable captured grant");
    }
    let (certificate_pem, expires_at) = tx.query_row(
        "SELECT k.certificate_pem, c.expires_at FROM ccp_credentials c
         JOIN ccp_credential_certificates k ON k.credential_id = c.id WHERE c.id = ?1",
        [credential.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(Some(RenewalReceipt {
        client_id: client,
        credential_id: credential,
        grant_ids,
        certificate_pem,
        expires_at,
    }))
}
