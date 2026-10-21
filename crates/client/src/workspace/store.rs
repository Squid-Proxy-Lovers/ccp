// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{Credential, Operation, Pending, WorkspaceProfile, WorkspaceTrust, crypto};

mod migration;
use anyhow::{Context, bail};
use rusqlite::{Connection, OpenFlags, params};
use serde::de::DeserializeOwned;
use std::{fs, path::Path, time::Duration};
use uuid::Uuid;

const APPLICATION_ID: i64 = 0x4343_5043;
pub(super) const PROFILE_FORMAT: u32 = 5;
const CONTENT_SCHEMA: &str = "CREATE TABLE content_state(singleton INTEGER PRIMARY KEY CHECK(singleton=1),pending TEXT CHECK(pending IS NULL OR length(CAST(pending AS BLOB))<=2101248),last TEXT CHECK(last IS NULL OR length(CAST(last AS BLOB))<=4096)); INSERT INTO content_state(singleton) VALUES(1);";
const STATUS_SCHEMA: &str = r"CREATE TABLE status_state(singleton INTEGER PRIMARY KEY CHECK(singleton=1),pending TEXT CHECK(pending IS NULL OR length(CAST(pending AS BLOB))<=69632),last TEXT CHECK(last IS NULL OR length(CAST(last AS BLOB))<=4096)); INSERT INTO status_state(singleton) VALUES(1);
CREATE TRIGGER status_pending_excludes_other BEFORE UPDATE OF pending ON status_state WHEN NEW.pending IS NOT NULL AND (EXISTS(SELECT 1 FROM content_state WHERE pending IS NOT NULL) OR EXISTS(SELECT 1 FROM profile WHERE pending IS NOT NULL)) BEGIN SELECT RAISE(ABORT,'pending status conflicts with another operation'); END;
CREATE TRIGGER content_pending_excludes_status BEFORE UPDATE OF pending ON content_state WHEN NEW.pending IS NOT NULL AND EXISTS(SELECT 1 FROM status_state WHERE pending IS NOT NULL) BEGIN SELECT RAISE(ABORT,'pending entry conflicts with status'); END;
CREATE TRIGGER lifecycle_pending_excludes_status BEFORE UPDATE OF pending,credential ON profile WHEN EXISTS(SELECT 1 FROM status_state WHERE pending IS NOT NULL) BEGIN SELECT RAISE(ABORT,'credential cannot change while status is pending'); END;
";
const PROFILE: &str = "workspace-profile.sqlite3";
const HISTORY_SCHEMA: &str = "CREATE TABLE completed_invitations(invitation_hash TEXT PRIMARY KEY, operation TEXT NOT NULL CHECK(operation IN ('enrollment','admission','recovery')))";

pub(super) fn create(directory: &Path, trust: &WorkspaceTrust) -> anyhow::Result<()> {
    publish(directory, PROFILE, |path| initialize(path, trust))
}

