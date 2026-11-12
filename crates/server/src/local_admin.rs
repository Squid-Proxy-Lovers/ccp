// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Explicit host-operator setup and recovery. No network route may expose these
//! functions: data-directory write access and exclusive maintenance are the
//! authority boundary, distinct from scoped administrative bearer keys.

use std::path::Path;

use anyhow::{Context, bail};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use uuid::Uuid;

use crate::directory_mode::{DirectoryLease, DirectoryMode};
use crate::foundation::{AccessStore, BootstrappedInstance, LocalKeyReset, LocalKeyTarget};

pub fn bootstrap(
    directory: &Path,
    operator: &str,
    workspace: &str,
) -> anyhow::Result<BootstrappedInstance> {
    with_store(directory, false, |store, now| {
        store.bootstrap_instance(operator, workspace, now)
    })
}

pub fn reset_key(
    directory: &Path,
    target: LocalKeyTarget,
    ttl_seconds: Option<u64>,
) -> anyhow::Result<LocalKeyReset> {
    if ttl_seconds == Some(0) {
        bail!("replacement key TTL must be positive");
    }
    with_store(directory, true, |store, now| {
        let expires_at = ttl_seconds
            .map(|ttl| {
                let ttl = i64::try_from(ttl).context("replacement key TTL is too large")?;
                now.checked_add(ttl)
                    .context("replacement key expiry overflow")
            })
            .transpose()?;
        store.reset_local_admin_key(target, expires_at, now)
    })
}

fn with_store<T>(
    directory: &Path,
    existing: bool,
    operation: impl FnOnce(&mut AccessStore<'_>, i64) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let path = directory.join("ccp.sqlite3");
    if existing && !path.try_exists()? {
        bail!("instance database does not exist");
    }
    let lease = DirectoryLease::maintenance(directory)?;
    let path = lease.directory().join("ccp.sqlite3");
    crate::init::init_sqlite(&path)?;
    let mut connection = Connection::open(path)?;
    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs(),
    )?;
    operation(&mut AccessStore::new(&mut connection)?, now)
}

/// Metadata only, deliberately excluding key verifiers, secrets and contents.
#[derive(Serialize)]
pub struct LocalAdminInfo {
    pub instance_id: String,
    pub mode: &'static str,
    pub global_admins: Vec<LocalPrincipal>,
    pub workspaces: Vec<LocalWorkspace>,
}

#[derive(Serialize)]
pub struct LocalPrincipal {
    pub id: String,
    pub name: String,
    pub enabled: bool,
}

#[derive(Serialize)]
pub struct LocalWorkspace {
    pub id: String,
    pub name: String,
    pub state: String,
    pub owner_id: Option<String>,
}

pub fn info(directory: &Path) -> anyhow::Result<LocalAdminInfo> {
    if !directory.join("ccp.sqlite3").try_exists()? {
        bail!("instance database does not exist");
    }
    let lease = DirectoryLease::maintenance(directory)?;
    let connection = Connection::open_with_flags(
        lease.directory().join("ccp.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    crate::init::reject_unsupported_schema(&connection)?;
    let initialized: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'ccp_instance' AND type = 'table')",
        [],
        |row| row.get(0),
    )?;
    if !initialized {
        bail!("workspace administration is not initialized; run workspace-init");
    }
    let instance_id: String = connection.query_row(
        "SELECT instance_id FROM ccp_instance WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    Uuid::parse_str(&instance_id).context("invalid stored instance UUID")?;
    let global_admins = connection.prepare("SELECT p.id, p.name, p.enabled FROM ccp_admin_principals p JOIN ccp_server_admins a ON a.admin_id = p.id ORDER BY p.name, p.id")?
        .query_map([], |row| Ok(LocalPrincipal { id: row.get(0)?, name: row.get(1)?, enabled: row.get(2)? }))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let workspaces = connection
        .prepare("SELECT id, name, state, owner_admin_id FROM ccp_workspaces ORDER BY name, id")?
        .query_map([], |row| {
            Ok(LocalWorkspace {
                id: row.get(0)?,
                name: row.get(1)?,
                state: row.get(2)?,
                owner_id: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for admin in &global_admins {
        Uuid::parse_str(&admin.id).context("invalid stored administrator UUID")?;
    }
    for workspace in &workspaces {
        Uuid::parse_str(&workspace.id).context("invalid stored workspace UUID")?;
        if let Some(owner) = &workspace.owner_id {
            Uuid::parse_str(owner).context("invalid stored owner UUID")?;
        }
    }
    let mode = match lease.mode() {
        DirectoryMode::Native => "native",
        DirectoryMode::Workspace => "workspace",
    };
    Ok(LocalAdminInfo {
        instance_id,
        mode,
        global_admins,
        workspaces,
    })
}
