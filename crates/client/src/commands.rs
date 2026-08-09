// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use protocol::{AppendMetadata, ConflictPolicy, TransferScope, TransferSelector};

use crate::enrollment_structs::StoredEnrollment;
use crate::storage::{
    delete_session_enrollments, load_enrollments, select_enrollment, summarize_sessions,
};
use crate::transport::{
    perform_add_book, perform_add_entry, perform_add_shelf, perform_append, perform_brief_me,
    perform_clear_status, perform_delete, perform_delete_shelf, perform_export, perform_get,
    perform_get_entry_at, perform_import, perform_list_team_status, perform_restore,
    perform_search, perform_search_team_status, perform_set_status,
};

const CLIENT_INPUT_FORMATS: &str = r#"Input formats:
  client subscribe <session>
  client subscribe-all
  client remote-sessions
  client sessions
  client master-instructions <session>
  client delete-session <session>
  client list <session>
  client get <session> <entry-name> [--shelf <name>] [--book <name>]
  client search-entries <session> <query>
  client search-shelves <session> <query>
  client search-books <session> <query>
  client search-context <session> <query>
  client search-deleted <session> <query>
  client add-shelf <session> <shelf-name> <shelf-description>
  client add-book <session> --shelf <name> <book-name> <book-description>
  client add-entry <session> --shelf <name> --book <name> <entry-name> <entry-description> [--labels <a,b>] <entry-data>
  client append <session> <entry-name> [--shelf <name>] [--book <name>] <content>
  client delete <session> <entry-name> [--shelf <name>] [--book <name>]
  client delete-shelf <session> <shelf-name>
  client restore <session> <entry-key>
  client history <session> <entry-name> [--shelf <name>] [--book <name>]
  client export <session> [--output <name.droplet>] [--shelf <name>] [--book <name>] [--entry <name>]... [--no-history]
  client import <session> <file.droplet> [--policy error|overwrite|skip|merge-history]
  client brief-me <session>
  client get-entry-at <session> <entry-name> --at <timestamp> [--shelf <name>] [--book <name>]
  client set-status <session> --team <shelf> --agent <name> <status...>
  client clear-status <session> --team <shelf> --agent <name>
  client team-status <session> --team <shelf>
  client search-team-status <session> --team <shelf> <query...>

<session> can be a session name or session id discovered via `client sessions`."#;

#[derive(Parser)]
#[command(name = "client")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Subscribe(SubscribeArgs),
    SubscribeAll(ServerArgs),
    RemoteSessions(RemoteSessionsArgs),
    Sessions,
    MasterInstructions(SessionSelectorArgs),
    DeleteSession(SessionSelectorArgs),
    List(SessionSelectorArgs),
    Get(EntryArgs),
    SearchEntries(SearchArgs),
    SearchShelves(SearchArgs),
    SearchBooks(SearchArgs),
    SearchContext(SearchArgs),
    SearchDeleted(SearchDeletedArgs),
    AddShelf(AddShelfArgs),
    AddBook(AddBookArgs),
    AddEntry(AddEntryArgs),
    Append(AppendArgs),
    Delete(EntryArgs),
    DeleteShelf(DeleteShelfArgs),
    Restore(RestoreArgs),
    History(EntryArgs),
    Export(ExportArgs),
    Import(ImportArgs),
    BriefMe(SessionSelectorArgs),
    GetEntryAt(GetEntryAtArgs),
    SetStatus(SetStatusArgs),
    ClearStatus(StatusIdentityArgs),
    TeamStatus(TeamStatusArgs),
    SearchTeamStatus(SearchTeamStatusArgs),
}

#[derive(Args)]
struct SubscribeArgs {
    #[arg(long, value_name = "http-url")]
    server: Option<String>,
    #[arg(value_name = "session")]
    session: String,
}

#[derive(Args)]
struct RemoteSessionsArgs {
    #[arg(long, value_name = "http-url")]
    server: Option<String>,
}

#[derive(Args)]
struct ServerArgs {
    #[arg(long, value_name = "http-url")]
    server: Option<String>,
}

#[derive(Args)]
struct SessionSelectorArgs {
    #[arg(value_name = "session")]
    session: String,
}

#[derive(Args)]
struct EntryArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(value_name = "entry-name")]
    entry_name: String,
    #[arg(long, value_name = "name")]
    shelf: Option<String>,
    #[arg(long, value_name = "name")]
    book: Option<String>,
}

