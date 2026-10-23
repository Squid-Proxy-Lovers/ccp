// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Args)]
pub(crate) struct Request {
    #[arg(long)]
    request_file: PathBuf,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Create one shelf; inspect its workspace listing after an uncertain result.
    CreateShelf(Request),
    /// Create one book; inspect its shelf listing after an uncertain result.
    CreateBook(Request),
}

pub(crate) async fn run(path: &Path, command: Command) -> anyhow::Result<()> {
    let mut profile = WorkspaceProfile::open(path)?;
    match command {
        Command::CreateShelf(request) => {
            output(&profile.content_create_shelf(&read(&request)?).await?)
        }
        Command::CreateBook(request) => {
            output(&profile.content_create_book(&read(&request)?).await?)
        }
    }
}

fn read<T: DeserializeOwned>(request: &Request) -> anyhow::Result<T> {
    let value =
        crate::workspace::cli::read(&request.request_file, u64::try_from(MAX_REQUEST)?, true)?;
    serde_json::from_str(&value)
        .map_err(|_json| anyhow::anyhow!("invalid shelf or book creation JSON"))
}

fn output(value: &impl Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}
