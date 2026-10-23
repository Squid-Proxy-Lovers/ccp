// SPDX-License-Identifier: AGPL-3.0-or-later
use super::{ConversionError, Result};
use anyhow::{Context, bail};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::{
    fs::{self, File, OpenOptions},
    path::{Component, Path, PathBuf},
};

pub(super) struct Stage {
    directory: PathBuf,
    destination: PathBuf,
    parent: PathBuf,
}
impl Stage {
    pub fn create(source: &Path, destination: &Path) -> anyhow::Result<Self> {
        if !destination.is_absolute()
            || destination
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            bail!("absolute destination required");
        }
        let name = destination
            .file_name()
            .context("destination name required")?;
        let parent = fs::canonicalize(
            destination
                .parent()
                .context("destination parent required")?,
        )?;
        let destination = parent.join(name);
        let source = fs::canonicalize(source)?;
        if destination.starts_with(&source) || source.starts_with(&destination) {
            bail!("source and destination overlap");
        }
        match fs::symlink_metadata(&destination) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => bail!("destination already exists or unavailable"),
        }
        let directory = parent.join(format!(".ccp-native-conversion-{}", uuid::Uuid::new_v4()));
        private_directory(&directory)?;
        let stage = Self {
            directory,
            destination,
            parent,
        };
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        options.open(stage.database())?;
        Ok(stage)
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn database(&self) -> PathBuf {
        self.directory.join("ccp.sqlite3")
    }
    pub fn publish(&self) -> Result<()> {
        self.publish_with_sync(sync_directory)
    }
    pub(super) fn publish_with_sync(
        &self,
        sync: impl Fn(&Path) -> std::io::Result<()>,
    ) -> Result<()> {
        let database = self.database();
        if fs::metadata(&database)
            .map_err(|_e| ConversionError::DestinationUnavailable)?
            .len()
            > 4 * 1024 * 1024 * 1024
        {
            return Err(ConversionError::DestinationUnavailable);
        }
        File::open(&database)
            .and_then(|f| f.sync_all())
            .map_err(|_e| ConversionError::DestinationUnavailable)?;
        private_directory(&self.destination)
            .map_err(|_e| ConversionError::DestinationUnavailable)?;
        if fs::hard_link(&database, self.destination.join("ccp.sqlite3")).is_err() {
            // Remove only an empty reservation, never another writer's files.
            if let Err(error) = fs::remove_dir(&self.destination)
                && error.kind() != std::io::ErrorKind::DirectoryNotEmpty
            {
                eprintln!("native conversion: empty destination reservation could not be removed");
            }
            return Err(ConversionError::DestinationUnavailable);
        }
        // From this point the complete database is visible. A failed sync must
        // not delete it or pretend the operation definitely did not commit.
        sync(&self.destination)
            .and_then(|()| sync(&self.parent))
            .map_err(|_e| ConversionError::PublicationUnconfirmed)
    }
}
fn private_directory(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(path)
}
fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    {
        let _path = path;
    }
    Ok(())
}
impl Drop for Stage {
    fn drop(&mut self) {
        if fs::remove_dir_all(&self.directory).is_err() {
            eprintln!("native conversion: private staging cleanup failed");
        }
    }
}
