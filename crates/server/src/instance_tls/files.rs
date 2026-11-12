// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use uuid::Uuid;

pub(super) const FILE_NAME: &str = "ccp-instance-tls.json";
const MAX_BYTES: u64 = 64 * 1024;

pub(super) fn exists(directory: &Path) -> anyhow::Result<bool> {
    exists_path(&directory.join(FILE_NAME))
}

fn exists_path(path: &Path) -> anyhow::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn read(directory: &Path) -> anyhow::Result<Vec<u8>> {
    read_path(&directory.join(FILE_NAME))
}

pub(super) fn read_pinned(directory: &Path, hash: &[u8]) -> anyhow::Result<Vec<u8>> {
    let generation = generation_path(directory, hash)?;
    if exists_path(&generation)? {
        return read_path(&generation);
    }
    let bytes = read(directory)?;
    if Sha256::digest(&bytes).as_slice() != hash {
        bail!("selected TLS generation is missing; restore the matching backup");
    }
    Ok(bytes)
}

pub(super) fn generation_path(directory: &Path, hash: &[u8]) -> anyhow::Result<PathBuf> {
    if hash.len() != 32 {
        bail!("invalid TLS generation pin");
    }
    let hex: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(directory.join(format!("ccp-instance-tls-{hex}.json")))
}

fn read_path(path: &Path) -> anyhow::Result<Vec<u8>> {
    let metadata =
        fs::symlink_metadata(path).context("TLS bundle is missing; restore matching backup")?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        bail!("TLS bundle must be a bounded regular file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            bail!("TLS bundle permissions must exclude group and other users");
        }
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len())? > MAX_BYTES {
        bail!("TLS bundle is too large");
    }
    Ok(bytes)
}

pub(super) fn publish(directory: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    publish_at(directory, &directory.join(FILE_NAME), bytes)
}

pub(super) fn publish_generation(directory: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    publish_at(
        directory,
        &generation_path(directory, &Sha256::digest(bytes))?,
        bytes,
    )
}

fn publish_at(directory: &Path, destination: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if u64::try_from(bytes.len())? > MAX_BYTES {
        bail!("TLS bundle is too large");
    }
    let temporary = directory.join(format!(".ccp-instance-tls-{}.tmp", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    let operation = (|| -> anyhow::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        // Unlike rename, hard_link cannot replace an existing destination. The
        // linked inode already contains the complete, synchronized bundle.
        fs::hard_link(&temporary, destination)
            .context("cannot publish TLS bundle without replacing existing material")?;
        sync_directory(directory)?;
        Ok(())
    })();
    drop(file);
    fs::remove_file(&temporary).context("failed to remove private TLS staging file")?;
    sync_directory(directory)?;
    operation
}

fn sync_directory(directory: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    File::open(directory)?.sync_all()?;
    #[cfg(not(unix))]
    {
        let _directory = directory;
    }
    Ok(())
}
