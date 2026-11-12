// SPDX-License-Identifier: AGPL-3.0-or-later

//! Read-only software compatibility inspection, separate from authenticated work.

use super::{network, store};
use anyhow::bail;
use protocol::workspace as api;
use serde::Serialize;
use std::path::Path;

#[cfg(test)]
mod tests;

#[derive(Serialize)]
pub struct UpdatePreflight {
    pub format: u32,
    pub profile_format: u32,
    pub migration_required: bool,
    pub pending: bool,
    pub server: api::SoftwareInfo,
}

/// Inspect existing state and pinned HTTPS metadata without opening a mutable profile.
///
/// Migration and pending-operation flags require a later updater to defer activation.
/// This is neither an authorization check nor an atomic reservation for an update.
pub async fn update_preflight(directory: &Path) -> anyhow::Result<UpdatePreflight> {
    let (trust, version, pending) = store::inspect_update(directory)?;
    // The read-only SQLite snapshot is closed before contacting the server.
    let http = network::client(&trust, None)?;
    let server: api::SoftwareInfo =
        network::json(http.get(format!("{}api/v1/software-info", trust.https_origin))).await?;
    validate(&server, &trust.instance_id)?;
    Ok(UpdatePreflight {
        format: 1,
        profile_format: version,
        migration_required: version != store::PROFILE_FORMAT,
        pending,
        server,
    })
}

fn validate(server: &api::SoftwareInfo, instance: &str) -> anyhow::Result<()> {
    if server.instance_id != instance
        || server.native_wire != protocol::PROTOCOL_VERSION
        || server.workspace_api != api::API_VERSION
        || server.package_version.is_empty()
        || server.package_version.len() > 64
        || !server.package_version.is_ascii()
        || server.package_version.chars().any(char::is_control)
        || server.package_version.trim() != server.package_version
    {
        bail!("server software compatibility is unavailable or unsupported");
    }
    Ok(())
}
