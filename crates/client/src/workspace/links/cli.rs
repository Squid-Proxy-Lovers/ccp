// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::bail;
use protocol::enrollment_link::{EnrollmentLink, MAX_LINK_BYTES};
use std::{
    io::Read,
    path::{Path, PathBuf},
};
pub(crate) async fn run(value: Option<&str>, profile: Option<&Path>) -> anyhow::Result<()> {
    let value = input(value)?;
    let link = EnrollmentLink::parse(&value)?;
    let root = if profile.is_none() {
        crate::storage::client_home_dir()?.join("workspaces")
    } else {
        PathBuf::new()
    };
    let result = super::enroll_link(&link, profile, &root).await?;
    println!("Workspace: {:?}", result.workspace.name);
    println!("Client: {}", result.client_id);
    println!("Profile: {}", result.profile.display());
    if profile.is_some() {
        println!("Use the same --profile path for future links to reuse this registration.");
    }
    if result.reused_registration {
        println!(
            "Reused this instance's existing registration; workspace selections are unchanged."
        );
    }
    if result.name_changed {
        println!(
            "The link's display-name hint changed; the workspace name above was verified from the server."
        );
    }
    println!(
        "Next: add this profile to a private CCP_WORKSPACE_CONFIG and configure ccp-workspace-mcp in your MCP host."
    );
    println!(
        "Setup guide: docs/workspace-mcp.md in the matching CCP source or release documentation."
    );
    Ok(())
}
fn input(value: Option<&str>) -> anyhow::Result<String> {
    let value = match value {
        Some("-") => {
            let mut bytes = Vec::new();
            std::io::stdin()
                .take(u64::try_from(MAX_LINK_BYTES + 3)?)
                .read_to_end(&mut bytes)?;
            String::from_utf8(bytes)
                .map_err(|_utf8| anyhow::anyhow!("enrollment link is not valid UTF-8"))?
        }
        Some(value) => {
            eprintln!(
                "A link on argv can remain in shell history and process listings. Use enroll without arguments for hidden paste."
            );
            value.to_owned()
        }
        None => {
            rpassword::prompt_password("Paste enrollment link (hidden): ").map_err(|_terminal| {
                anyhow::anyhow!("hidden paste needs a terminal; use enroll - for bounded stdin")
            })?
        }
    };
    let value = value
        .strip_suffix("\r\n")
        .or_else(|| value.strip_suffix('\n'))
        .unwrap_or(&value);
    if value.len() > MAX_LINK_BYTES {
        bail!("enrollment link exceeds size limit");
    }
    if value.is_empty() {
        bail!("an enrollment link is required");
    }
    Ok(value.to_owned())
}
