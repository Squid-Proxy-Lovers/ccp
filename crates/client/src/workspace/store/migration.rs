// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use std::io::Write;

pub(super) fn upgrade(profile: &mut WorkspaceProfile, directory: &Path) -> anyhow::Result<()> {
    snapshot(directory)?;
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
    tx.execute_batch(HISTORY_SCHEMA)?;
    let changed=tx.execute("UPDATE profile SET pending=?1,credential=?2 WHERE singleton=1 AND pending IS ?3 AND credential IS ?4",
        params![pending,credential,profile.pending_record,profile.credential_record])?;
    if changed != 1 {
        bail!("profile changed before format upgrade");
    }
    tx.pragma_update(None, "user_version", 2)?;
    tx.commit()?;
    profile.pending_record = pending;
    profile.credential_record = credential;
    Ok(())
}

fn snapshot(directory: &Path) -> anyhow::Result<()> {
    let path = directory.join(PROFILE);
    private_file(&path)?;
    let bytes = fs::read(path)?;
    let target = directory.join(format!(
        "workspace-profile-v1-{}.sqlite3",
        crypto::hash(&bytes)
    ));
    if target.try_exists()? {
        private_file(&target)?;
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
