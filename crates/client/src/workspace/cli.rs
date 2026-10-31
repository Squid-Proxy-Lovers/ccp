// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{WorkspaceProfile, WorkspaceTrust, selection, store};
use anyhow::bail;
use clap::{Args, Subcommand};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Args)]
pub(crate) struct WorkspaceArgs {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Read software compatibility metadata without opening any profile.
    SoftwareInfo,
    /// Inspect profile/server compatibility without migration, renewal or writes.
    UpdatePreflight {
        #[arg(long)]
        profile: PathBuf,
    },
    /// Read current global guidance and an optional explicit workspace board.
    Guidance {
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        workspace_id: Option<String>,
        /// Include authenticated context for guidance delivery bookkeeping.
        #[arg(long)]
        delivery_context: bool,
    },
    Status {
        #[command(flatten)]
        args: super::work_status::cli::StatusArgs,
    },
    Content {
        #[command(flatten)]
        args: super::content::cli::ContentArgs,
    },
    Selection {
        #[command(flatten)]
        args: selection::cli::SelectionArgs,
    },
    Trust {
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        instance_id: String,
        #[arg(long)]
        https_origin: String,
        #[arg(long)]
        mtls_origin: String,
        #[arg(long)]
        ca_file: PathBuf,
    },
    Info {
        #[arg(long)]
        profile: PathBuf,
    },
    Enroll {
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        invitation_file: PathBuf,
    },
    Join {
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        invitation_file: PathBuf,
    },
    Renew {
        #[arg(long)]
        profile: PathBuf,
    },
    Recover {
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        expected_client_id: String,
        #[arg(long)]
        invitation_file: PathBuf,
    },
    Workspaces {
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        after: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: u32,
    },
}
pub(crate) async fn run(args: WorkspaceArgs) -> anyhow::Result<()> {
    match args.command {
        Command::SoftwareInfo => output(&serde_json::json!({
            "format":1,"package_version":env!("CARGO_PKG_VERSION"),
            "native_wire":protocol::PROTOCOL_VERSION,
            "workspace_api":protocol::workspace::API_VERSION,
            "client_profile_read_min":store::PROFILE_MIN_FORMAT,
            "client_profile_write":store::PROFILE_FORMAT,
        })),
        Command::UpdatePreflight { profile } => output(&super::update_preflight(&profile).await?),
        Command::Guidance {
            profile,
            workspace_id,
            delivery_context,
        } => {
            let mut profile = WorkspaceProfile::open(&profile)?;
            if delivery_context {
                output(&profile.guidance_delivery(workspace_id.as_deref()).await?)
            } else {
                output(&profile.guidance(workspace_id.as_deref()).await?)
            }
        }
        Command::Status { args } => super::work_status::cli::run(args).await,
        Command::Content { args } => super::content::cli::run(args).await,
        Command::Selection { args } => selection::cli::run(args).await,
        Command::Trust {
            profile,
            instance_id,
            https_origin,
            mtls_origin,
            ca_file,
        } => {
            let ca = read(&ca_file, 16 * 1024, false)?;
            let trust = WorkspaceTrust::new(&instance_id, &https_origin, &mtls_origin, &ca)?;
            output(&WorkspaceProfile::trust(&profile, trust).await?)
        }
        Command::Info { profile } => output(&WorkspaceProfile::open(&profile)?.info()?),
        Command::Enroll {
            profile,
            invitation_file,
        } => {
            let invitation = read(&invitation_file, 256, true)?;
            output(
                &WorkspaceProfile::open(&profile)?
                    .enroll(&invitation)
                    .await?,
            )
        }
        Command::Join {
            profile,
            invitation_file,
        } => {
            let invitation = read(&invitation_file, 256, true)?;
            output(&WorkspaceProfile::open(&profile)?.join(&invitation).await?)
        }
        Command::Renew { profile } => output(&WorkspaceProfile::open(&profile)?.renew().await?),
        Command::Recover {
            profile,
            expected_client_id,
            invitation_file,
        } => {
            let invitation = read(&invitation_file, 256, true)?;
            output(
                &WorkspaceProfile::open(&profile)?
                    .recover(&expected_client_id, &invitation)
                    .await?,
            )
        }
        Command::Workspaces {
            profile,
            after,
            limit,
        } => output(
            &WorkspaceProfile::open(&profile)?
                .workspaces(after.as_deref(), limit)
                .await?,
        ),
    }
}
fn output(value: &impl serde::Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}
pub(super) fn read(path: &Path, limit: u64, private: bool) -> anyhow::Result<String> {
    if private {
        store::private_file(path)?;
    }
    if !std::fs::metadata(path)?.is_file() {
        bail!("input must be a regular file");
    }
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        bail!("input must be a regular file");
    }
    let mut value = String::new();
    file.take(limit + 1).read_to_string(&mut value)?;
    if u64::try_from(value.len())? > limit {
        bail!("input exceeds size limit");
    }
    Ok(value)
}
