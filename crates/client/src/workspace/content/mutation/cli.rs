// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub(crate) enum Command {
    Mutate {
        #[arg(long)]
        request_file: PathBuf,
    },
    Resume {
        #[command(flatten)]
        expected: ExpectedOperation,
    },
    Resolve {
        #[arg(long)]
        via_profile: Option<PathBuf>,
        #[command(flatten)]
        expected: ExpectedOperation,
    },
    Pending,
}

#[derive(Args)]
pub(crate) struct ExpectedOperation {
    #[arg(long, requires_all = ["operation_id", "credential_id", "request_hash"])]
    workspace_id: Option<String>,
    #[arg(long, requires = "workspace_id")]
    operation_id: Option<String>,
    #[arg(long, requires = "workspace_id")]
    credential_id: Option<String>,
    #[arg(long, requires = "workspace_id")]
    request_hash: Option<String>,
}

impl ExpectedOperation {
    fn check(&self, profile: &WorkspaceProfile) -> anyhow::Result<()> {
        let fields = [
            &self.workspace_id,
            &self.operation_id,
            &self.credential_id,
            &self.request_hash,
        ];
        if fields.iter().all(|value| value.is_none()) {
            return Ok(());
        }
        let complete = "expected operation guard requires all four binding fields";
        profile.require_expected_operation(&wire::ResolveEntryOperation {
            workspace_id: self.workspace_id.as_ref().context(complete)?.clone(),
            operation_id: self.operation_id.as_ref().context(complete)?.clone(),
            credential_id: self.credential_id.as_ref().context(complete)?.clone(),
            request_hash: self.request_hash.as_ref().context(complete)?.clone(),
        })
    }
}

pub(crate) async fn run(path: &Path, command: Command) -> anyhow::Result<()> {
    let mut profile = WorkspaceProfile::open(path)?;
    match command {
        Command::Mutate { request_file } => {
            let value =
                crate::workspace::cli::read(&request_file, u64::try_from(MAX_REQUEST)?, true)?;
            let request = serde_json::from_str(&value)
                .map_err(|_request| anyhow::anyhow!("invalid entry mutation JSON"))?;
            let receipt = profile.mutate_entry(request).await?;
            mutation_output(&profile, &receipt)
        }
        Command::Resume { expected } => {
            expected.check(&profile)?;
            let receipt = profile.resume_entry_operation().await?;
            mutation_output(&profile, &receipt)
        }
        Command::Resolve {
            via_profile,
            expected,
        } => {
            expected.check(&profile)?;
            let value = match via_profile {
                Some(other) => {
                    profile
                        .resolve_entry_operation_via(&mut WorkspaceProfile::open(&other)?)
                        .await?
                }
                None => profile.resolve_entry_operation().await?,
            };
            output(&value)
        }
        Command::Pending => output(&profile.pending_entry_operation()),
    }
}

#[derive(Serialize)]
struct MutationOutput<'a> {
    #[serde(flatten)]
    receipt: &'a wire::EntryReceipt,
    operation: Option<ContentOperationInfo>,
}

fn mutation_output(profile: &WorkspaceProfile, receipt: &wire::EntryReceipt) -> anyhow::Result<()> {
    output(&MutationOutput {
        receipt,
        operation: profile.info()?.last_content,
    })
}

fn output(value: &impl Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}