#[derive(Args)]
struct SearchArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(required = true, value_name = "query")]
    query: Vec<String>,
}

#[derive(Args)]
struct SearchDeletedArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(value_name = "query")]
    query: Vec<String>,
}

#[derive(Args)]
struct DeleteShelfArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(value_name = "shelf-name")]
    shelf_name: String,
}

#[derive(Args)]
struct AddShelfArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(value_name = "shelf-name")]
    shelf_name: String,
    #[arg(value_name = "shelf-description")]
    shelf_description: Vec<String>,
}

#[derive(Args)]
struct AddBookArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(long, value_name = "name")]
    shelf: String,
    #[arg(value_name = "book-name")]
    book_name: String,
    #[arg(value_name = "book-description")]
    book_description: Vec<String>,
}

#[derive(Args)]
struct AddEntryArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(long, value_name = "name")]
    shelf: String,
    #[arg(long, value_name = "name")]
    book: String,
    #[arg(value_name = "entry-name")]
    entry_name: String,
    #[arg(value_name = "entry-description")]
    entry_description: String,
    #[arg(long, value_name = "a,b")]
    labels: Option<String>,
    #[arg(required = true, value_name = "entry-data")]
    entry_data: Vec<String>,
}

#[derive(Args)]
struct AppendArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(value_name = "entry-name")]
    entry_name: String,
    #[arg(long, value_name = "name")]
    shelf: Option<String>,
    #[arg(long, value_name = "name")]
    book: Option<String>,
    #[arg(required = true, value_name = "content")]
    content: Vec<String>,
}

#[derive(Args)]
struct RestoreArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(value_name = "entry-key")]
    entry_key: String,
}

#[derive(Args)]
struct ExportArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(long, value_name = "path")]
    output: Option<PathBuf>,
    /// Export only entries in this shelf (enables scoped export).
    #[arg(long, value_name = "name")]
    shelf: Option<String>,
    /// Export only entries in this book (requires --shelf).
    #[arg(long, value_name = "name")]
    book: Option<String>,
    /// Export specific named entries (requires --shelf and --book; repeatable).
    #[arg(long, value_name = "name")]
    entry: Vec<String>,
    /// Exclude history from the bundle.
    #[arg(long)]
    no_history: bool,
}

#[derive(Args)]
struct ImportArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(value_name = "bundle-path")]
    bundle_path: PathBuf,
    /// Conflict resolution policy: error (default), overwrite, skip, merge-history.
    #[arg(long, value_name = "policy", default_value = "error")]
    policy: String,
    /// Shorthand for --policy overwrite (deprecated; prefer --policy overwrite).
    #[arg(long)]
    overwrite: bool,
}

#[derive(Args)]
struct GetEntryAtArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(value_name = "entry-name")]
    entry_name: String,
    #[arg(long, value_name = "timestamp")]
    at: String,
    #[arg(long, value_name = "name")]
    shelf: Option<String>,
    #[arg(long, value_name = "name")]
    book: Option<String>,
}

#[derive(Args)]
struct SetStatusArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(long, value_name = "shelf")]
    team: String,
    #[arg(long, value_name = "name")]
    agent: String,
    #[arg(required = true, value_name = "status")]
    status: Vec<String>,
}

#[derive(Args)]
struct StatusIdentityArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(long, value_name = "shelf")]
    team: String,
    #[arg(long, value_name = "name")]
    agent: String,
}

#[derive(Args)]
struct TeamStatusArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(long, value_name = "shelf")]
    team: String,
}

#[derive(Args)]
struct SearchTeamStatusArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(long, value_name = "shelf")]
    team: String,
    #[arg(required = true, value_name = "query")]
    query: Vec<String>,
}

