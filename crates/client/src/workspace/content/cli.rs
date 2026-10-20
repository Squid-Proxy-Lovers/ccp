// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args)]
pub(crate) struct ContentArgs {
    #[arg(long)]
    profile: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct Scope {
    #[arg(long)]
    workspace_id: String,
}

#[derive(Args)]
struct Paging {
    #[arg(long)]
    after: Option<String>,
    #[arg(long, default_value_t = 32)]
    limit: u16,
}

#[derive(Subcommand)]
enum Command {
    #[command(flatten)]
    Mutation(super::mutation::cli::Command),
    Shelves {
        #[command(flatten)]
        scope: Scope,
        #[command(flatten)]
        page: Paging,
    },
    Books {
        #[command(flatten)]
        scope: Scope,
        #[arg(long)]
        shelf_id: String,
        #[command(flatten)]
        page: Paging,
    },
    Shelf {
        #[command(flatten)]
        scope: Scope,
        #[arg(long)]
        node_id: String,
    },
    Book {
        #[command(flatten)]
        scope: Scope,
        #[arg(long)]
        node_id: String,
    },
    Entry {
        #[command(flatten)]
        scope: Scope,
        #[arg(long)]
        entry_id: String,
        #[arg(long)]
        revision: Option<u64>,
    },
    Entries {
        #[command(flatten)]
        scope: Scope,
        #[arg(long)]
        book_id: String,
        #[arg(long)]
        archived: bool,
        #[command(flatten)]
        page: Paging,
    },
    History {
        #[command(flatten)]
        scope: Scope,
        #[arg(long)]
        entry_id: String,
        #[arg(long)]
        after: Option<u64>,
        #[arg(long, default_value_t = 32)]
        limit: u16,
    },
}

pub(crate) async fn run(args: ContentArgs) -> anyhow::Result<()> {
    match args.command {
        Command::Mutation(command) => super::mutation::cli::run(&args.profile, command).await,
        command => run_read(&args.profile, command).await,
    }
}

async fn run_read(path: &std::path::Path, command: Command) -> anyhow::Result<()> {
    let mut profile = WorkspaceProfile::open(path)?;
    match command {
        Command::Mutation(_) => bail!("entry mutation command requires the mutation dispatcher"),
        Command::Shelves { scope, page } => output(
            &profile
                .content_shelves(&ShelfPage {
                    workspace_id: scope.workspace_id,
                    after: page.after,
                    limit: Some(page.limit),
                })
                .await?,
        ),
        Command::Books {
            scope,
            shelf_id,
            page,
        } => output(
            &profile
                .content_books(&BookPage {
                    workspace_id: scope.workspace_id,
                    shelf_id,
                    after: page.after,
                    limit: Some(page.limit),
                })
                .await?,
        ),
        Command::Shelf { scope, node_id } => output(
            &profile
                .content_shelf(&NodeTarget {
                    workspace_id: scope.workspace_id,
                    node_id,
                })
                .await?,
        ),
        Command::Book { scope, node_id } => output(
            &profile
                .content_book(&NodeTarget {
                    workspace_id: scope.workspace_id,
                    node_id,
                })
                .await?,
        ),
        Command::Entry {
            scope,
            entry_id,
            revision,
        } => output(
            &profile
                .content_entry(&EntryTarget {
                    workspace_id: scope.workspace_id,
                    entry_id,
                    revision,
                })
                .await?,
        ),
        Command::Entries {
            scope,
            book_id,
            archived,
            page,
        } => output(
            &profile
                .content_entries(&EntryPage {
                    workspace_id: scope.workspace_id,
                    book_id,
                    archived,
                    after: page.after,
                    limit: Some(page.limit),
                })
                .await?,
        ),
        Command::History {
            scope,
            entry_id,
            after,
            limit,
        } => output(
            &profile
                .content_history(&HistoryPage {
                    workspace_id: scope.workspace_id,
                    entry_id,
                    after,
                    limit: Some(limit),
                })
                .await?,
        ),
    }
}

fn output(value: &impl Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}
