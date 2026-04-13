// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::super::*;

impl ServerState {
    pub async fn restore_deleted_entry(
        &self,
        session_id: i64,
        entry_key: &str,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<RestoreResult> {
        self.ensure_write_access(session_id, auth_context).await?;
        let (deleted_entry, deleted_history) =
            super::super::database::load_deleted_entry(entry_key)?
                .with_context(|| format!("deleted entry '{entry_key}' not found"))?;
        if deleted_entry.session_id != session_id {
            bail!("deleted entry '{entry_key}' not found for this session");
        }

        let (restored_path, restored_key) = entry_path_for(
            &deleted_entry.name,
            Some(&deleted_entry.shelf_name),
            Some(&deleted_entry.book_name),
        );

        // snapshot shelves/books before we touch them
        let shelves_snapshot;
        let books_snapshot;
        let book_counts_snapshot;
        let shelf_book_counts_snapshot;
        let shelf_entry_counts_snapshot;

        {
            let mut sessions = self.sessions.write().await;
            let session = sessions
                .get_mut(&session_id)
                .with_context(|| format!("unknown session id {session_id}"))?;
            if session.entries.contains_key(&restored_key) {
                bail!(
                    "entry '{}' already exists in shelf '{}' / book '{}'",
                    deleted_entry.name,
                    deleted_entry.shelf_name,
                    deleted_entry.book_name
                );
            }

            shelves_snapshot = session.shelves.clone();
            books_snapshot = session.books.clone();
            book_counts_snapshot = session.book_entry_counts.clone();
            shelf_book_counts_snapshot = session.shelf_book_counts.clone();
            shelf_entry_counts_snapshot = session.shelf_entry_counts.clone();

            session.upsert_library_metadata(
                restored_path.shelf_name(),
                restored_path.book_name(),
                Some(&deleted_entry.shelf_description),
                Some(&deleted_entry.book_description),
            );
            session.refresh_library_metadata(
                restored_path.shelf_name(),
                Some(restored_path.book_name()),
            );
            let restored_entry = session.build_entry(
                restored_path.clone(),
                deleted_entry.description.clone(),
                deleted_entry.labels.clone(),
                deleted_entry.context.clone(),
                deleted_entry.created_at.clone(),
                deleted_entry.updated_at.clone(),
                deleted_history.clone(),
            );
            session.insert_entry(restored_key.clone(), restored_entry);
        }

        let restored_entry = {
            let sessions = self.sessions.read().await;
            let session = sessions
                .get(&session_id)
                .with_context(|| format!("unknown session id {session_id}"))?;
            let entry = session.entries.get(&restored_key).with_context(|| {
                format!("entry '{}' not found after restore", deleted_entry.name)
            })?;
            message_entry_from(entry)
        };
        if let Err(error) = super::super::database::persist_restored_entry(
            &deleted_entry,
            deleted_history.as_slice(),
        ) {
            // put everything back the way it was
            let mut sessions = self.sessions.write().await;
            let session = sessions
                .get_mut(&session_id)
                .with_context(|| format!("unknown session id {session_id}"))?;
            session.remove_entry(&restored_key);
            session.shelves = shelves_snapshot;
            session.books = books_snapshot;
            session.book_entry_counts = book_counts_snapshot;
            session.shelf_book_counts = shelf_book_counts_snapshot;
            session.shelf_entry_counts = shelf_entry_counts_snapshot;
            // push restored descriptions back into existing entry summaries
            session.refresh_library_metadata(restored_path.shelf_name(), None);
            session.rebuild_list_entries_cache();
            return Err(error);
        }

        checkpoint_journal(&self.journal);
        Ok(RestoreResult {
            entry_key: deleted_entry.entry_key,
            restored_entry,
        })
    }
}