pub(crate) async fn run() -> anyhow::Result<()> {
    let cli = parse_cli();

    match cli.command {
        Command::Subscribe(args) => {
            let server = resolved_server(args.server.as_deref());
            let sessions = crate::transport_helpers::list_remote_sessions(&server).await?;
            let session = sessions
                .iter()
                .find(|candidate| {
                    candidate.session_name == args.session
                        || candidate.session_id.to_string() == args.session
                })
                .with_context(|| {
                    format!("session '{}' is not hosted by {}", args.session, server)
                })?;
            let saved = crate::storage::save_subscription(&server, session)?;
            println!(
                "Subscribed to session '{}' (id={}) at {}",
                saved.metadata.session_name, saved.metadata.session_id, server
            );
        }

        Command::SubscribeAll(args) => {
            let server = resolved_server(args.server.as_deref());
            let sessions = crate::transport_helpers::list_remote_sessions(&server).await?;
            for session in &sessions {
                crate::storage::save_subscription(&server, session)?;
            }
            println!(
                "Subscribed to {} open topic(s) at {}",
                sessions.len(),
                server
            );
        }

        Command::RemoteSessions(args) => {
            let server = resolved_server(args.server.as_deref());
            let sessions = crate::transport_helpers::list_remote_sessions(&server).await?;
            println!("{}", serde_json::to_string_pretty(&sessions)?);
        }

        // list all sessions and their details
        Command::Sessions => list_sessions()?,

        Command::MasterInstructions(args) => {
            let enrollment = select_enrollment(&args.session, false)?;
            let response = crate::transport::perform_request(
                &enrollment,
                protocol::ClientRequest::GetMasterInstructions {
                    session_id: enrollment.metadata.session_id,
                },
            )
            .await?;
            print_json(&crate::transport_helpers::response_to_json_string(
                response,
            )?)?;
        }

        // delete a session and all its enrollments
        Command::DeleteSession(args) => {
            let removed = delete_session_enrollments(&args.session)?;
            println!(
                "Removed {removed} saved subscription(s) for session '{}'.",
                args.session
            );
        }

        // list all entries in a session
        Command::List(args) => {
            let enrollment = select_enrollment(&args.session, false)?;
            check_cert_timeout(&enrollment);
            print_json(&perform_get(&enrollment, "list", None, None, None).await?)?;
        }

        // get a specific entry in a session
        Command::Get(args) => {
            let enrollment = select_enrollment(&args.session, false)?;
            check_cert_timeout(&enrollment);
            print_json(
                &perform_get(
                    &enrollment,
                    "get",
                    Some(&args.entry_name),
                    args.shelf.as_deref(),
                    args.book.as_deref(),
                )
                .await?,
            )?;
        }

        // shelves -> books -> entries

        // search for entries in a session
        Command::SearchEntries(args) => {
            let query = args.query.join(" ");
            let enrollment = select_enrollment(&args.session, false)?;
            check_cert_timeout(&enrollment);
            print_json(&perform_search(&enrollment, "search_entries", &query).await?)?;
        }

        // search for shelves in a session
        Command::SearchShelves(args) => {
            let query = args.query.join(" ");
            let enrollment = select_enrollment(&args.session, false)?;
            check_cert_timeout(&enrollment);
            print_json(&perform_search(&enrollment, "search_shelves", &query).await?)?;
        }
        // search for books in a session
        Command::SearchBooks(args) => {
            let query = args.query.join(" ");
            let enrollment = select_enrollment(&args.session, false)?;
            check_cert_timeout(&enrollment);
            print_json(&perform_search(&enrollment, "search_books", &query).await?)?;
        }
        // search for context in a session
        Command::SearchContext(args) => {
            let query = args.query.join(" ");
            let enrollment = select_enrollment(&args.session, false)?;
            check_cert_timeout(&enrollment);
            print_json(&perform_search(&enrollment, "search_context", &query).await?)?;
        }
        // search for deleted entries in a session
        Command::SearchDeleted(args) => {
            let query = args.query.join(" ");
            let enrollment = select_enrollment(&args.session, false)?;
            check_cert_timeout(&enrollment);
            print_json(&perform_search(&enrollment, "search_deleted", &query).await?)?;
        }

        Command::AddShelf(args) => {
            let shelf_description = args.shelf_description.join(" ");
            let enrollment = select_enrollment(&args.session, true)?;
            check_cert_timeout(&enrollment);
            print_json(
                &perform_add_shelf(&enrollment, &args.shelf_name, &shelf_description).await?,
            )?;
        }

        Command::AddBook(args) => {
            let book_description = args.book_description.join(" ");
            let enrollment = select_enrollment(&args.session, true)?;
            check_cert_timeout(&enrollment);
            print_json(
                &perform_add_book(&enrollment, &args.shelf, &args.book_name, &book_description)
                    .await?,
            )?;
        }

        Command::AddEntry(args) => {
            let entry_data = args.entry_data.join(" ");
            let labels = args.labels.as_deref().map(parse_labels).unwrap_or_default();
            let enrollment = select_enrollment(&args.session, true)?;
            check_cert_timeout(&enrollment);
            print_json(
                &perform_add_entry(
                    &enrollment,
                    &args.entry_name,
                    &args.entry_description,
                    &labels,
                    &entry_data,
                    &args.shelf,
                    &args.book,
                )
                .await?,
            )?;
        }

        // append content to an entry
        Command::Append(args) => {
            let content = args.content.join(" ");
            let enrollment = select_enrollment(&args.session, true)?;
            check_cert_timeout(&enrollment);
            let metadata = append_metadata_from_env();
            print_json(
                &perform_append(
                    &enrollment,
                    &args.entry_name,
                    &content,
                    metadata,
                    args.shelf.as_deref(),
                    args.book.as_deref(),
                )
                .await?,
            )?;
        }

        // soft delete — entry is archived, not destroyed
        Command::Delete(args) => {
            let enrollment = select_enrollment(&args.session, true)?;
            check_cert_timeout(&enrollment);
            print_json(
                &perform_delete(
                    &enrollment,
                    &args.entry_name,
                    args.shelf.as_deref(),
                    args.book.as_deref(),
                )
                .await?,
            )?;
        }

        Command::DeleteShelf(args) => {
            let enrollment = select_enrollment(&args.session, true)?;
            check_cert_timeout(&enrollment);
            print_json(&perform_delete_shelf(&enrollment, &args.shelf_name).await?)?;
        }

        Command::Restore(args) => {
            let enrollment = select_enrollment(&args.session, true)?;
            check_cert_timeout(&enrollment);
            print_json(&perform_restore(&enrollment, &args.entry_key).await?)?;
        }

        // get the history of an entry
        Command::History(args) => {
            let enrollment = select_enrollment(&args.session, false)?;
            check_cert_timeout(&enrollment);
            print_json(
                &perform_get(
                    &enrollment,
                    "get_history",
                    Some(&args.entry_name),
                    args.shelf.as_deref(),
                    args.book.as_deref(),
                )
                .await?,
            )?;
        }

        // export a session to a bundle
        Command::Export(args) => {
            if args.book.is_some() && args.shelf.is_none() {
                bail!("--book requires --shelf");
            }
            if !args.entry.is_empty() && (args.shelf.is_none() || args.book.is_none()) {
                bail!("--entry requires both --shelf and --book");
            }

            let enrollment = select_enrollment(&args.session, false)?;
            check_cert_timeout(&enrollment);

            let scope = match (&args.shelf, &args.book) {
                (None, _) => TransferScope::Session,
                (Some(shelf), None) => TransferScope::Shelf {
                    shelf: shelf.clone(),
                },
                (Some(shelf), Some(book)) if args.entry.is_empty() => TransferScope::Book {
                    shelf: shelf.clone(),
                    book: book.clone(),
                },
                (Some(shelf), Some(book)) => TransferScope::Entries {
                    shelf: shelf.clone(),
                    book: book.clone(),
                    entries: args.entry.clone(),
                },
            };
            let selector = TransferSelector {
                scope,
                include_history: !args.no_history,
            };
            let bundle = perform_export(&enrollment, selector).await?;
            let serialized = serde_json::to_string_pretty(&bundle)?;

            if let Some(mut path) = args.output {
                // default to .droplet extension
                if path.extension().is_none() {
                    path.set_extension("droplet");
                }
                std::fs::write(&path, serialized.as_bytes())
                    .with_context(|| format!("failed to write {}", path.display()))?;
                println!("{}", path.display());
            } else {
                println!("{serialized}");
            }
        }

        // import a droplet into a session
        Command::Import(args) => {
            let ext = args
                .bundle_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if ext != "droplet" && ext != "json" {
                eprintln!("warning: expected a .droplet or .json file, got .{ext}");
            }
            let policy = if args.overwrite {
                ConflictPolicy::Overwrite
            } else {
                match args.policy.as_str() {
                    "error" => ConflictPolicy::Error,
                    "overwrite" => ConflictPolicy::Overwrite,
                    "skip" => ConflictPolicy::Skip,
                    "merge-history" => ConflictPolicy::MergeHistory,
                    other => bail!(
                        "unknown policy '{other}'; use error, overwrite, skip, or merge-history"
                    ),
                }
            };
            let enrollment = select_enrollment(&args.session, true)?;
            check_cert_timeout(&enrollment);
            print_json(&perform_import(&enrollment, &args.bundle_path, policy).await?)?;
        }

        Command::BriefMe(args) => {
            let enrollment = select_enrollment(&args.session, false)?;
            check_cert_timeout(&enrollment);
            print_json(&perform_brief_me(&enrollment).await?)?;
        }

        Command::GetEntryAt(args) => {
            let enrollment = select_enrollment(&args.session, false)?;
            check_cert_timeout(&enrollment);
            print_json(
                &perform_get_entry_at(
                    &enrollment,
                    &args.entry_name,
                    args.shelf.as_deref(),
                    args.book.as_deref(),
                    &args.at,
                )
                .await?,
            )?;
        }

        Command::SetStatus(args) => {
            let enrollment = select_enrollment(&args.session, true)?;
            print_json(
                &perform_set_status(&enrollment, &args.team, &args.agent, &args.status.join(" "))
                    .await?,
            )?;
        }

        Command::ClearStatus(args) => {
            let enrollment = select_enrollment(&args.session, true)?;
            print_json(&perform_clear_status(&enrollment, &args.team, &args.agent).await?)?;
        }

        Command::TeamStatus(args) => {
            let enrollment = select_enrollment(&args.session, false)?;
            print_json(&perform_list_team_status(&enrollment, &args.team).await?)?;
        }

        Command::SearchTeamStatus(args) => {
            let enrollment = select_enrollment(&args.session, false)?;
            print_json(
                &perform_search_team_status(&enrollment, &args.team, &args.query.join(" ")).await?,
            )?;
        }
    }

    Ok(())
}

