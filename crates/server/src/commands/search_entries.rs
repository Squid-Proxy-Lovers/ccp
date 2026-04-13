// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::super::*;
use super::search_helpers::{
    Ranked, SearchQuery, run_bounded_search_task, score_precomputed_candidate,
};

impl ServerState {
    pub async fn search_entries(
        &self,
        session_id: i64,
        auth_context: &ConnectionAuthContext,
        query: &str,
    ) -> anyhow::Result<Vec<EntrySummary>> {
        self.ensure_read_access(session_id, auth_context).await?;
        let search_query = SearchQuery::new(query)?;

        let sessions = self.sessions.read().await;
        let session = sessions
            .get(&session_id)
            .with_context(|| format!("unknown session id {session_id}"))?;
        if let Some(cached) = session.entry_query_cache.get(&search_query.raw) {
            return Ok(cached.clone());
        }
        let snapshots = session
            .entries
            .values()
            .map(|entry| (entry.summary.clone(), entry.entry_search.clone()))
            .collect::<Vec<_>>();
        drop(sessions);

        let cache_key = search_query.raw.clone();
        let results: Vec<EntrySummary> = run_bounded_search_task(move || {
            let mut entries: Vec<Ranked<EntrySummary>> = snapshots
                .into_iter()
                .filter_map(|(summary, search_cache)| {
                    let score = score_precomputed_candidate(
                        &search_query,
                        &search_cache.literal_haystack,
                        &search_cache.fuzzy_fields,
                        &search_cache.fuzzy_tokens,
                    )?;
                    Some(Ranked::new(score, summary))
                })
                .collect();
            entries.sort_by(|left, right| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
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
            entries.into_iter().map(|ranked| ranked.value).collect()
        })
        .await?;

        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .with_context(|| format!("unknown session id {session_id}"))?;
        session.cache_entry_query(cache_key, results.clone());
        Ok(results)
    }
}
