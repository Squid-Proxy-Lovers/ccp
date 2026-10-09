// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::super::*;

impl ServerState {
    pub async fn delete_shelf(
        &self,
        session_id: i64,
        shelf_name: &str,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<DeleteShelfResult> {
        let _mutation = self.mutation_lock.lock().await;
        self.ensure_write_access(session_id, auth_context).await?;
        let shelf_name = normalize_segment(Some(shelf_name), "");
        if shelf_name.is_empty() {
            bail!("shelf_name is required for delete_shelf");
        }

        self.checkpoint_locked().await?;

        let (deleted_books, deleted_entries, mut original_session) = {
            let mut sessions = self.sessions.write().await;
            let session = sessions
                .get_mut(&session_id)
                .with_context(|| format!("unknown session id {session_id}"))?;

            if !session.shelves.contains_key(&shelf_name) {
                bail!("shelf '{shelf_name}' not found");
            }

            let original_session = session.clone();

            // count what we're about to remove
            let deleted_books = session
                .books
                .keys()
                .filter(|(s, _)| s == &shelf_name)
                .count();
            let deleted_entries = session
                .entries
                .values()
                .filter(|e| e.path.shelf_name() == shelf_name)
                .count();

            // remove all entries in this shelf
            let entry_keys: Vec<String> = session
                .entries
                .iter()
                .filter(|(_, e)| e.path.shelf_name() == shelf_name)
                .map(|(k, _)| k.clone())
                .collect();
            for key in &entry_keys {
                session.remove_entry(key);
            }

            session.remove_shelf_if_empty(&shelf_name);

            (deleted_books, deleted_entries, original_session)
        };

        // persist the full snapshot so SQLite matches memory
        if let Err(error) = super::super::database::Snapshot::capture(self)
            .await
            .and_then(super::super::database::persist_snapshot)
        {
            let mut sessions = self.sessions.write().await;
            if let Some(failed_session) = sessions.get(&session_id) {
                // Keep generations monotonic across rollback. Searches may have
                // captured the temporary deleted state while persistence ran.
                original_session.entry_search_generation = failed_session.entry_search_generation;
                original_session.context_search_generation =
                    failed_session.context_search_generation;
            }
            original_session.invalidate_entry_search_results();
            original_session.invalidate_context_search_results();
            sessions.insert(session_id, original_session);
            return Err(error);
        }

        Ok(DeleteShelfResult {
            shelf_name,
            deleted_books,
            deleted_entries,
        })
    }
}
