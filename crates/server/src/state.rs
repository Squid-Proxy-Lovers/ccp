// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::{Mutex, RwLock};

use anyhow::{Context, bail};
use protocol::{
    AddBookResult, AddShelfResult, AgentStatus, AppendMetadata, AppendResult, BookSummary,
    BundleEntry, ClearStatusResult, DeleteResult, DeleteShelfResult, DeletedEntrySummary,
    EntrySummary, MessageEntry, MessageHistoryEntry, RestoreResult, RevokeCertResult,
    SearchContextMatch, SessionMetadata, ShelfSummary,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Serialize;
use strsim::{jaro_winkler, normalized_levenshtein};
use uuid::Uuid;

use self::commands::search_helpers::{normalize_search_text, tokenize_search_text};
use crate::identity::ConnectionAuthContext;
use crate::init::{derive_auth_token, hash_token, open_sqlite_connection};
use crate::journal::{JournalEntry, JournalHandle, load_entries};

#[path = "commands/mod.rs"]
mod commands;
#[path = "database.rs"]
mod database;
#[cfg(test)]
#[path = "tests/state.rs"]
mod tests;

const DEFAULT_SHELF_NAME: &str = "main";
const DEFAULT_BOOK_NAME: &str = "default";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct EntryPath {
    shelf_name: String,
    book_name: String,
    chapter_name: String,
}

impl EntryPath {
    fn new(name: &str, shelf: Option<&str>, book: Option<&str>) -> Self {
        Self {
            shelf_name: normalize_segment(shelf, DEFAULT_SHELF_NAME),
            book_name: normalize_segment(book, DEFAULT_BOOK_NAME),
            chapter_name: normalize_segment(Some(name), ""),
        }
    }

    fn chapter_name(&self) -> &str {
        &self.chapter_name
    }

    fn shelf_name(&self) -> &str {
        &self.shelf_name
    }

    fn book_name(&self) -> &str {
        &self.book_name
    }

    fn key(&self) -> String {
        format!(
            "{}::{}::{}",
            self.shelf_name, self.book_name, self.chapter_name
        )
    }
}

fn validate_segment(value: &str) -> anyhow::Result<()> {
    if value.contains("::") {
        bail!("names cannot contain '::'");
    }
    Ok(())
}

fn normalize_segment(value: Option<&str>, default: &str) -> String {
    value
        .and_then(|segment| {
            let trimmed = segment.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .unwrap_or_else(|| default.to_string())
}

fn entry_path_for(name: &str, shelf: Option<&str>, book: Option<&str>) -> (EntryPath, String) {
    let path = EntryPath::new(name, shelf, book);
    let key = path.key();
    (path, key)
}

#[derive(Clone)]
pub struct AuthGrant {
    pub session_id: i64,
    pub access_level: String,
    pub metadata: SessionMetadata,
    token_nonce: String,
    token_hash: String,
    last_used_at: Option<String>,
}

#[derive(Clone)]
struct CertGrant {
    session_id: i64,
    access_level: String,
    cert_pem: String,
    created_at: String,
    expires_at: String,
}

#[derive(Clone)]
struct CachedMessagePack {
    path: EntryPath,
    summary: EntrySummary,
    description: String,
    labels: Vec<String>,
    context: String,
    created_at: String,
    updated_at: String,
    history: Vec<MessageHistoryEntry>,
    entry_search: EntrySearchCache,
    context_search: ContextSearchCache,
}

impl CachedMessagePack {
    fn name(&self) -> &str {
        self.path.chapter_name()
    }
}

#[derive(Clone)]
struct DeletedMessagePack {
    entry_key: String,
    session_id: i64,
    shelf_name: String,
    book_name: String,
    shelf_description: String,
    book_description: String,
    name: String,
    description: String,
    labels: Vec<String>,
    context: String,
    created_at: String,
    updated_at: String,
    deleted_at: String,
    deleted_by_client_common_name: String,
}

#[derive(Clone)]
struct CachedShelf {
    description: String,
}

#[derive(Clone)]
struct CachedBook {
    description: String,
}

#[derive(Clone)]
struct EntrySearchCache {
    literal_haystack: String,
    fuzzy_fields: Vec<String>,
    fuzzy_tokens: Vec<String>,
}

#[derive(Clone)]
struct ContextSearchCache {
    normalized_context: String,
    fuzzy_tokens: Vec<String>,
}

#[derive(Clone)]
struct SessionCache {
    metadata: SessionMetadata,
    is_active: bool,
    last_started_at: Option<String>,
    last_stopped_at: Option<String>,
    shelves: HashMap<String, CachedShelf>,
    books: HashMap<(String, String), CachedBook>,
    entries: HashMap<String, CachedMessagePack>,
    list_entries_cache: Vec<EntrySummary>,
    shelf_book_counts: HashMap<String, usize>,
    shelf_entry_counts: HashMap<String, usize>,
    book_entry_counts: HashMap<(String, String), usize>,
    entry_query_cache: HashMap<String, Vec<EntrySummary>>,
    context_query_cache: HashMap<String, Vec<SearchContextMatch>>,
}

impl SessionCache {
    fn new(metadata: SessionMetadata, is_active: bool) -> Self {
        let mut session = Self {
            metadata,
            is_active,
            last_started_at: None,
            last_stopped_at: None,
            shelves: HashMap::new(),
            books: HashMap::new(),
            entries: HashMap::new(),
            list_entries_cache: Vec::new(),
            shelf_book_counts: HashMap::new(),
            shelf_entry_counts: HashMap::new(),
            book_entry_counts: HashMap::new(),
            entry_query_cache: HashMap::new(),
            context_query_cache: HashMap::new(),
        };
        session.ensure_default_library();
        session
    }

    fn ensure_default_library(&mut self) {
        self.shelves
            .entry(DEFAULT_SHELF_NAME.to_string())
            .or_insert_with(|| CachedShelf {
                description: String::new(),
            });
        self.shelf_book_counts
            .entry(DEFAULT_SHELF_NAME.to_string())
            .or_insert(0);
        self.shelf_entry_counts
            .entry(DEFAULT_SHELF_NAME.to_string())
            .or_insert(0);
        if self
            .books
            .insert(
                (
                    DEFAULT_SHELF_NAME.to_string(),
                    DEFAULT_BOOK_NAME.to_string(),
                ),
                CachedBook {
                    description: String::new(),
                },
            )
            .is_none()
        {
            *self
                .shelf_book_counts
                .entry(DEFAULT_SHELF_NAME.to_string())
                .or_insert(0) += 1;
        }
        self.book_entry_counts
            .entry((
                DEFAULT_SHELF_NAME.to_string(),
                DEFAULT_BOOK_NAME.to_string(),
            ))
            .or_insert(0);
    }

    fn shelf_description(&self, shelf_name: &str) -> String {
        self.shelves
            .get(shelf_name)
            .map(|shelf| shelf.description.clone())
            .unwrap_or_default()
    }

    fn book_description(&self, shelf_name: &str, book_name: &str) -> String {
        self.books
            .get(&(shelf_name.to_string(), book_name.to_string()))
            .map(|book| book.description.clone())
            .unwrap_or_default()
    }

    fn upsert_library_metadata(
        &mut self,
        shelf_name: &str,
        book_name: &str,
        shelf_description: Option<&str>,
        book_description: Option<&str>,
    ) {
        let shelf_entry = self
            .shelves
            .entry(shelf_name.to_string())
            .or_insert_with(|| CachedShelf {
                description: String::new(),
            });
        self.shelf_book_counts
            .entry(shelf_name.to_string())
            .or_insert(0);
        self.shelf_entry_counts
            .entry(shelf_name.to_string())
            .or_insert(0);
        if let Some(description) = normalize_optional_text(shelf_description) {
            shelf_entry.description = description;
        }

        let book_key = (shelf_name.to_string(), book_name.to_string());
        let was_new_book = !self.books.contains_key(&book_key);
        let book_entry = self
            .books
            .entry(book_key.clone())
            .or_insert_with(|| CachedBook {
                description: String::new(),
            });
        if was_new_book {
            *self
                .shelf_book_counts
                .entry(shelf_name.to_string())
                .or_insert(0) += 1;
        }
        self.book_entry_counts.entry(book_key).or_insert(0);
        if let Some(description) = normalize_optional_text(book_description) {
            book_entry.description = description;
        }
    }

    fn add_shelf(&mut self, shelf_name: &str, description: Option<&str>) -> AddShelfResult {
        let shelf = self
            .shelves
            .entry(shelf_name.to_string())
            .or_insert_with(|| CachedShelf {
                description: String::new(),
            });
        self.shelf_book_counts
            .entry(shelf_name.to_string())
            .or_insert(0);
        self.shelf_entry_counts
            .entry(shelf_name.to_string())
            .or_insert(0);
        if let Some(description) = normalize_optional_text(description) {
            shelf.description = description;
        }
        AddShelfResult {
            shelf_name: shelf_name.to_string(),
            description: shelf.description.clone(),
        }
    }

    fn add_book(
        &mut self,
        shelf_name: &str,
        book_name: &str,
        description: Option<&str>,
    ) -> anyhow::Result<AddBookResult> {
        let Some(shelf) = self.shelves.get(shelf_name) else {
            bail!("shelf '{shelf_name}' does not exist");
        };
        let book = self
            .books
            .entry((shelf_name.to_string(), book_name.to_string()))
            .or_insert_with(|| CachedBook {
                description: String::new(),
            });
        let entry_count_key = (shelf_name.to_string(), book_name.to_string());
        if !self.book_entry_counts.contains_key(&entry_count_key) {
            *self
                .shelf_book_counts
                .entry(shelf_name.to_string())
                .or_insert(0) += 1;
            self.book_entry_counts.insert(entry_count_key, 0);
        }
        if let Some(description) = normalize_optional_text(description) {
            book.description = description;
        }
        Ok(AddBookResult {
            shelf_name: shelf_name.to_string(),
            book_name: book_name.to_string(),
            shelf_description: shelf.description.clone(),
            description: book.description.clone(),
        })
    }

    fn build_summary(
        &self,
        path: &EntryPath,
        description: &str,
        labels: &[String],
    ) -> EntrySummary {
        EntrySummary {
            name: path.chapter_name().to_string(),
            description: description.to_string(),
            labels: labels.to_vec(),
            shelf_name: path.shelf_name.clone(),
            book_name: path.book_name.clone(),
            shelf_description: self.shelf_description(path.shelf_name()),
            book_description: self.book_description(path.shelf_name(), path.book_name()),
        }
    }

    fn build_entry(
        &self,
        path: EntryPath,
        description: String,
        labels: Vec<String>,
        context: String,
        created_at: String,
        updated_at: String,
        history: Vec<MessageHistoryEntry>,
    ) -> CachedMessagePack {
        let summary = self.build_summary(&path, &description, &labels);
        CachedMessagePack {
            path,
            summary: summary.clone(),
            description,
            labels,
            context_search: build_context_search_cache(&context),
            entry_search: build_entry_search_cache(&summary),
            context,
            created_at,
            updated_at,
            history,
        }
    }

    fn insert_entry(&mut self, key: String, entry: CachedMessagePack) -> Option<CachedMessagePack> {
        let previous = self.entries.remove(&key);
        if let Some(previous) = &previous {
            self.decrement_entry_counts(previous);
        }
        self.increment_entry_counts(&entry);
        self.entries.insert(key, entry);
        self.rebuild_list_entries_cache();
        self.invalidate_entry_search_results();
        self.invalidate_context_search_results();
        previous
    }

    fn remove_entry(&mut self, key: &str) -> Option<CachedMessagePack> {
        let removed = self.entries.remove(key);
        if let Some(entry) = &removed {
            self.decrement_entry_counts(entry);
            self.rebuild_list_entries_cache();
            self.invalidate_entry_search_results();
            self.invalidate_context_search_results();
        }
        removed
    }

    fn refresh_library_metadata(&mut self, shelf_name: &str, book_name: Option<&str>) {
        let shelf_description = self.shelf_description(shelf_name);
        let book_descriptions = self
            .books
            .iter()
            .filter(|((candidate_shelf, candidate_book), _)| {
                candidate_shelf == shelf_name
                    && book_name.is_none_or(|book_name| candidate_book == book_name)
            })
            .map(|((_, candidate_book), book)| (candidate_book.clone(), book.description.clone()))
            .collect::<HashMap<_, _>>();
        for entry in self.entries.values_mut() {
            if entry.path.shelf_name() != shelf_name {
                continue;
            }
            if let Some(book_name) = book_name
                && entry.path.book_name() != book_name
            {
                continue;
            }
            entry.summary.shelf_description = shelf_description.clone();
            entry.summary.book_description = book_descriptions
                .get(entry.path.book_name())
                .cloned()
                .unwrap_or_default();
            entry.entry_search = build_entry_search_cache(&entry.summary);
        }
        self.rebuild_list_entries_cache();
        self.invalidate_entry_search_results();
    }

    fn refresh_appended_context(entry: &mut CachedMessagePack, appended_content: &str) {
        if entry.context.is_empty() {
            entry.context = appended_content.to_string();
        } else {
            entry.context.reserve(appended_content.len() + 1);
            entry.context.push('\n');
            entry.context.push_str(appended_content);
        }
        extend_context_search_cache(&mut entry.context_search, appended_content);
    }

    fn invalidate_entry_search_results(&mut self) {
        self.entry_query_cache.clear();
    }

    fn invalidate_context_search_results(&mut self) {
        self.context_query_cache.clear();
    }

    fn cache_entry_query(&mut self, key: String, results: Vec<EntrySummary>) {
        const MAX_QUERY_CACHE_SIZE: usize = 256;
        if self.entry_query_cache.len() >= MAX_QUERY_CACHE_SIZE {
            self.entry_query_cache.clear();
        }
        self.entry_query_cache.entry(key).or_insert(results);
    }

    fn cache_context_query(&mut self, key: String, results: Vec<SearchContextMatch>) {
        const MAX_QUERY_CACHE_SIZE: usize = 256;
        if self.context_query_cache.len() >= MAX_QUERY_CACHE_SIZE {
            self.context_query_cache.clear();
        }
        self.context_query_cache.entry(key).or_insert(results);
    }

    fn rebuild_list_entries_cache(&mut self) {
        self.list_entries_cache = self
            .entries
            .values()
            .map(|entry| entry.summary.clone())
            .collect();
        self.list_entries_cache.sort_by(|left, right| {
            (&left.shelf_name, &left.book_name, &left.name).cmp(&(
                &right.shelf_name,
                &right.book_name,
                &right.name,
            ))
        });
    }

    fn increment_entry_counts(&mut self, entry: &CachedMessagePack) {
        *self
            .shelf_entry_counts
            .entry(entry.path.shelf_name.clone())
            .or_insert(0) += 1;
        *self
            .book_entry_counts
            .entry((entry.path.shelf_name.clone(), entry.path.book_name.clone()))
            .or_insert(0) += 1;
    }

    fn decrement_entry_counts(&mut self, entry: &CachedMessagePack) {
        if let Some(count) = self.shelf_entry_counts.get_mut(entry.path.shelf_name()) {
            *count = count.saturating_sub(1);
        }
        if let Some(count) = self
            .book_entry_counts
            .get_mut(&(entry.path.shelf_name.clone(), entry.path.book_name.clone()))
        {
            *count = count.saturating_sub(1);
        }
    }

    fn remove_shelf_if_empty(&mut self, shelf_name: &str) {
        let entry_count = self
            .shelf_entry_counts
            .get(shelf_name)
            .copied()
            .unwrap_or(0);
        if entry_count > 0 {
            return;
        }
        let book_keys: Vec<_> = self
            .books
            .keys()
            .filter(|(s, _)| s == shelf_name)
            .cloned()
            .collect();
        for key in &book_keys {
            self.books.remove(key);
            self.book_entry_counts.remove(key);
            self.shelf_book_counts
                .entry(shelf_name.to_string())
                .and_modify(|c| *c = c.saturating_sub(1));
        }
        self.shelves.remove(shelf_name);
        self.shelf_book_counts.remove(shelf_name);
        self.shelf_entry_counts.remove(shelf_name);
        self.invalidate_entry_search_results();
    }
}

pub struct ServerState {
    sessions: RwLock<HashMap<i64, SessionCache>>,
    auth_tokens: RwLock<HashMap<String, AuthGrant>>,
    cert_grants: RwLock<HashMap<String, CertGrant>>,
    revoked_cert_common_names: RwLock<HashSet<String>>,
    append_locks: Mutex<HashMap<(i64, String), Arc<Mutex<()>>>>,
    journal: Arc<JournalHandle>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionActivity {
    pub id: String,
    pub session_id: i64,
    pub session_name: String,
    pub kind: String,
    pub shelf_name: String,
    pub book_name: String,
    pub entry_name: String,
    pub actor: String,
    pub agent_name: Option<String>,
    pub host_name: Option<String>,
    pub reason: Option<String>,
    pub content: String,
    pub created_at: String,
}

fn message_entry_from(entry: &CachedMessagePack) -> MessageEntry {
    MessageEntry {
        name: entry.summary.name.clone(),
        description: entry.summary.description.clone(),
        labels: entry.summary.labels.clone(),
        context: entry.context.clone(),
        shelf_name: entry.summary.shelf_name.clone(),
        book_name: entry.summary.book_name.clone(),
        shelf_description: entry.summary.shelf_description.clone(),
        book_description: entry.summary.book_description.clone(),
    }
}

impl ServerState {
    pub async fn recent_activity(
        &self,
        session_selector: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<SessionActivity>> {
        let sessions = self.sessions.read().await;
        let mut activity = Vec::new();

        for (session_id, session) in sessions.iter() {
            if session_selector.is_some_and(|selector| {
                selector != session_id.to_string() && selector != session.metadata.session_name
            }) {
                continue;
            }

            for entry in session.entries.values() {
                let mut original_content = entry.context.clone();
                for history in entry.history.iter().rev() {
                    if original_content == history.appended_content {
                        original_content.clear();
                    } else if let Some(prefix) =
                        original_content.strip_suffix(&format!("\n{}", history.appended_content))
                    {
                        original_content = prefix.to_string();
                    }
                }
                activity.push(SessionActivity {
                    id: format!(
                        "entry:{session_id}:{}:{}:{}",
                        entry.path.shelf_name(),
                        entry.path.book_name(),
                        entry.name()
                    ),
                    session_id: *session_id,
                    session_name: session.metadata.session_name.clone(),
                    kind: "entry_created".to_string(),
                    shelf_name: entry.path.shelf_name().to_string(),
                    book_name: entry.path.book_name().to_string(),
                    entry_name: entry.name().to_string(),
                    actor: "http-client".to_string(),
                    agent_name: None,
                    host_name: None,
                    reason: None,
                    content: original_content,
                    created_at: entry.created_at.clone(),
                });

                for (index, history) in entry.history.iter().enumerate() {
                    activity.push(SessionActivity {
                        id: format!(
                            "append:{session_id}:{}:{}:{}:{index}:{}",
                            entry.path.shelf_name(),
                            entry.path.book_name(),
                            entry.name(),
                            history.operation_id
                        ),
                        session_id: *session_id,
                        session_name: session.metadata.session_name.clone(),
                        kind: "entry_appended".to_string(),
                        shelf_name: entry.path.shelf_name().to_string(),
                        book_name: entry.path.book_name().to_string(),
                        entry_name: entry.name().to_string(),
                        actor: history.client_common_name.clone(),
                        agent_name: history.agent_name.clone(),
                        host_name: history.host_name.clone(),
                        reason: history.reason.clone(),
                        content: history.appended_content.clone(),
                        created_at: history.created_at.clone(),
                    });
                }
            }
        }

        if let Some(selector) = session_selector
            && !sessions.iter().any(|(id, session)| {
                selector == id.to_string() || selector == session.metadata.session_name
            })
        {
            bail!("unknown session '{selector}'");
        }

        activity.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        activity.truncate(limit.clamp(1, 500));
        Ok(activity)
    }

    pub async fn list_sessions(&self) -> Vec<SessionMetadata> {
        let sessions = self.sessions.read().await;
        let mut result = sessions
            .values()
            .map(|session| session.metadata.clone())
            .collect::<Vec<_>>();
        result.sort_by(|left, right| left.session_name.cmp(&right.session_name));
        result
    }

    pub async fn sessions_by_id(
        &self,
        session_ids: &[i64],
    ) -> anyhow::Result<Vec<SessionMetadata>> {
        let sessions = self.sessions.read().await;
        let mut result = Vec::with_capacity(session_ids.len());
        for session_id in session_ids {
            let session = sessions
                .get(session_id)
                .with_context(|| format!("unknown session id {session_id}"))?;
            result.push(session.metadata.clone());
        }
        Ok(result)
    }

    pub async fn create_session(&self, session_name: &str) -> anyhow::Result<SessionMetadata> {
        let name = session_name.trim();
        if name.is_empty() {
            bail!("session name must not be empty");
        }
        if let Some(existing) = self
            .sessions
            .read()
            .await
            .values()
            .find(|session| session.metadata.session_name == name)
            .map(|session| session.metadata.clone())
        {
            return Ok(existing);
        }
        let session_id = crate::init::create_session(name)?;
        let metadata = SessionMetadata {
            session_name: name.to_string(),
            session_id,
            description: "Runtime session for CCP inter-agent communication".to_string(),
            owner: String::new(),
            labels: Vec::new(),
            visibility: "public".to_string(),
            purpose: "Runtime session for CCP inter-agent communication".to_string(),
        };
        self.sessions
            .write()
            .await
            .insert(session_id, SessionCache::new(metadata.clone(), true));
        Ok(metadata)
    }

    pub async fn delete_session(&self, session_selector: &str) -> anyhow::Result<SessionMetadata> {
        let mut sessions = self.sessions.write().await;
        let session_id = sessions
            .iter()
            .find(|(id, session)| {
                id.to_string() == session_selector
                    || session.metadata.session_name == session_selector
            })
            .map(|(id, _)| *id)
            .with_context(|| format!("unknown session '{session_selector}'"))?;
        let removed = sessions
            .remove(&session_id)
            .expect("session was just resolved");
        drop(sessions);
        self.append_locks
            .lock()
            .await
            .retain(|(candidate_id, _), _| *candidate_id != session_id);
        let connection = open_sqlite_connection()?;
        connection
            .execute("DELETE FROM sessions WHERE id = ?1", [session_id])
            .with_context(|| format!("failed to delete session {session_id}"))?;
        Ok(removed.metadata)
    }

    pub async fn session_stats(
        &self,
        session_selector: &str,
    ) -> anyhow::Result<protocol::SessionStats> {
        let sessions = self.sessions.read().await;
        let session = sessions
            .iter()
            .find(|(id, session)| {
                id.to_string() == session_selector
                    || session.metadata.session_name == session_selector
            })
            .map(|(_, session)| session)
            .with_context(|| format!("unknown session '{session_selector}'"))?;
        Ok(protocol::SessionStats {
            session: session.metadata.clone(),
            shelves: session.shelves.len(),
            books: session.books.len(),
            entries: session.entries.len(),
            is_active: session.is_active,
        })
    }

    pub async fn all_session_stats(&self) -> Vec<protocol::SessionStats> {
        let sessions = self.sessions.read().await;
        let mut result = sessions
            .values()
            .map(|session| protocol::SessionStats {
                session: session.metadata.clone(),
                shelves: session.shelves.len(),
                books: session.books.len(),
                entries: session.entries.len(),
                is_active: session.is_active,
            })
            .collect::<Vec<_>>();
        result.sort_by(|left, right| left.session.session_name.cmp(&right.session.session_name));
        result
    }

    pub async fn master_instructions(
        &self,
        session_id: i64,
    ) -> anyhow::Result<protocol::MasterInstructions> {
        if !self.sessions.read().await.contains_key(&session_id) {
            bail!("unknown session id {session_id}");
        }
        let connection = open_sqlite_connection()?;
        let global = connection.query_row(
            "SELECT content, updated_at FROM global_master_instructions WHERE id = 1",
            [],
            |row| {
                Ok(protocol::InstructionRecord {
                    content: row.get(0)?,
                    updated_at: row.get(1)?,
                })
            },
        )?;
        let session = connection
            .query_row(
                "SELECT content, updated_at FROM session_master_instructions WHERE session_id = ?1",
                [session_id],
                |row| {
                    Ok(protocol::InstructionRecord {
                        content: row.get(0)?,
                        updated_at: row.get(1)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_else(|| protocol::InstructionRecord {
                content: String::new(),
                updated_at: String::new(),
            });
        Ok(protocol::MasterInstructions { global, session })
    }

    pub fn global_master_instructions(&self) -> anyhow::Result<protocol::InstructionRecord> {
        let connection = open_sqlite_connection()?;
        connection
            .query_row(
                "SELECT content, updated_at FROM global_master_instructions WHERE id = 1",
                [],
                |row| {
                    Ok(protocol::InstructionRecord {
                        content: row.get(0)?,
                        updated_at: row.get(1)?,
                    })
                },
            )
            .map_err(Into::into)
    }

    pub fn set_global_master_instructions(
        &self,
        content: &str,
    ) -> anyhow::Result<protocol::InstructionRecord> {
        let connection = open_sqlite_connection()?;
        connection.execute(
            "INSERT INTO global_master_instructions (id, content, updated_at)
             VALUES (1, ?1, CURRENT_TIMESTAMP)
             ON CONFLICT(id) DO UPDATE SET content = excluded.content, updated_at = CURRENT_TIMESTAMP",
            [content],
        )?;
        connection
            .query_row(
                "SELECT content, updated_at FROM global_master_instructions WHERE id = 1",
                [],
                |row| {
                    Ok(protocol::InstructionRecord {
                        content: row.get(0)?,
                        updated_at: row.get(1)?,
                    })
                },
            )
            .map_err(Into::into)
    }

    pub async fn set_session_master_instructions(
        &self,
        session_selector: &str,
        content: &str,
    ) -> anyhow::Result<protocol::InstructionRecord> {
        let session_id = self
            .sessions
            .read()
            .await
            .iter()
            .find(|(id, session)| {
                id.to_string() == session_selector
                    || session.metadata.session_name == session_selector
            })
            .map(|(id, _)| *id)
            .with_context(|| format!("unknown session '{session_selector}'"))?;
        let connection = open_sqlite_connection()?;
        connection.execute(
            "INSERT INTO session_master_instructions (session_id, content, updated_at)
             VALUES (?1, ?2, CURRENT_TIMESTAMP)
             ON CONFLICT(session_id) DO UPDATE SET content = excluded.content, updated_at = CURRENT_TIMESTAMP",
            params![session_id, content],
        )?;
        connection
            .query_row(
                "SELECT content, updated_at FROM session_master_instructions WHERE session_id = ?1",
                [session_id],
                |row| {
                    Ok(protocol::InstructionRecord {
                        content: row.get(0)?,
                        updated_at: row.get(1)?,
                    })
                },
            )
            .map_err(Into::into)
    }

    pub async fn resolve_auth_token(&self, token: &str) -> Option<AuthGrant> {
        self.auth_tokens.read().await.get(token).cloned()
    }

    pub async fn note_auth_token_used(&self, token: &str) -> anyhow::Result<()> {
        let timestamp = current_timestamp_string()?;
        let token_hash = hash_token(token);

        {
            let auth_tokens = self.auth_tokens.read().await;
            if !auth_tokens.contains_key(token) {
                bail!("unknown auth token");
            }
        }

        self.journal.append(JournalEntry::AuthTokenUsed {
            token_hash: token_hash.clone(),
            last_used_at: timestamp.clone(),
        })?;

        let mut auth_tokens = self.auth_tokens.write().await;
        let Some(grant) = auth_tokens.get_mut(token) else {
            bail!("unknown auth token");
        };
        grant.last_used_at = Some(timestamp);
        Ok(())
    }

    pub async fn record_issued_cert(
        &self,
        session_id: i64,
        common_name: &str,
        access_level: &str,
        cert_pem: &str,
        expires_at: &str,
    ) -> anyhow::Result<()> {
        let created_at = current_timestamp_string()?;
        self.journal.append(JournalEntry::IssuedCert {
            session_id,
            common_name: common_name.to_string(),
            access_level: access_level.to_string(),
            cert_pem: cert_pem.to_string(),
            created_at: created_at.clone(),
            expires_at: expires_at.to_string(),
        })?;

        self.cert_grants.write().await.insert(
            common_name.to_string(),
            CertGrant {
                session_id,
                access_level: access_level.to_string(),
                cert_pem: cert_pem.to_string(),
                created_at,
                expires_at: expires_at.to_string(),
            },
        );
        Ok(())
    }

    pub async fn mark_sessions_stopped(&self) -> anyhow::Result<()> {
        let timestamp = current_timestamp_string()?;
        let mut sessions = self.sessions.write().await;
        for session in sessions.values_mut() {
            session.is_active = false;
            session.last_stopped_at = Some(timestamp.clone());
        }
        Ok(())
    }

    async fn append_lock(&self, session_id: i64, entry_key: &str) -> Arc<Mutex<()>> {
        let mut append_locks = self.append_locks.lock().await;
        append_locks
            .entry((session_id, entry_key.to_string()))
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn remove_append_lock(&self, session_id: i64, entry_key: &str) {
        self.append_locks
            .lock()
            .await
            .remove(&(session_id, entry_key.to_string()));
    }

    pub async fn ensure_ping_access(
        &self,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<()> {
        let revoked = self.revoked_cert_common_names.read().await;
        if revoked.contains(&auth_context.common_name) {
            bail!("client does not have access for this session");
        }
        Ok(())
    }

    async fn ensure_read_access(
        &self,
        session_id: i64,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<()> {
        if auth_context.session_id != session_id {
            bail!("client certificate does not authorize this session");
        }

        let revoked = self.revoked_cert_common_names.read().await;
        if revoked.contains(&auth_context.common_name) {
            bail!("client does not have read access for this session");
        }
        Ok(())
    }

    async fn ensure_write_access(
        &self,
        session_id: i64,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<()> {
        if auth_context.session_id != session_id {
            bail!("client certificate does not authorize this session");
        }

        let revoked = self.revoked_cert_common_names.read().await;
        if revoked.contains(&auth_context.common_name) || !auth_context.can_write {
            bail!("client does not have write access for this session");
        }
        Ok(())
    }
}
fn current_timestamp_string() -> anyhow::Result<String> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX_EPOCH")?
        .as_secs()
        .to_string())
}

fn current_precise_timestamp_string() -> anyhow::Result<String> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX_EPOCH")?
        .as_millis()
        .to_string())
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn deleted_entry_key(name: &str, shelf: &str, book: &str, deleted_at: &str) -> String {
    format!("{shelf}::{book}::{name}::{deleted_at}")
}

fn parse_labels(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|label| label.trim())
        .filter(|label| !label.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn serialize_labels(labels: &[String]) -> String {
    labels.join(",")
}

fn build_entry_search_cache(summary: &EntrySummary) -> EntrySearchCache {
    let labels = serialize_labels(&summary.labels);
    let fuzzy_fields = [
        summary.shelf_name.as_str(),
        summary.book_name.as_str(),
        summary.name.as_str(),
        summary.description.as_str(),
        labels.as_str(),
        summary.shelf_description.as_str(),
        summary.book_description.as_str(),
    ]
    .into_iter()
    .map(normalize_search_text)
    .filter(|field: &String| !field.is_empty())
    .collect::<Vec<_>>();
    let literal_haystack = fuzzy_fields.join(" ");
    let fuzzy_tokens = fuzzy_fields
        .iter()
        .flat_map(|field| tokenize_search_text(field))
        .collect();
    EntrySearchCache {
        literal_haystack,
        fuzzy_fields,
        fuzzy_tokens,
    }
}

fn build_context_search_cache(context: &str) -> ContextSearchCache {
    let mut cache = ContextSearchCache {
        normalized_context: String::new(),
        fuzzy_tokens: Vec::new(),
    };
    extend_context_search_cache(&mut cache, context);
    cache
}

fn extend_context_search_cache(cache: &mut ContextSearchCache, appended_content: &str) {
    let normalized_chunk = normalize_search_text(appended_content);
    if !normalized_chunk.is_empty() {
        if !cache.normalized_context.is_empty() {
            cache.normalized_context.push(' ');
        }
        cache.normalized_context.push_str(&normalized_chunk);
    }
    cache
        .fuzzy_tokens
        .extend(tokenize_search_text(appended_content));
}

fn checkpoint_journal(journal: &JournalHandle) {
    let _ = journal.truncate_blocking();
}
