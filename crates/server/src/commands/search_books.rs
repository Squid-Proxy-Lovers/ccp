// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::super::*;
use super::search_helpers::{Ranked, SearchQuery, score_search_candidate};

impl ServerState {
    pub async fn search_books(
        &self,
        session_id: i64,
        auth_context: &ConnectionAuthContext,
        query: &str,
    ) -> anyhow::Result<Vec<BookSummary>> {
        self.ensure_read_access(session_id, auth_context).await?;
        let search_query = SearchQuery::new(query)?;
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(&session_id)
            .with_context(|| format!("unknown session id {session_id}"))?;
        let mut books: Vec<Ranked<BookSummary>> = session
            .books
            .iter()
            .filter_map(|((shelf_name, book_name), book)| {
                let shelf_description = session.shelf_description(shelf_name);
                let entry_count = session
                    .book_entry_counts
                    .get(&(shelf_name.clone(), book_name.clone()))
                    .copied()
                    .unwrap_or(0);
                let score = score_search_candidate(
                    &search_query,
                    &[shelf_name, book_name, &shelf_description, &book.description],
                    &[shelf_name, book_name, &shelf_description, &book.description],
                )?;
                Some(Ranked::new(
                    score,
                    BookSummary {
                        shelf_name: shelf_name.clone(),
                        book_name: book_name.clone(),
                        shelf_description,
                        description: book.description.clone(),
                        entry_count,
                    },
                ))
            })
            .collect();
        books.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    (&left.value.shelf_name, &left.value.book_name)
                        .cmp(&(&right.value.shelf_name, &right.value.book_name))
                })
        });
        Ok(books.into_iter().map(|ranked| ranked.value).collect())
    }
}
