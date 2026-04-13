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
        self.ensure_write_access(session_id, auth_context).await?;
        let shelf_name = normalize_segment(Some(shelf_name), "");
        if shelf_name.is_empty() {
            bail!("shelf_name is required for delete_shelf");
        }

        let (deleted_books, deleted_entries) = {
            let mut sessions = self.sessions.write().await;
            let session = sessions
                .get_mut(&session_id)
                .with_context(|| format!("unknown session id {session_id}"))?;

            if !session.shelves.contains_key(&shelf_name) {
                bail!("shelf '{shelf_name}' not found");
            }

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

            (deleted_books, deleted_entries)
        };

        // persist the full snapshot so SQLite matches memory
        super::super::database::persist_snapshot(
            super::super::database::Snapshot::capture(self).await,
        )?;

        checkpoint_journal(&self.journal);
        Ok(DeleteShelfResult {
            shelf_name,
            deleted_books,
            deleted_entries,
        })
    }
}
