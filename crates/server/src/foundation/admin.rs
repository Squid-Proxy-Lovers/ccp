// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::bail;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::parse_id;

const KEY_PREFIX: &str = "ccp_admin_";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdminKeyScope {
    Instance,
    Workspace(Uuid),
}

/// Current browser-session metadata, never a remote actor claim.
pub(crate) struct AdminKeyIdentity {
    pub key_id: Uuid,
    pub principal_id: Uuid,
    pub principal_name: String,
    pub key_name: String,
    pub scope: AdminKeyScope,
    pub is_owner: bool,
}

impl AdminKeyScope {
    pub(super) fn workspace(self) -> Option<Uuid> {
        match self {
            Self::Instance => None,
            Self::Workspace(id) => Some(id),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AdminKeyRequest {
    pub principal_id: Uuid,
    pub scope: AdminKeyScope,
    pub name: String,
    pub expires_at: Option<i64>,
}

/// A secret returned only by issuance. Deliberately has no Debug or Serialize
/// implementation: API adapters must explicitly choose when to disclose it.
pub struct IssuedAdminKey {
    pub id: Uuid,
    pub principal_id: Uuid,
    secret: String,
}

impl IssuedAdminKey {
    pub fn secret(&self) -> &str {
        &self.secret
    }

    pub(super) fn generate(principal_id: Uuid) -> anyhow::Result<Self> {
        let mut random = [0_u8; 32];
        getrandom::getrandom(&mut random)
            .map_err(|error| anyhow::anyhow!("failed to generate admin key: {error}"))?;
        let hex: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self {
            id: Uuid::new_v4(),
            principal_id,
            secret: format!("{KEY_PREFIX}{hex}"),
        })
    }
}

pub(super) fn key_hash(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}

pub(super) fn validate_secret(secret: &str) -> anyhow::Result<()> {
    let Some(hex) = secret.strip_prefix(KEY_PREFIX) else {
        bail!("admin key is not valid");
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("admin key is not valid");
    }
    Ok(())
}

/// Checks the principal's current role within the key's own scope. Global
/// read-all must not keep a revoked workspace-admin key alive.
pub(super) fn has_key_role(
    connection: &Connection,
    principal: Uuid,
    workspace: Option<Uuid>,
) -> anyhow::Result<bool> {
    let enabled: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM ccp_admin_principals WHERE id = ?1 AND enabled = 1)",
        [principal.to_string()],
        |row| row.get(0),
    )?;
    if !enabled {
        return Ok(false);
    }
    match workspace {
        None => Ok(connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM ccp_server_admins WHERE admin_id = ?1)",
            [principal.to_string()], |row| row.get(0),
        )?),
        Some(workspace) => Ok(connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM ccp_workspaces w WHERE w.id = ?1
             AND w.state != 'purged' AND (w.owner_admin_id = ?2 OR EXISTS(
             SELECT 1 FROM ccp_workspace_admins a WHERE a.workspace_id = w.id AND a.admin_id = ?2)))",
            params![workspace.to_string(), principal.to_string()], |row| row.get(0),
        )?),
    }
}

pub(super) fn live_admin_key(
    connection: &Connection,
    key: Uuid,
    now: i64,
) -> anyhow::Result<Option<(Uuid, Option<Uuid>)>> {
    if now < 0 {
        bail!("invalid server time");
    }
    let record: Option<(String, Option<String>)> = connection
        .query_row(
            "SELECT admin_id, workspace_id FROM ccp_admin_keys WHERE id = ?1
         AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > ?2)",
            params![key.to_string(), now],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((principal, workspace)) = record else {
        return Ok(None);
    };
    let principal = parse_id(principal)?;
    let workspace = workspace.map(parse_id).transpose()?;
    if !has_key_role(connection, principal, workspace)? {
        return Ok(None);
    }
    Ok(Some((principal, workspace)))
}

/// Initial local setup secrets. No implicit Debug/Serialize/Clone disclosure.
pub struct BootstrappedInstance {
    pub instance_id: Uuid,
    pub workspace_id: Uuid,
    pub instance_key: IssuedAdminKey,
    pub owner_key: IssuedAdminKey,
}

#[derive(Clone, Copy, Debug)]
pub enum LocalKeyTarget {
    InstancePrincipal(Uuid),
    WorkspaceOwner(Uuid),
}

/// A replacement returned once to an explicitly authorized host operator.
pub struct LocalKeyReset {
    pub key: IssuedAdminKey,
    pub scope: AdminKeyScope,
    pub expires_at: Option<i64>,
    pub revoked_keys: usize,
}
