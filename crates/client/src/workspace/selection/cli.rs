// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args)]
pub(crate) struct SelectionArgs {
    #[arg(long)]
    profile: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Init {
        #[arg(long)]
        selection_dir: PathBuf,
    },
    Show {
        #[arg(long)]
        selection_dir: PathBuf,
    },
    Set {
        #[arg(long)]
        selection_dir: PathBuf,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long = "workspace-id")]
        workspace_ids: Vec<String>,
        #[arg(long)]
        default_workspace_id: Option<String>,
    },
    Resolve {
        #[arg(long)]
        selection_dir: PathBuf,
    },
    Target {
        #[arg(long)]
        selection_dir: Option<PathBuf>,
        #[arg(long)]
        workspace_id: Option<String>,
    },
}

pub(crate) async fn run(args: SelectionArgs) -> anyhow::Result<()> {
    let mut profile = WorkspaceProfile::open(&args.profile)?;
    match args.command {
        Command::Init { selection_dir } => output(&profile.initialize_selection(&selection_dir)?),
        Command::Show { selection_dir } => output(&profile.selection(&selection_dir)?),
        Command::Set {
            selection_dir,
            expected_revision,
            workspace_ids,
            default_workspace_id,
        } => output(
            &profile
                .set_selection(
                    &selection_dir,
                    SelectionUpdate {
                        workspace_ids,
                        default_workspace_id,
                        expected_revision,
                    },
                )
                .await?,
        ),
        Command::Resolve { selection_dir } => {
            output(&profile.selected_workspaces(&selection_dir).await?)
        }
        Command::Target {
            selection_dir,
            workspace_id,
        } => output(
            &profile
                .target(selection_dir.as_deref(), workspace_id.as_deref())
                .await?,
        ),
    }
}

fn output(value: &impl Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}