pub(super) fn publish(
    directory: &Path,
    filename: &str,
    initialize: impl FnOnce(&Path) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    private_directory(directory, true)?;
    let target = directory.join(filename);
    if target.try_exists()? {
        bail!("private state already exists; it will not be replaced");
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
    let result = initialize(&temporary).and_then(|()| {
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
    tx.pragma_update(None, "user_version", PROFILE_FORMAT)?;
    tx.execute_batch("CREATE TABLE profile(singleton INTEGER PRIMARY KEY CHECK(singleton=1),trust TEXT NOT NULL,pending TEXT,credential TEXT)")?;
    tx.execute_batch(HISTORY_SCHEMA)?;
    tx.execute_batch(CONTENT_SCHEMA)?;
    tx.execute_batch(STATUS_SCHEMA)?;
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
    private_file_limit(&path, 16 * 1024 * 1024)?;
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
    let version = format(&connection)?;
    if version < 4 {
        private_file(&directory.join(PROFILE))?;
    }
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
    let saved_pending = pending.clone();
    let saved_credential = credential.clone();
    let pending: Option<Pending> = pending.as_deref().map(decode).transpose()?;
    let credential: Option<Credential> = credential.as_deref().map(decode).transpose()?;
    if version == 1 && pending.is_some() && credential.is_some() {
        bail!("unsupported simultaneous pending and active profile state");
    }
    if version < 3
        && pending
            .as_ref()
            .is_some_and(|p| p.operation == Operation::Recovery)
    {
        bail!("recovery state requires profile format 3");
    }
    crypto::validate_state(pending.as_ref(), credential.as_ref())?;
    let content_state = super::content::mutation::State::load(&connection, version)?;
    content_state.validate(&trust, credential.as_ref(), pending.is_some())?;
    let status_state = super::work_status::State::load(&connection, version)?;
    status_state.validate(
        &trust,
        credential.as_ref(),
        pending.is_some() || content_state.is_pending(),
    )?;
    let mut profile = WorkspaceProfile {
        connection,
        trust,
        pending,
        credential,
        pending_record: saved_pending,
        credential_record: saved_credential,
        content_state,
        status_state,
    };
    profile.info()?;
    if version < PROFILE_FORMAT {
        migration::upgrade(&mut profile, directory, version)?;
    }
    Ok(profile)
}

fn format(connection: &Connection) -> anyhow::Result<u32> {
    let app: i64 = connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if app != APPLICATION_ID || !(1..=i64::from(PROFILE_FORMAT)).contains(&version) {
        bail!("unrecognized or unsupported workspace profile format");
    }
    Ok(u32::try_from(version)?)
}
fn decode<T: DeserializeOwned>(value: &str) -> anyhow::Result<T> {
    serde_json::from_str(value)
        .map_err(|_private_data| anyhow::anyhow!("invalid workspace profile data"))
}

impl WorkspaceProfile {
    pub(super) fn prepare(&mut self, invitation: &str) -> anyhow::Result<()> {
        self.prepare_operation(Operation::Enrollment, Some(invitation))
    }
    pub(super) fn prepare_operation(
        &mut self,
        operation: Operation,
        invitation: Option<&str>,
    ) -> anyhow::Result<()> {
        self.require_no_content_pending()?;
        if operation == Operation::Recovery {
            bail!("recovery requires an explicit expected client identity");
        }
        if let Some(pending) = &self.pending {
            if pending.operation != operation
                || pending.invitation_hash != invitation.map(|s| crypto::hash(s.as_bytes()))
            {
                bail!(
                    "finish the original pending operation with its original invitation before starting another"
                );
            }
            return crypto::validate_state(self.pending.as_ref(), self.credential.as_ref());
        }
        if let Some(invitation) = invitation {
            self.unused_invitation(invitation)?;
        }
        let pending = if operation == Operation::Enrollment {
            crypto::pending(invitation.context("enrollment needs an invitation")?)?
        } else {
            crypto::pending_operation(operation, invitation, self.credential.as_ref())?
        };
        self.persist_pending(pending)
    }

    pub(super) fn prepare_recovery(
        &mut self,
        client: &str,
        invitation: &str,
    ) -> anyhow::Result<()> {
        self.require_no_content_pending()?;
        crypto::canonical_id(client)?;
        if self.credential.is_some() {
            bail!(
                "recovery requires a separate trusted empty profile; preserve the original profile"
            );
        }
        if let Some(pending) = &self.pending {
            if pending.operation != Operation::Recovery
                || pending.expected_client_id.as_deref() != Some(client)
                || pending.invitation_hash.as_deref() != Some(&crypto::hash(invitation.as_bytes()))
            {
                bail!(
                    "finish the original pending operation with its original target and invitation"
                );
            }
            return crypto::validate_state(self.pending.as_ref(), None);
        }
        self.unused_invitation(invitation)?;
        let mut pending = crypto::pending_operation(Operation::Recovery, Some(invitation), None)?;
        pending.expected_client_id = Some(client.to_owned());
        self.persist_pending(pending)
    }

    fn unused_invitation(&self, invitation: &str) -> anyhow::Result<()> {
        let completed: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM completed_invitations WHERE invitation_hash=?1)",
            [crypto::hash(invitation.as_bytes())],
            |r| r.get(0),
        )?;
        if completed {
            bail!("this invitation already completed locally; inspect current workspaces");
        }
        Ok(())
    }

    fn persist_pending(&mut self, pending: Pending) -> anyhow::Result<()> {
        crypto::validate_state(Some(&pending), self.credential.as_ref())?;
        let record = serde_json::to_string(&pending)?;
        let changed=self.connection.execute("UPDATE profile SET pending=?1 WHERE singleton=1 AND pending IS NULL AND credential IS ?2 AND NOT EXISTS(SELECT 1 FROM content_state WHERE pending IS NOT NULL) AND NOT EXISTS(SELECT 1 FROM status_state WHERE pending IS NOT NULL)",
            params![record,self.credential_record]).map_err(|_write|anyhow::anyhow!("cannot persist pending operation; no request was sent"))?;
        if changed != 1 {
            bail!("workspace profile state changed");
        }
        self.pending = Some(pending);
        self.pending_record = Some(record);
        Ok(())
    }
    pub(super) fn activate(&mut self, credential: Credential) -> anyhow::Result<()> {
        self.require_no_content_pending()?;
        let pending = self
            .pending
            .as_ref()
            .context("missing pending enrollment")?;
        if credential.key_pem != pending.key_pem {
            bail!("credential key differs from durable pending enrollment");
        }
        crypto::validate_state(self.pending.as_ref(), self.credential.as_ref())?;
        super::lifecycle::validate_result(pending, &credential, self.credential.as_ref())?;
        crypto::credential(&self.trust, &credential, crypto::now()?)?;
        let record = serde_json::to_string(&credential)?;
        let tx = self.connection.transaction()?;
        let changed=tx.execute("UPDATE profile SET credential=?1,pending=NULL WHERE singleton=1 AND pending IS ?2 AND credential IS ?3 AND NOT EXISTS(SELECT 1 FROM content_state WHERE pending IS NOT NULL) AND NOT EXISTS(SELECT 1 FROM status_state WHERE pending IS NOT NULL)",
            params![record,self.pending_record,self.credential_record])
            .map_err(|_write|anyhow::anyhow!("cannot activate credential; pending operation retained for retry"))?;
        if changed != 1 {
            bail!("workspace profile state changed; preserve pending operation");
        }
        if let Some(hash) = &pending.invitation_hash {
            tx.execute(
                "INSERT INTO completed_invitations(invitation_hash,operation) VALUES (?1,?2)",
                params![hash, pending.operation.name()],
            )
            .map_err(|_history| {
                anyhow::anyhow!("cannot record completed operation; pending retained")
            })?;
        }
        tx.commit().map_err(|_commit| {
            anyhow::anyhow!("credential activation failed; inspect profile before retry")
        })?;
        self.pending = None;
        self.pending_record = None;
        self.credential_record = Some(record);
        self.credential = Some(credential);
        Ok(())
    }
}

pub(super) fn private_directory(path: &Path, create: bool) -> anyhow::Result<()> {
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
    private_file_limit(path, 4 * 1024 * 1024)
}
fn private_file_limit(path: &Path, limit: u64) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit {
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
