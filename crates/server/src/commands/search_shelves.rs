// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::super::*;
use super::search_helpers::{Ranked, SearchQuery, score_search_candidate};

impl ServerState {
    pub async fn search_shelves(
        &self,
        session_id: i64,
        auth_context: &ConnectionAuthContext,
        query: &str,
    ) -> anyhow::Result<Vec<ShelfSummary>> {
        self.ensure_read_access(session_id, auth_context).await?;
        let search_query = SearchQuery::new(query)?;
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(&session_id)
            .with_context(|| format!("unknown session id {session_id}"))?;
        let mut shelves: Vec<Ranked<ShelfSummary>> = session
            .shelves
            .iter()
            .filter_map(|(shelf_name, shelf)| {
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
                let score = score_search_candidate(
                    &search_query,
                    &[shelf_name, &shelf.description],
                    &[shelf_name, &shelf.description],
                )?;
                Some(Ranked::new(
                    score,
                    ShelfSummary {
                        shelf_name: shelf_name.clone(),
                        description: shelf.description.clone(),
                        book_count,
                        entry_count,
                    },
                ))
            })
            .collect();
        shelves.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.value.shelf_name.cmp(&right.value.shelf_name))
        });
        Ok(shelves.into_iter().map(|ranked| ranked.value).collect())
    }
}
