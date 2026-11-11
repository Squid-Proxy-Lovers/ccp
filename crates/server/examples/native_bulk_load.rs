// SPDX-License-Identifier: AGPL-3.0-or-later
//! Opt-in startup benchmark; run with a fresh CCP_SERVER_DATA_DIR:
//! cargo run --release --locked -p server --example native_bulk_load
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, ensure};
use rusqlite::params;
use server::identity::ConnectionAuthContext;
use server::init::{initialize_cpp_server, journal_path, open_sqlite_connection};
use server::journal::{JournalEntry, JournalHandle};
use server::state::ServerState;

const ENTRIES: usize = 20_000;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let directory = std::env::var_os("CCP_SERVER_DATA_DIR")
        .context("set CCP_SERVER_DATA_DIR to a fresh benchmark directory")?;
    ensure!(
        !std::path::Path::new(&directory).exists(),
        "directory must be fresh"
    );
    let bootstrap = initialize_cpp_server("native-bulk-bench").await?;
    let session_id = bootstrap.session_id;
    let mut connection = open_sqlite_connection()?;
    let transaction = connection.transaction()?;
    transaction.execute("INSERT OR IGNORE INTO shelves (session_id, shelf_name, description) VALUES (?1, 'main', 'shelf')", [session_id])?;
    transaction.execute("INSERT OR IGNORE INTO books (session_id, shelf_name, book_name, description) VALUES (?1, 'main', 'default', 'book')", [session_id])?;
    {
        let mut insert = transaction.prepare("INSERT INTO message_packs (session_id, name, shelf_name, book_name, description, labels, context, created_at, updated_at) VALUES (?1, ?2, 'main', 'default', 'entry', 'label', 'context', '1', '1')")?;
        for index in 0..ENTRIES {
            insert.execute(params![session_id, format!("entry-{index:05}")])?;
        }
    }
    transaction.commit()?;
    let journal = Arc::new(JournalHandle::start(journal_path())?);
    timed_load(Arc::clone(&journal), session_id, "sqlite").await?;
    connection.execute("DELETE FROM message_packs", [])?;
    for index in 0..ENTRIES {
        journal.append(JournalEntry::AddEntry {
            session_id,
            name: format!("entry-{index:05}"),
            description: "entry".into(),
            labels: vec!["label".into()],
            context: "context".into(),
            shelf_name: "main".into(),
            book_name: "default".into(),
            created_at: "1".into(),
            updated_at: "1".into(),
        })?;
        journal.append(JournalEntry::AddBook {
            session_id,
            shelf_name: "main".into(),
            book_name: "default".into(),
            description: "updated book".into(),
        })?;
    }
    journal.shutdown()?;
    let journal = Arc::new(JournalHandle::start(journal_path())?);
    timed_load(Arc::clone(&journal), session_id, "journal + book updates").await?;
    journal.shutdown()?;
    Ok(())
}

async fn timed_load(
    journal: Arc<JournalHandle>,
    session_id: i64,
    phase: &str,
) -> anyhow::Result<()> {
    let started = Instant::now();
    let state = ServerState::load_from_storage(journal).await?;
    let elapsed = started.elapsed();
    state
        .record_issued_cert(session_id, "bench", "read_write", "test", "4102444800")
        .await?;
    let actor = ConnectionAuthContext {
        common_name: "bench".into(),
        session_id,
        can_write: true,
        can_revoke_others: false,
    };
    ensure!(
        state.list_entries(session_id, &actor).await?.len() == ENTRIES,
        "entry count differs"
    );
    println!("{phase}: {ENTRIES} entries in {elapsed:.3?}");
    Ok(())
}
