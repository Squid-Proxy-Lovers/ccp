// SPDX-License-Identifier: AGPL-3.0-or-later
use super::{InspectionError as Error, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub(super) const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Fingerprint {
    pub bytes: u64,
    pub sha256: String,
}

pub(super) struct Snapshot {
    source: PathBuf,
    database: PathBuf,
    pub fingerprint: Fingerprint,
    _scratch: Scratch,
}
impl Snapshot {
    pub(super) fn capture(directory: &Path) -> Result<Self> {
        if !directory.is_absolute() {
            return Err(Error::SourceUnavailable);
        }
        let info = fs::symlink_metadata(directory).map_err(|_error| Error::SourceUnavailable)?;
        if !info.is_dir() || info.file_type().is_symlink() {
            return Err(Error::SourceUnavailable);
        }
        let source = fs::canonicalize(directory).map_err(|_error| Error::SourceUnavailable)?;
        clean(&source)?;
        let mut input = source_file(&source.join("ccp.sqlite3"))?;
        let mut header = [0_u8; 16];
        input
            .read_exact(&mut header)
            .map_err(|_error| Error::InvalidDatabase)?;
        if &header != b"SQLite format 3\0" {
            return Err(Error::InvalidDatabase);
        }
        input
            .seek(SeekFrom::Start(0))
            .map_err(|_error| Error::SourceUnavailable)?;
        let scratch = Scratch::create()?;
        let database = scratch.0.join("snapshot.sqlite3");
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut output = options
            .open(&database)
            .map_err(|_error| Error::ScratchUnavailable)?;
        let fingerprint = stream(&mut input, Some(&mut output))?;
        drop(output);
        clean(&source)?;
        Ok(Self {
            source,
            database,
            fingerprint,
            _scratch: scratch,
        })
    }
    pub(super) fn database(&self) -> &Path {
        &self.database
    }
    pub(super) fn verify(&self) -> Result<()> {
        clean(&self.source)?;
        let mut file = source_file(&self.source.join("ccp.sqlite3"))?;
        if stream(&mut file, None)? != self.fingerprint {
            return Err(Error::SourceChanged);
        }
        clean(&self.source)
    }
}

fn source_file(path: &Path) -> Result<File> {
    let metadata = fs::symlink_metadata(path).map_err(|_error| Error::SourceUnavailable)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(Error::SourceUnavailable);
    }
    if metadata.len() > MAX_BYTES {
        return Err(Error::TooLarge);
    }
    File::open(path).map_err(|_error| Error::SourceUnavailable)
}

fn clean(directory: &Path) -> Result<()> {
    for name in ["ccp.sqlite3-wal", "ccp.sqlite3-shm", "ccp.sqlite3-journal"] {
        match fs::symlink_metadata(directory.join(name)) {
            Ok(_) => return Err(Error::SidecarsPresent),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error::SourceUnavailable),
        }
    }
    match fs::symlink_metadata(directory.join("runtime-journal.jsonl")) {
        Ok(m) if m.is_file() && !m.file_type().is_symlink() && m.len() == 0 => Ok(()),
        Ok(_) => Err(Error::PendingJournal),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(Error::SourceUnavailable),
    }
}

fn stream(input: &mut File, mut output: Option<&mut File>) -> Result<Fingerprint> {
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|_error| Error::SourceUnavailable)?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(u64::try_from(count).map_err(|_error| Error::TooLarge)?)
            .ok_or(Error::TooLarge)?;
        if bytes > MAX_BYTES {
            return Err(Error::TooLarge);
        }
        let chunk = buffer.get(..count).ok_or(Error::SourceUnavailable)?;
        if let Some(file) = output.as_mut() {
            file.write_all(chunk)
                .map_err(|_error| Error::ScratchUnavailable)?;
        }
        hash.update(chunk);
    }
    Ok(Fingerprint {
        bytes,
        sha256: format!("{:x}", hash.finalize()),
    })
}

struct Scratch(PathBuf);
impl Scratch {
    fn create() -> Result<Self> {
        let path =
            std::env::temp_dir().join(format!("ccp-native-inspection-{}", uuid::Uuid::new_v4()));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        builder.mode(0o700);
        builder
            .create(&path)
            .map_err(|_error| Error::ScratchUnavailable)?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        if fs::remove_dir_all(&self.0).is_err() {
            eprintln!("native inventory: private scratch cleanup failed");
        }
    }
}
