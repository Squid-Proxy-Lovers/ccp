// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args)]
pub(crate) struct StatusArgs {
    #[arg(long)]
    profile: PathBuf,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Get {
        #[arg(long)]
        workspace_id: String,
        #[arg(long)]
        status_id: String,
    },
    List {
        #[arg(long)]
        workspace_id: String,
        #[arg(long)]
        after: Option<String>,
        #[arg(long, default_value_t = 32)]
        limit: u16,
    },
    Mutate {
        #[arg(long)]
        request_file: PathBuf,
    },
    Pending,
    Resume {
        #[command(flatten)]
        target: Expected,
    },
    /// Forget locally only. The original server request is not cancelled.
    Forget {
        #[command(flatten)]
        target: Expected,
        #[arg(long, required = true)]
        confirm_local_forget: bool,
    },
}
#[derive(Args)]
struct Expected {
    #[arg(long)]
    workspace_id: String,
    #[arg(long)]
    status_id: String,
    #[arg(long)]
    operation_id: String,
    #[arg(long)]
    credential_id: String,
    #[arg(long)]
    request_hash: String,
}
impl Expected {
    fn value(self) -> WorkStatusTarget {
        WorkStatusTarget {
            workspace_id: self.workspace_id,
            status_id: self.status_id,
            operation_id: self.operation_id,
            credential_id: self.credential_id,
            request_hash: self.request_hash,
        }
    }
}
pub(crate) async fn run(args: StatusArgs) -> anyhow::Result<()> {
    let mut profile = WorkspaceProfile::open(&args.profile)?;
    match args.command {
        Command::Get {
            workspace_id,
            status_id,
        } => output(
            &profile
                .status_message(&wire::Get {
                    workspace_id,
                    status_id,
                })
                .await?,
        ),
        Command::List {
            workspace_id,
            after,
            limit,
        } => output(
            &profile
                .status_messages(&wire::List {
                    workspace_id,
                    after,
                    limit: Some(limit),
                })
                .await?,
        ),
        Command::Mutate { request_file } => {
            let text =
                crate::workspace::cli::read(&request_file, u64::try_from(MAX_REQUEST)?, true)?;
            let request = serde_json::from_str(&text)
                .map_err(|_error| anyhow::anyhow!("invalid status mutation JSON"))?;
            let value = profile.mutate_status(request).await?;
            mutation_output(&profile, &value)
        }
        Command::Pending => output(
            &serde_json::json!({"pending":profile.status_state.pending_info(),"last":profile.status_state.last_info()}),
        ),
        Command::Resume { target } => {
            let value = profile.resume_status_operation(&target.value()).await?;
            mutation_output(&profile, &value)
        }
        Command::Forget {
            target,
            confirm_local_forget,
        } => {
            profile.forget_status_operation(&target.value(), confirm_local_forget)?;
            output(
                &serde_json::json!({"locally_forgotten":true,"cancelled":false,"operation":profile.status_state.last_info()}),
            )
        }
    }
}
#[derive(Serialize)]
struct MutationOutput<'a> {
    #[serde(flatten)]
    receipt: &'a wire::Receipt,
    operation: Option<WorkStatusOperationInfo>,
}
fn mutation_output(profile: &WorkspaceProfile, receipt: &wire::Receipt) -> anyhow::Result<()> {
    output(&MutationOutput {
        receipt,
        operation: profile.status_state.last_info(),
    })
}
fn output(value: &impl Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}
