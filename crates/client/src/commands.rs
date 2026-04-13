// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use protocol::{AppendMetadata, ConflictPolicy, TransferScope, TransferSelector};

use crate::enrollment::enroll;
use crate::enrollment_structs::StoredEnrollment;
use crate::storage::{
    delete_session_enrollments, load_enrollments, select_enrollment, summarize_sessions,
};
use crate::transport::{
    perform_add_book, perform_add_entry, perform_add_shelf, perform_append, perform_brief_me,
    perform_delete, perform_delete_shelf, perform_export, perform_get, perform_get_entry_at,
    perform_import, perform_restore, perform_revoke_cert, perform_search,
};

const CLIENT_INPUT_FORMATS: &str = r#"Input formats:
  client enroll --redeem-url <url> --token <token>
  client sessions
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
  client revoke-cert <session> <client-common-name>
  client brief-me <session>
  client get-entry-at <session> <entry-name> --at <timestamp> [--shelf <name>] [--book <name>]

<session> can be a session name or session id discovered via `client sessions`."#;

#[derive(Parser)]
#[command(name = "client")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Enroll(EnrollArgs),
    Sessions,
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
    RevokeCert(RevokeCertArgs),
    BriefMe(SessionSelectorArgs),
    GetEntryAt(GetEntryAtArgs),
}

#[derive(Args)]
struct EnrollArgs {
    #[arg(long, value_name = "url")]
    redeem_url: String,
    #[arg(long, value_name = "token")]
    token: String,
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
struct RevokeCertArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(value_name = "client-common-name")]
    client_common_name: String,
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

pub(crate) async fn run() -> anyhow::Result<()> {
    let cli = parse_cli();

    match cli.command {
        // authentication command, send redeem url and token to the server
        // save the enrollment to the filesystem (~/.ccp/enrollments/<session-name>/<session-id>/enrollment.json)
        Command::Enroll(args) => {
            enroll(&args.redeem_url, &args.token).await?;
        }

        // list all sessions and their details
        Command::Sessions => list_sessions()?,

        // delete a session and all its enrollments
        Command::DeleteSession(args) => {
            let removed = delete_session_enrollments(&args.session)?;
            println!(
                "Removed {removed} saved enrollment(s) for session '{}'.",
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

        // revoke a client certificate
        Command::RevokeCert(args) => {
            let enrollment = select_enrollment(&args.session, true)?;
            check_cert_timeout(&enrollment);
            print_json(&perform_revoke_cert(&enrollment, &args.client_common_name).await?)?;
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
    }

    Ok(())
}

fn list_sessions() -> anyhow::Result<()> {
    let enrollments = load_enrollments()?;
    if enrollments.is_empty() {
        println!("No saved enrollments found.");
        return Ok(());
    }

    for session in summarize_sessions(&enrollments) {
        println!(
            "session={} session_id={} access={} certs={} endpoint={} owner={} visibility={} labels={} purpose={} client_cert_expires_at={}{}",
            session.session_name,
            session.session_id,
            session.available_access.join(","),
            session.enrollment_count,
            session.endpoint,
            session.owner,
            session.visibility,
            session.labels.join(","),
            session.purpose,
            session.latest_client_cert_expires_at,
            session
                .cert_warning
                .as_ref()
                .map(|warning| format!(" cert_warning={warning}"))
                .unwrap_or_default(),
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

pub(crate) fn check_cert_timeout(enrollment: &StoredEnrollment) {
    // check if the client certificate has expired
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let expires_at = enrollment.metadata.client_cert_expires_at;
    if now >= expires_at {
        eprintln!(
            "WARNING: client certificate expired at unix={expires_at}; request a new enrollment token and re-enroll"
        );
        return;
    }

    let warning_window = std::env::var("CCP_CERT_WARNING_WINDOW_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    if warning_window == 0 {
        return;
    }
    let remaining = expires_at.saturating_sub(now);
    if remaining <= warning_window {
        eprintln!(
            "WARNING: client certificate expires soon at unix={expires_at}; request a new enrollment token before it expires"
        );
    }
}

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
