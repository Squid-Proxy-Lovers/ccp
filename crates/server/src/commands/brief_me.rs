// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::HashMap;

use protocol::{RecentEntry, SessionBrief, ShelfOverview};

use super::super::*;

impl ServerState {
    pub async fn brief_me(
        &self,
        session_id: i64,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<SessionBrief> {
        self.ensure_read_access(session_id, auth_context).await?;

        let sessions = self.sessions.read().await;
        let session = sessions
            .get(&session_id)
            .with_context(|| format!("unknown session id {session_id}"))?;

        let shelves: Vec<ShelfOverview> = session
            .shelves
            .iter()
            .map(|(shelf_name, cached_shelf)| {
                let book_count = session
                    .shelf_book_counts
                    .get(shelf_name)
                    .copied()
                    .unwrap_or(0);
                let entry_count = session
                    .shelf_entry_counts
                    .get(shelf_name)
                    .copied()
                    .unwrap_or(0);
                ShelfOverview {
                    shelf_name: shelf_name.clone(),
                    description: cached_shelf.description.clone(),
                    book_count,
                    entry_count,
                }
            })
            .collect();

        let mut sorted_entries: Vec<_> = session.entries.values().collect();
        sorted_entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let recent_entries: Vec<RecentEntry> = sorted_entries
            .into_iter()
            .take(10)
            .map(|entry| RecentEntry {
                name: entry.name().to_string(),
                description: entry.description.clone(),
                shelf_name: entry.path.shelf_name().to_string(),
                book_name: entry.path.book_name().to_string(),
                updated_at: entry.updated_at.clone(),
            })
            .collect();

        let mut label_counts: HashMap<String, usize> = HashMap::new();
        for entry in session.entries.values() {
            for label in &entry.labels {
                *label_counts.entry(label.clone()).or_insert(0) += 1;
            }
        }
        let mut label_pairs: Vec<_> = label_counts.into_iter().collect();
        label_pairs.sort_by(|a, b| b.1.cmp(&a.1));
        let frequent_labels: Vec<String> = label_pairs
            .into_iter()
            .take(10)
            .map(|(label, _)| label)
            .collect();

        Ok(SessionBrief {
            session_name: session.metadata.session_name.clone(),
            session_id: session.metadata.session_id,
            total_entries: session.entries.len(),
            total_shelves: session.shelves.len(),
            total_books: session.books.len(),
            shelves,
            recent_entries,
            frequent_labels,
        })
    }
}
