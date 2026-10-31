// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use std::io::Write;

pub(super) fn upgrade(
    profile: &mut WorkspaceProfile,
    directory: &Path,
    version: u32,
) -> anyhow::Result<()> {
    snapshot(directory, version)?;
    if version >= 3 {
        let tx = profile.connection.transaction()?;
        if version == 3 {
            tx.execute_batch(CONTENT_SCHEMA)?;
        }
        tx.execute_batch(STATUS_SCHEMA)?;
        tx.pragma_update(None, "user_version", PROFILE_FORMAT)?;
        tx.commit()?;
        return Ok(());
    }
    let pending = profile
        .pending
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let credential = profile
        .credential
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let tx = profile.connection.transaction()?;
    if version == 2 {
        tx.execute_batch(
            "ALTER TABLE completed_invitations RENAME TO previous_completed_invitations",
        )?;
    }
    tx.execute_batch(HISTORY_SCHEMA)?;
    if version == 2 {
        tx.execute_batch("INSERT INTO completed_invitations SELECT invitation_hash,operation FROM previous_completed_invitations; DROP TABLE previous_completed_invitations")?;
    }
    let changed=tx.execute("UPDATE profile SET pending=?1,credential=?2 WHERE singleton=1 AND pending IS ?3 AND credential IS ?4",
        params![pending,credential,profile.pending_record,profile.credential_record])?;
    if changed != 1 {
        bail!("profile changed before format upgrade");
    }
    tx.execute_batch(CONTENT_SCHEMA)?;
    tx.execute_batch(STATUS_SCHEMA)?;
    tx.pragma_update(None, "user_version", PROFILE_FORMAT)?;
    tx.commit()?;
    profile.pending_record = pending;
    profile.credential_record = credential;
    Ok(())
}

fn snapshot(directory: &Path, version: u32) -> anyhow::Result<()> {
    let path = directory.join(PROFILE);
    profile_file(&path, version)?;
    let bytes = fs::read(path)?;
    let target = directory.join(format!(
        "workspace-profile-v{version}-{}.sqlite3",
        crypto::hash(&bytes)
    ));
    if target.try_exists()? {
        profile_file(&target, version)?;
        if fs::read(target)? != bytes {
            bail!("profile upgrade snapshot does not match");
        }
        return sync_directory(directory);
    }
    let temporary = directory.join(format!(".profile-upgrade-{}.sqlite3", Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    let result = fs::hard_link(&temporary, &target)
        .context("cannot publish private profile upgrade snapshot");
    fs::remove_file(&temporary)?;
    result?;
    sync_directory(directory)
}

fn profile_file(path: &Path, version: u32) -> anyhow::Result<()> {
    if version >= 4 {
        private_file_limit(path, 16 * 1024 * 1024)
    } else {
        private_file(path)
    }
}
