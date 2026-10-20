// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{Credential, Pending, WorkspaceProfile, WorkspaceTrust, crypto};
use anyhow::{Context, bail};
use rusqlite::{Connection, OpenFlags, params};
use serde::de::DeserializeOwned;
use std::{fs, path::Path, time::Duration};
use uuid::Uuid;

const APPLICATION_ID: i64 = 0x4343_5043;
const PROFILE: &str = "workspace-profile.sqlite3";

pub(super) fn create(directory: &Path, trust: &WorkspaceTrust) -> anyhow::Result<()> {
    private_directory(directory, true)?;
    let target = directory.join(PROFILE);
    if target.try_exists()? {
        bail!("workspace profile already exists; it will not be replaced");
    }
    let temporary = directory.join(format!(".workspace-profile-{}.sqlite3", Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&temporary)?.sync_all()?;
    let result = initialize(&temporary, trust).and_then(|()| {
        fs::hard_link(&temporary, &target)
            .context("cannot publish workspace profile without replacement")
    });
    fs::remove_file(&temporary).context("cannot remove private profile staging file")?;
    result?;
    sync_directory(directory)
}

fn initialize(path: &Path, trust: &WorkspaceTrust) -> anyhow::Result<()> {
    let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    connection.pragma_update(None, "journal_mode", "DELETE")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    let tx = connection.transaction()?;
    tx.pragma_update(None, "application_id", APPLICATION_ID)?;
    tx.pragma_update(None, "user_version", 1)?;
    tx.execute_batch("CREATE TABLE profile(singleton INTEGER PRIMARY KEY CHECK(singleton=1),trust TEXT NOT NULL,pending TEXT,credential TEXT)")?;
    tx.execute(
        "INSERT INTO profile(singleton,trust) VALUES (1,?1)",
        [serde_json::to_string(trust)?],
    )?;
    tx.commit()?;
    Ok(())
}

pub(super) fn open(directory: &Path) -> anyhow::Result<WorkspaceProfile> {
    private_directory(directory, false)?;
    let path = directory.join(PROFILE);
    private_file(&path)?;
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    connection.busy_timeout(Duration::from_secs(2))?;
    format(&connection)?;
    let mode: String = connection.pragma_query_value(None, "journal_mode", |r| r.get(0))?;
    if mode != "delete" {
        bail!("unsupported workspace profile journal mode");
    }
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
    connection
        .execute_batch("BEGIN EXCLUSIVE; COMMIT;")
        .context("workspace profile is busy or unavailable")?;
    format(&connection)?;
    let (trust, pending, credential) = connection.query_row(
        "SELECT trust,pending,credential FROM profile WHERE singleton=1",
        [],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        },
    )?;
    let trust: WorkspaceTrust = decode(&trust)?;
    trust.validate()?;
    let pending: Option<Pending> = pending.as_deref().map(decode).transpose()?;
    let credential: Option<Credential> = credential.as_deref().map(decode).transpose()?;
    if pending.is_some() && credential.is_some() {
        bail!("unsupported simultaneous pending and active profile state");
    }
    if let Some(pending) = &pending {
        crypto::validate_pending(pending)?;
    }
    let profile = WorkspaceProfile {
        connection,
        trust,
        pending,
        credential,
    };
    profile.info()?;
    Ok(profile)
}

fn format(connection: &Connection) -> anyhow::Result<()> {
    let app: i64 = connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if app != APPLICATION_ID || version != 1 {
        bail!("unrecognized or unsupported workspace profile format");
    }
    Ok(())
}
fn decode<T: DeserializeOwned>(value: &str) -> anyhow::Result<T> {
    serde_json::from_str(value)
        .map_err(|_private_data| anyhow::anyhow!("invalid workspace profile data"))
}

impl WorkspaceProfile {
    pub(super) fn prepare(&mut self, invitation: &str) -> anyhow::Result<()> {
        if let Some(pending) = &self.pending {
            if pending.invitation_hash != crypto::hash(invitation.as_bytes()) {
                bail!(
                    "pending enrollment requires its original invitation; do not replace its key or request"
                );
            }
            crypto::validate_pending(pending)?;
            return Ok(());
        }
        let pending = crypto::pending(invitation)?;
        let changed=self.connection.execute("UPDATE profile SET pending=?1 WHERE singleton=1 AND pending IS NULL AND credential IS NULL",
            [serde_json::to_string(&pending)?]).map_err(|_write|anyhow::anyhow!("cannot persist pending enrollment; no request was sent"))?;
        if changed != 1 {
            bail!("workspace profile state changed");
        }
        self.pending = Some(pending);
        Ok(())
    }
    pub(super) fn activate(&mut self, credential: Credential) -> anyhow::Result<()> {
        let pending = self
            .pending
            .as_ref()
            .context("missing pending enrollment")?;
        if credential.key_pem != pending.key_pem {
            bail!("credential key differs from durable pending enrollment");
        }
        crypto::credential(&self.trust, &credential, crypto::now()?)?;
        let tx = self.connection.transaction()?;
        let changed=tx.execute("UPDATE profile SET credential=?1,pending=NULL WHERE singleton=1 AND pending=?2 AND credential IS NULL",
            params![serde_json::to_string(&credential)?,serde_json::to_string(pending)?])
            .map_err(|_write|anyhow::anyhow!("cannot activate credential; pending enrollment retained for retry"))?;
        if changed != 1 {
            bail!("workspace profile state changed; preserve pending enrollment");
        }
        tx.commit().map_err(|_commit| {
            anyhow::anyhow!("credential activation failed; inspect profile before retry")
        })?;
        self.pending = None;
        self.credential = Some(credential);
        Ok(())
    }
}

fn private_directory(path: &Path, create: bool) -> anyhow::Result<()> {
    if create && !path.try_exists()? {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(path)?;
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("profile directory must be a real private directory");
    }
    #[cfg(unix)]
    check_permissions(&metadata)?;
    Ok(())
}
pub(super) fn private_file(path: &Path) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 4 * 1024 * 1024
    {
        bail!("invalid private profile file");
    }
    #[cfg(unix)]
    check_permissions(&metadata)?;
    Ok(())
}
#[cfg(unix)]
fn check_permissions(metadata: &fs::Metadata) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if metadata.permissions().mode() & 0o077 != 0 {
        bail!("profile material must have owner-only Unix permissions");
    }
    Ok(())
}
fn sync_directory(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        if !path.is_dir() {
            bail!("missing profile directory");
        }
    }
    Ok(())
}
