// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::sync::{Arc, OnceLock};

use super::super::*;
use tokio::sync::Semaphore;

static SEARCH_TASK_LIMITER: OnceLock<Arc<Semaphore>> = OnceLock::new();

#[derive(Clone)]
pub(super) struct SearchQuery {
    pub(super) raw: String,
    pub(super) terms: Vec<String>,
}

impl SearchQuery {
    pub(super) fn new(query: &str) -> anyhow::Result<Self> {
        optional_search_query(query).with_context(|| "query is required")
    }
}

#[derive(Clone)]
pub(super) struct Ranked<T> {
    pub(super) score: f64,
    pub(super) value: T,
}

impl<T> Ranked<T> {
    pub(super) fn new(score: f64, value: T) -> Self {
        Self { score, value }
    }
}

pub(super) fn optional_search_query(query: &str) -> Option<SearchQuery> {
    let raw = normalize_search_text(query);
    if raw.is_empty() {
        return None;
    }
    let terms = tokenize_search_text(&raw);
    if terms.is_empty() {
        return None;
    }
    Some(SearchQuery { raw, terms })
}

pub(super) fn score_search_candidate(
    query: &SearchQuery,
    literal_fields: &[&str],
    fuzzy_fields: &[&str],
) -> Option<f64> {
    let literal_haystack = normalize_search_text(&literal_fields.join(" "));
    let exact_phrase = !literal_haystack.is_empty() && literal_haystack.contains(&query.raw);
    let literal_term_hits = query
        .terms
        .iter()
        .filter(|term| literal_haystack.contains(term.as_str()))
        .count();
    let literal_all = literal_term_hits == query.terms.len();

    let fuzzy_tokens = fuzzy_fields
        .iter()
        .flat_map(|field| tokenize_search_text(field))
        .collect::<Vec<_>>();
    let normalized_fields = fuzzy_fields
        .iter()
        .map(|field| normalize_search_text(field))
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();

    let fuzzy_term_score = if fuzzy_tokens.is_empty() {
        0.0
    } else {
        query
            .terms
            .iter()
            .map(|term| {
                fuzzy_tokens
                    .iter()
                    .map(|candidate| fuzzy_similarity(term, candidate))
                    .fold(0.0, f64::max)
            })
            .sum::<f64>()
            / query.terms.len() as f64
    };
    let fuzzy_phrase_score = normalized_fields
        .iter()
        .map(|field| fuzzy_similarity(&query.raw, field))
        .fold(0.0, f64::max);
    let fuzzy_score = (fuzzy_term_score * 0.7) + (fuzzy_phrase_score * 0.3);

    if !exact_phrase && !literal_all && literal_term_hits == 0 && fuzzy_score < 0.78 {
        return None;
    }

    let literal_bonus = (if exact_phrase { 4.0 } else { 0.0 })
        + (if literal_all { 2.0 } else { 0.0 })
        + literal_term_hits as f64;
    Some(literal_bonus + fuzzy_score)
}

pub(super) fn score_precomputed_candidate(
    query: &SearchQuery,
    literal_haystack: &str,
    fuzzy_fields: &[String],
    fuzzy_tokens: &[String],
) -> Option<f64> {
    let exact_phrase = !literal_haystack.is_empty() && literal_haystack.contains(&query.raw);
    let literal_term_hits = query
        .terms
        .iter()
        .filter(|term| literal_haystack.contains(term.as_str()))
        .count();
    let literal_all = literal_term_hits == query.terms.len();

    if exact_phrase || literal_term_hits > 0 {
        let literal_bonus = (if exact_phrase { 4.0 } else { 0.0 })
            + (if literal_all { 2.0 } else { 0.0 })
            + literal_term_hits as f64;
        return Some(literal_bonus);
    }

    // Favor speed: only do typo-tolerant scoring for short single-term queries.
    if query.terms.len() != 1 || query.raw.len() > 12 {
        return None;
    }

    let fuzzy_term_score = if fuzzy_tokens.is_empty() {
        0.0
    } else {
        query
            .terms
            .iter()
            .map(|term| {
                fuzzy_tokens
                    .iter()
                    .map(|candidate| fuzzy_similarity(term, candidate))
                    .fold(0.0, f64::max)
            })
            .sum::<f64>()
            / query.terms.len() as f64
    };
    let fuzzy_phrase_score = fuzzy_fields
        .iter()
        .map(|field| fuzzy_similarity(&query.raw, field))
        .fold(0.0, f64::max);
    let fuzzy_score = (fuzzy_term_score * 0.7) + (fuzzy_phrase_score * 0.3);

    if fuzzy_score < 0.85 {
        return None;
    }

    Some(fuzzy_score)
}

pub(super) async fn run_bounded_search_task<F, T>(task: F) -> anyhow::Result<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let permit = search_task_limiter()
        .clone()
        .acquire_owned()
        .await
        .context("search task limiter closed")?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        task()
    })
    .await
    .context("search task panicked")
}

pub(super) fn build_context_snippets(context: &str, query: &SearchQuery) -> Vec<String> {
    let lowered = context.to_ascii_lowercase();
    let mut snippets = Vec::new();
    for term in &query.terms {
        let mut start_search = 0usize;
        while let Some(index) = lowered[start_search..].find(term) {
            let absolute = start_search + index;
            let snippet = snippet_around(context, absolute, term.len(), 100);
            if !snippets.iter().any(|existing| existing == &snippet) {
                snippets.push(snippet);
            }
            if snippets.len() >= 3 {
                return snippets;
            }
            start_search = absolute.saturating_add(term.len());
        }
    }
    if snippets.is_empty() {
        snippets.push(snippet_around(context, 0, 0, 100));
    }
    snippets
}

pub(crate) fn normalize_search_text(query: &str) -> String {
    query
        .split_whitespace()
        .map(|term| term.trim().to_ascii_lowercase())
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn tokenize_search_text(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(|term| term.trim().to_ascii_lowercase())
        .filter(|term| !term.is_empty())
        .collect()
}

fn fuzzy_similarity(left: &str, right: &str) -> f64 {
    normalized_levenshtein(left, right).max(jaro_winkler(left, right))
}

fn snippet_around(text: &str, start: usize, len: usize, radius: usize) -> String {
    let lower = start.saturating_sub(radius);
    let upper = text
        .len()
        .min(start.saturating_add(len).saturating_add(radius));
    text.get(lower..upper).unwrap_or(text).replace('\n', "\\n")
}

fn search_task_limiter() -> &'static Arc<Semaphore> {
    SEARCH_TASK_LIMITER.get_or_init(|| {
        let concurrency = std::thread::available_parallelism()
            .map(|parallelism| parallelism.get().saturating_mul(2))
            .unwrap_or(4)
            .max(2);
        Arc::new(Semaphore::new(concurrency))
    })
}
