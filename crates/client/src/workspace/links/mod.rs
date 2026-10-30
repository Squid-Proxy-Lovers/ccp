// SPDX-License-Identifier: AGPL-3.0-or-later
//! Link enrollment composes the existing pinned trust and durable profile APIs.
pub(crate) mod cli;
mod discovery;
mod profiles;
use super::{Operation, WorkspaceProfile, crypto, network, store};
use anyhow::{Context, bail};
use protocol::{enrollment_link::EnrollmentLink, workspace as api};
use serde::Serialize;
use std::path::{Path, PathBuf};
#[derive(Serialize)]
pub struct LinkEnrollment {
    pub profile: PathBuf,
    pub workspace: api::Workspace,
    pub client_id: String,
    pub reused_registration: bool,
    pub name_changed: bool,
}
pub async fn enroll_link(
    link: &EnrollmentLink,
    explicit: Option<&Path>,
    profile_root: &Path,
) -> anyhow::Result<LinkEnrollment> {
    let trust = discovery::trust(link.target()).await?;
    let http = network::client(&trust, None)?;
    network::health(&http, &trust.https_origin, &trust.instance_id).await?;
    // All identity checks precede profile discovery/publication and CSR intent.
    let directory = profiles::choose(link, explicit, profile_root)?;
    if directory.join("workspace-profile.sqlite3").try_exists()? {
        let (saved, _, _) = store::inspect_update(&directory).context(
            "existing profile needs local inspection before link enrollment; preserve it",
        )?;
        profiles::same_instance(&saved, link.target())?;
    } else {
        profiles::require_empty(&directory)?;
        WorkspaceProfile::trust(&directory, trust).await?;
    }
    let mut profile = WorkspaceProfile::open(&directory)?;
    profiles::same_instance(&profile.trust, link.target())?;
    let reused = profile.credential.is_some();
    let invitation_hash = crypto::hash(link.invitation().as_bytes());
    if !reused
        && profile.pending.as_ref().is_some_and(|pending| {
            pending.operation == Operation::Enrollment
                && pending.invitation_hash.as_ref() != Some(&invitation_hash)
        })
    {
        bail!(
            "profile {:?} has an unfinished enrollment for a different invitation. Retry the original link. If an administrator confirms it was never accepted, move this directory aside before using the new link. An explicit --profile keeps both profiles and requires explicit profile selection thereafter",
            directory
        );
    }
    let completed: bool = profile.connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM completed_invitations WHERE invitation_hash=?1)",
        [invitation_hash],
        |row| row.get(0),
    )?;
    if !completed {
        let admission = if reused {
            profile.join(link.invitation()).await
        } else {
            profile.enroll(link.invitation()).await
        };
        admission.with_context(|| format!("enrollment state is saved at {:?}; preserve the profile and retry the original link if the outcome is uncertain", directory))?;
    } else if !reused {
        bail!("completed invitation has no usable registration; preserve the profile for recovery");
    }
    // Explicit target resolution rechecks current access and never writes a
    // caller's selection or adds a grant from an unverified name hint.
    let target=profile.target(None,Some(&link.target().workspace_id.to_string())).await
        .context("registration state is saved, but target access could not be verified; preserve the profile")?;
    Ok(LinkEnrollment {
        profile: directory,
        client_id: target.client_id,
        name_changed: target.workspace.name != link.target().workspace_name,
        workspace: target.workspace,
        reused_registration: reused,
    })
}
