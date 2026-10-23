// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use clap::Subcommand;
use std::path::Path;

#[derive(Subcommand)]
pub(crate) enum Command {
    ShelfState {
        #[arg(long)]
        workspace_id: String,
        #[arg(long)]
        node_id: String,
    },
    ArchivedShelves {
        #[arg(long)]
        workspace_id: String,
        #[arg(long)]
        after: Option<String>,
        #[arg(long, default_value_t = 32)]
        limit: u16,
    },
}

pub(crate) async fn run(path: &Path, command: Command) -> anyhow::Result<()> {
    let mut profile = WorkspaceProfile::open(path)?;
    match command {
        Command::ShelfState {
            workspace_id,
            node_id,
        } => {
            let value = profile
                .content_shelf_state(&NodeTarget {
                    workspace_id,
                    node_id,
                })
                .await?;
            println!("{}", serde_json::to_string(&value)?);
        }
        Command::ArchivedShelves {
            workspace_id,
            after,
            limit,
        } => {
            let value = profile
                .content_archived_shelves(&ShelfPage {
                    workspace_id,
                    after,
                    limit: Some(limit),
                })
                .await?;
            println!("{}", serde_json::to_string(&value)?);
        }
    }
    Ok(())
}
