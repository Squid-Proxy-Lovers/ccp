// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::super::*;
use super::search_helpers::{
    Ranked, SearchQuery, build_context_snippets, run_bounded_search_task,
    score_precomputed_candidate,
};

impl ServerState {
    pub async fn search_context(
        &self,
        session_id: i64,
        auth_context: &ConnectionAuthContext,
        query: &str,
    ) -> anyhow::Result<Vec<SearchContextMatch>> {
        self.ensure_read_access(session_id, auth_context).await?;
        let search_query = SearchQuery::new(query)?;

        let sessions = self.sessions.read().await;
        let session = sessions
            .get(&session_id)
            .with_context(|| format!("unknown session id {session_id}"))?;
        if let Some(cached) = session.context_query_cache.get(&search_query.raw) {
            return Ok(cached.clone());
        }
        let snapshots = session
            .entries
            .values()
            .map(|entry| {
                (
                    entry.summary.clone(),
                    entry.context.clone(),
                    entry.context_search.clone(),
                )
            })
            .collect::<Vec<_>>();
        drop(sessions);

        let cache_key = search_query.raw.clone();
        let results: Vec<SearchContextMatch> = run_bounded_search_task(move || {
            let mut matches: Vec<Ranked<SearchContextMatch>> = snapshots
                .into_iter()
                .filter_map(|(summary, context, context_search)| {
                    let score = score_precomputed_candidate(
                        &search_query,
                        &context_search.normalized_context,
                        std::slice::from_ref(&context_search.normalized_context),
                        &context_search.fuzzy_tokens,
                    )?;
                    Some(Ranked::new(
                        score,
                        SearchContextMatch {
                            name: summary.name.clone(),
                            description: summary.description.clone(),
                            snippets: build_context_snippets(&context, &search_query),
                            shelf_name: summary.shelf_name.clone(),
                            book_name: summary.book_name.clone(),
                            shelf_description: summary.shelf_description.clone(),
                            book_description: summary.book_description.clone(),
                        },
                    ))
                })
                .collect();
            matches.sort_by(|left, right| {
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
            matches.into_iter().map(|ranked| ranked.value).collect()
        })
        .await?;

        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .with_context(|| format!("unknown session id {session_id}"))?;
        session.cache_context_query(cache_key, results.clone());
        Ok(results)
    }
}