fn list_sessions() -> anyhow::Result<()> {
    let enrollments = load_enrollments()?;
    if enrollments.is_empty() {
        println!("No saved subscriptions found.");
        return Ok(());
    }

    for session in summarize_sessions(&enrollments) {
        println!(
            "session={} session_id={} endpoint={} owner={} visibility={} labels={} purpose={}",
            session.session_name,
            session.session_id,
            session.endpoint,
            session.owner,
            session.visibility,
            session.labels.join(","),
            session.purpose,
        );
    }

    Ok(())
}

fn append_metadata_from_env() -> AppendMetadata {
    // configure agent name and host name from environment variables
    let agent_name = std::env::var("CCP_AGENT_NAME")
        .ok()
        .filter(|value| !value.is_empty());
    let host_name = std::env::var("CCP_AGENT_HOST")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(hostname_fallback);
    let reason = std::env::var("CCP_APPEND_REASON")
        .ok()
        .filter(|value| !value.is_empty());
    AppendMetadata {
        agent_name,
        host_name,
        reason,
    }
}

fn resolved_server(explicit: Option<&str>) -> String {
    explicit
        .map(ToString::to_string)
        .or_else(|| std::env::var("CCP_SERVER_URL").ok())
        .unwrap_or_else(|| "http://192.168.130.34:1338".to_string())
}

