// SPDX-License-Identifier: AGPL-3.0-or-later
use super::super::{WorkspaceTrust, crypto, store};
use anyhow::{Context, bail};
use protocol::enrollment_link::{EnrollmentLink, LinkTarget};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn absolute(path: &Path) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}
pub(super) fn same_instance(trust: &WorkspaceTrust, target: &LinkTarget) -> anyhow::Result<()> {
    let hash = crypto::hash(&crypto::ca_der(&trust.ca_pem)?);
    let expected: String = target
        .ca_sha256
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if trust.instance_id != target.instance_id.to_string() || hash != expected {
        bail!("existing profile belongs to a different pinned instance; it was not replaced");
    }
    Ok(())
}
pub(super) fn choose(
    link: &EnrollmentLink,
    explicit: Option<&Path>,
    root: &Path,
) -> anyhow::Result<PathBuf> {
    if let Some(path) = explicit {
        return absolute(path);
    }
    let root = absolute(root)?;
    let mut matching = Vec::new();
    let candidate = root.join(slug(&link.target().workspace_name));
    let mut other_instance_at_candidate = false;
    match fs::symlink_metadata(&root) {
        Ok(_) => {
            store::private_directory(&root, false)?;
            for (index, entry) in fs::read_dir(&root)?.enumerate() {
                if index >= 1024 {
                    bail!("too many profile entries; choose an explicit --profile");
                }
                let entry = entry?;
                let kind = entry.file_type()?;
                if kind.is_symlink() {
                    bail!("profile discovery found a symlink; choose an explicit --profile");
                }
                if !kind.is_dir()
                    || !entry
                        .path()
                        .join("workspace-profile.sqlite3")
                        .try_exists()?
                {
                    continue;
                }
                let (trust, _, _) = store::inspect_update(&entry.path()).context(
                    "profile discovery cannot inspect existing state; choose an explicit --profile",
                )?;
                if entry.path() == candidate
                    && trust.instance_id != link.target().instance_id.to_string()
                {
                    other_instance_at_candidate = true;
                }
                if trust.instance_id == link.target().instance_id.to_string() {
                    same_instance(&trust, link.target())?;
                    matching.push(entry.path());
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    match matching.len() {
        0 if other_instance_at_candidate => Ok(root.join(format!(
            "{}-{:08x}",
            slug(&link.target().workspace_name),
            link.target().instance_id.as_fields().0
        ))),
        0 => Ok(candidate),
        1 => matching.pop().context("profile selection changed"),
        _ => bail!("multiple profiles belong to this instance; choose an explicit --profile"),
    }
}
pub(super) fn require_empty(directory: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(directory) {
        Ok(_) => {
            store::private_directory(directory, false)?;
            if fs::read_dir(directory)?.next().is_some() {
                bail!("nonempty directory has no supported profile; no files were replaced");
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
fn slug(name: &str) -> String {
    let mut result = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() {
            result.push(char::from(byte.to_ascii_lowercase()));
        } else if !result.is_empty() && !result.ends_with('-') {
            result.push('-');
        }
        if result.len() >= 64 {
            break;
        }
    }
    let result = result.trim_end_matches('-');
    if result.is_empty() || matches!(result, "." | "..") {
        "workspace".into()
    } else {
        result.into()
    }
}
