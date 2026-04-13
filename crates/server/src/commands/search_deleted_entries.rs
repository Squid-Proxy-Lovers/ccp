// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::super::*;
use super::search_helpers::{Ranked, optional_search_query, score_search_candidate};

impl ServerState {
    pub async fn search_deleted_entries(
        &self,
        session_id: i64,
        auth_context: &ConnectionAuthContext,
        query: &str,
    ) -> anyhow::Result<Vec<DeletedEntrySummary>> {
        self.ensure_read_access(session_id, auth_context).await?;
        let maybe_query = optional_search_query(query);

        // SQLite query + fuzzy scoring are both blocking work
        tokio::task::spawn_blocking(move || {
            let connection = open_sqlite_connection()?;
            let mut stmt = connection.prepare(
                "SELECT entry_key, name, description, labels, shelf_name, book_name, shelf_description, book_description, deleted_at, deleted_by_client_common_name
                 FROM deleted_message_packs
                 WHERE session_id = ?1
                 ORDER BY deleted_at DESC, name ASC",
            )?;
            let rows = stmt.query_map([session_id], |row| {
                Ok(DeletedEntrySummary {
                    entry_key: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    labels: parse_labels(&row.get::<_, String>(3)?),
                    shelf_name: row.get(4)?,
                    book_name: row.get(5)?,
                    shelf_description: row.get(6)?,
                    book_description: row.get(7)?,
                    deleted_at: row.get(8)?,
                    deleted_by_client_common_name: row.get(9)?,
                })
            })?;

            let mut results = Vec::new();
            for row in rows {
                let summary = row?;
                let score = maybe_query
                    .as_ref()
                    .and_then(|search_query| {
                        score_search_candidate(
                            search_query,
                            &[
                                &summary.shelf_name,
                                &summary.book_name,
                                &summary.name,
                                &summary.description,
                                &serialize_labels(&summary.labels),
                                &summary.shelf_description,
                                &summary.book_description,
                            ],
                            &[
                                &summary.shelf_name,
                                &summary.book_name,
                                &summary.name,
                                &summary.description,
                                &serialize_labels(&summary.labels),
                                &summary.shelf_description,
                                &summary.book_description,
                            ],
                        )
                    })
                    .unwrap_or(1.0);
                if maybe_query.is_none() || score > 0.0 {
                    results.push(Ranked::new(score, summary));
                }
            }
            results.sort_by(|left, right| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| right.value.deleted_at.cmp(&left.value.deleted_at))
                    .then_with(|| {
                        (
                            &left.value.shelf_name,
                            &left.value.book_name,
                            &left.value.name,
                        )
                            .cmp(&(
                                &right.value.shelf_name,
                                &right.value.book_name,
                                &right.value.name,
                            ))
                    })
            });
            Ok(results.into_iter().map(|ranked| ranked.value).collect())
        })
        .await
        .context("search task panicked")?
    }
}