fn hostname_fallback() -> Option<String> {
    // fallback to HOSTNAME environment variable if not set
    if let Ok(hostname) = std::env::var("HOSTNAME")
        && !hostname.trim().is_empty()
    {
        return Some(hostname);
    }
    None
}

fn parse_labels(raw: &str) -> Vec<String> {
    // parse the labels from the raw string
    // labels should be comma separated strings
    raw.split(',')
        .map(|label| label.trim())
        .filter(|label| !label.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub(crate) fn check_cert_timeout(_enrollment: &StoredEnrollment) {}

fn print_json(body: &str) -> anyhow::Result<()> {
    let value: serde_json::Value =
        serde_json::from_str(body).context("server did not return valid JSON")?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn parse_cli() -> Cli {
    match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => print_error(error, CLIENT_INPUT_FORMATS),
    }
}

fn print_error(error: clap::Error, input_formats: &str) -> ! {
    let use_stderr = error.use_stderr();
    let exit_code = error.exit_code();
    let _ = error.print();
    if use_stderr {
        let mut stderr = io::stderr();
        let _ = writeln!(stderr, "\n{input_formats}");
    } else {
        let mut stdout = io::stdout();
        let _ = writeln!(stdout, "\n{input_formats}");
    }
    std::process::exit(exit_code);
}
