// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

impl ServerState {
    pub async fn load_from_storage(journal: Arc<JournalHandle>) -> anyhow::Result<Self> {
        let connection = open_sqlite_connection()?;
        let mut sessions = load_sessions(&connection)?;
        load_shelves(&connection, &mut sessions)?;
        load_books(&connection, &mut sessions)?;
        load_message_packs(&connection, &mut sessions)?;
        load_message_history(&connection, &mut sessions)?;
        let auth_tokens = load_auth_tokens(&connection)?;
        let cert_grants = load_cert_grants(&connection)?;
        let revoked_cert_common_names = load_revoked_cert_common_names(&connection)?;

        let state = Self {
            sessions: RwLock::new(sessions),
            auth_tokens: RwLock::new(auth_tokens),
            cert_grants: RwLock::new(cert_grants),
            revoked_cert_common_names: RwLock::new(revoked_cert_common_names),
            append_locks: Mutex::new(HashMap::new()),
            journal,
        };

        for entry in load_entries(state.journal.path())? {
            state.apply_journal_entry(entry).await;
        }

        Ok(state)
    }

    pub async fn persist_snapshot_to_sqlite(&self) -> anyhow::Result<()> {
        persist_snapshot(Snapshot::capture(self).await)
    }

    pub fn try_persist_snapshot_to_sqlite(&self) -> anyhow::Result<bool> {
        let Some(snapshot) = Snapshot::try_capture(self) else {
            return Ok(false);
        };
        persist_snapshot(snapshot)?;
        Ok(true)
    }

    async fn apply_journal_entry(&self, entry: JournalEntry) {
        match entry {
            JournalEntry::AuthTokenUsed {
                token_hash,
                last_used_at,
            } => {
                if let Some((_, grant)) = self
                    .auth_tokens
                    .write()
                    .await
                    .iter_mut()
                    .find(|(_, grant)| grant.token_hash == token_hash)
                {
                    grant.last_used_at = Some(last_used_at);
                }
            }
            JournalEntry::IssuedCert {
                session_id,
                common_name,
                access_level,
                cert_pem,
                created_at,
                expires_at,
            } => {
                self.cert_grants.write().await.insert(
                    common_name,
                    CertGrant {
                        session_id,
                        access_level,
                        cert_pem,
                        created_at,
                        expires_at,
                    },
                );
            }
            JournalEntry::AddShelf {
                session_id,
                shelf_name,
                description,
            } => {
                if let Some(session) = self.sessions.write().await.get_mut(&session_id) {
                    session.add_shelf(&shelf_name, Some(&description));
                    session.refresh_library_metadata(&shelf_name, None);
                }
            }
            JournalEntry::AddBook {
                session_id,
                shelf_name,
                book_name,
                description,
            } => {
                if let Some(session) = self.sessions.write().await.get_mut(&session_id) {
                    let _ = session.add_book(&shelf_name, &book_name, Some(&description));
                    session.refresh_library_metadata(&shelf_name, Some(&book_name));
                }
            }
            JournalEntry::AddEntry {
                session_id,
                name,
                description,
                labels,
                context,
                shelf_name,
                book_name,
                created_at,
                updated_at,
            } => {
                if let Some(session) = self.sessions.write().await.get_mut(&session_id) {
                    let (path, key) = entry_path_for(&name, Some(&shelf_name), Some(&book_name));
                    let history = session
                        .entries
                        .get(&key)
                        .map(|entry| entry.history.clone())
                        .unwrap_or_default();
                    let entry = session.build_entry(
                        path,
                        description,
                        labels,
                        context,
                        created_at,
                        updated_at,
                        history,
                    );
                    session.insert_entry(key, entry);
                }
            }
            // Transfer journal entries are audit records only; state is fully
            // captured in the SQLite snapshot so replay is a no-op.
            JournalEntry::TransferExported { .. }
            | JournalEntry::TransferImported { .. }
            | JournalEntry::TransferImportFailed { .. } => {}

            JournalEntry::AppendEntry {
                session_id,
                name,
                operation_id,
                client_common_name,
                agent_name,
                host_name,
                reason,
                appended_content,
                shelf_name,
                book_name,
                created_at,
            } => {
                let mut sessions = self.sessions.write().await;
                let Some(session) = sessions.get_mut(&session_id) else {
                    return;
                };
                let (_, key) = entry_path_for(&name, Some(&shelf_name), Some(&book_name));
                let Some(entry) = session.entries.get_mut(&key) else {
                    return;
                };
                SessionCache::refresh_appended_context(entry, &appended_content);
                entry.updated_at = created_at.clone();
                entry.history.push(MessageHistoryEntry {
                    operation_id,
                    client_common_name,
                    agent_name,
                    host_name,
                    reason,
                    appended_content,
                    created_at,
                });
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct Snapshot {
    sessions: HashMap<i64, SessionCache>,
    auth_tokens: HashMap<String, AuthGrant>,
    cert_grants: HashMap<String, CertGrant>,
}

impl Snapshot {
    pub(super) async fn capture(state: &ServerState) -> Self {
        Self {
            sessions: state.sessions.read().await.clone(),
            auth_tokens: state.auth_tokens.read().await.clone(),
            cert_grants: state.cert_grants.read().await.clone(),
        }
    }

    fn try_capture(state: &ServerState) -> Option<Self> {
        Some(Self {
            sessions: state.sessions.try_read().ok()?.clone(),
            auth_tokens: state.auth_tokens.try_read().ok()?.clone(),
            cert_grants: state.cert_grants.try_read().ok()?.clone(),
        })
    }
}

pub(super) fn persist_snapshot(snapshot: Snapshot) -> anyhow::Result<()> {
    let mut connection = open_sqlite_connection()?;
    let transaction = connection
        .transaction()
        .context("failed to start sqlite snapshot transaction")?;
    persist_snapshot_transaction(&transaction, &snapshot)?;
    transaction
        .commit()
        .context("failed to commit sqlite snapshot transaction")?;
    Ok(())
}

pub(super) fn persist_deleted_entry(
    deleted_entry: &DeletedMessagePack,
    deleted_history: &[MessageHistoryEntry],
) -> anyhow::Result<()> {
    let mut connection = open_sqlite_connection()?;
    let transaction = connection
        .transaction()
        .context("failed to start sqlite delete transaction")?;
    insert_deleted_entry(&transaction, deleted_entry, deleted_history)?;
    transaction
        .execute(
            "DELETE FROM message_packs WHERE session_id = ?1 AND shelf_name = ?2 AND book_name = ?3 AND name = ?4",
            params![
                deleted_entry.session_id,
                deleted_entry.shelf_name,
                deleted_entry.book_name,
                deleted_entry.name
            ],
        )
        .context("failed to delete active message pack during archive")?;
    transaction
        .commit()
        .context("failed to commit sqlite delete transaction")?;
    Ok(())
}

pub(super) fn persist_transfer_log(
    session_id: i64,
    direction: &str,
    scope_json: &str,
    bundle_sha256: &str,
    policy: Option<&str>,
    outcome: &str,
    entry_count: usize,
    created_at: &str,
    created_by: &str,
) -> anyhow::Result<()> {
    let connection = open_sqlite_connection()?;
    connection
        .execute(
            "INSERT INTO transfer_log (
                session_id, direction, scope_json, bundle_sha256,
                policy, outcome, entry_count, created_at, created_by
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                session_id,
                direction,
                scope_json,
                bundle_sha256,
                policy,
                outcome,
                entry_count as i64,
                created_at,
                created_by,
            ],
        )
        .context("failed to insert transfer_log row")?;
    Ok(())
}

pub(super) fn persist_restored_entry(
    deleted_entry: &DeletedMessagePack,
    deleted_history: &[MessageHistoryEntry],
) -> anyhow::Result<()> {
    let mut connection = open_sqlite_connection()?;
    let transaction = connection
        .transaction()
        .context("failed to start sqlite restore transaction")?;
    persist_library_metadata_transaction(
        &transaction,
        deleted_entry.session_id,
        &deleted_entry.shelf_name,
        &deleted_entry.shelf_description,
        &deleted_entry.book_name,
        &deleted_entry.book_description,
    )?;
    transaction
        .execute(
            "INSERT INTO message_packs (
                session_id,
                name,
                shelf_name,
                book_name,
                description,
                labels,
                context,
                created_at,
                updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                deleted_entry.session_id,
                deleted_entry.name,
                deleted_entry.shelf_name,
                deleted_entry.book_name,
                deleted_entry.description,
                serialize_labels(&deleted_entry.labels),
                deleted_entry.context,
                deleted_entry.created_at,
                deleted_entry.updated_at
            ],
        )
        .with_context(|| format!("failed to restore message pack {}", deleted_entry.name))?;
    let message_pack_id = transaction.last_insert_rowid();
    for history in deleted_history {
        transaction
            .execute(
                "INSERT INTO message_history (
                    message_pack_id,
                    operation_id,
                    client_common_name,
                    agent_name,
                    host_name,
                    reason,
                    appended_content,
                    created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    message_pack_id,
                    history.operation_id,
                    history.client_common_name,
                    history.agent_name,
                    history.host_name,
                    history.reason,
                    history.appended_content,
                    history.created_at
                ],
            )
            .context("failed to restore message history entry")?;
    }
    transaction
        .execute(
            "DELETE FROM deleted_message_history WHERE entry_key = ?1",
            [deleted_entry.entry_key.as_str()],
        )
        .context("failed to delete archived history during restore")?;
    transaction
        .execute(
            "DELETE FROM deleted_message_packs WHERE entry_key = ?1",
            [deleted_entry.entry_key.as_str()],
        )
        .context("failed to delete archived entry during restore")?;
    transaction
        .commit()
        .context("failed to commit sqlite restore transaction")?;
    Ok(())
}

pub(super) fn persist_revoked_cert(
    session_id: i64,
    client_common_name: &str,
    grant: &CertGrant,
    revoked_at: &str,
) -> anyhow::Result<()> {
    let mut connection = open_sqlite_connection()?;
    let transaction = connection
        .transaction()
        .context("failed to start sqlite revoke transaction")?;
    transaction
        .execute(
            "INSERT INTO issued_client_certs (
                session_id,
                common_name,
                access_level,
                cert_pem,
                created_at,
                expires_at,
                revoked_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(common_name) DO UPDATE SET
                session_id = excluded.session_id,
                access_level = excluded.access_level,
                cert_pem = excluded.cert_pem,
                created_at = excluded.created_at,
                expires_at = excluded.expires_at,
                revoked_at = excluded.revoked_at",
            params![
                session_id,
                client_common_name,
                grant.access_level,
                grant.cert_pem,
                grant.created_at,
                grant.expires_at,
                revoked_at
            ],
        )
        .context("failed to mark revoked client certificate")?;
    transaction
        .commit()
        .context("failed to commit sqlite revoke transaction")?;
    Ok(())
}

pub(super) fn load_deleted_entry(
    entry_key: &str,
) -> anyhow::Result<Option<(DeletedMessagePack, Vec<MessageHistoryEntry>)>> {
    let connection = open_sqlite_connection()?;
    let deleted_entry = connection
        .query_row(
            "SELECT entry_key, session_id, shelf_name, book_name, shelf_description, book_description, name, description, labels, context, created_at, updated_at, deleted_at, deleted_by_client_common_name
             FROM deleted_message_packs
             WHERE entry_key = ?1",
            [entry_key],
            |row| {
                Ok(DeletedMessagePack {
                    entry_key: row.get(0)?,
                    session_id: row.get(1)?,
                    shelf_name: row.get(2)?,
                    book_name: row.get(3)?,
                    shelf_description: row.get(4)?,
                    book_description: row.get(5)?,
                    name: row.get(6)?,
                    description: row.get(7)?,
                    labels: parse_labels(&row.get::<_, String>(8)?),
                    context: row.get(9)?,
                    created_at: row.get(10)?,
                    updated_at: row.get(11)?,
                    deleted_at: row.get(12)?,
                    deleted_by_client_common_name: row.get(13)?,
                })
            },
        )
        .optional()?;
    let Some(deleted_entry) = deleted_entry else {
        return Ok(None);
    };

    let mut stmt = connection.prepare(
        "SELECT operation_id, client_common_name, agent_name, host_name, reason, appended_content, created_at
         FROM deleted_message_history
         WHERE entry_key = ?1
         ORDER BY id",
    )?;
    let rows = stmt.query_map([entry_key], |row| {
        Ok(MessageHistoryEntry {
            operation_id: row.get(0)?,
            client_common_name: row.get(1)?,
            agent_name: row.get(2)?,
            host_name: row.get(3)?,
            reason: row.get(4)?,
            appended_content: row.get(5)?,
            created_at: row.get(6)?,
        })
    })?;
    let mut history = Vec::new();
    for row in rows {
        history.push(row?);
    }

    Ok(Some((deleted_entry, history)))
}

fn insert_deleted_entry(
    transaction: &Transaction<'_>,
    deleted_entry: &DeletedMessagePack,
    deleted_history: &[MessageHistoryEntry],
) -> anyhow::Result<()> {
    transaction
        .execute(
            "INSERT INTO deleted_message_packs (
                entry_key,
                session_id,
                shelf_name,
                book_name,
                shelf_description,
                book_description,
                name,
                description,
                labels,
                context,
                created_at,
                updated_at,
                deleted_at,
                deleted_by_client_common_name
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                deleted_entry.entry_key,
                deleted_entry.session_id,
                deleted_entry.shelf_name,
                deleted_entry.book_name,
                deleted_entry.shelf_description,
                deleted_entry.book_description,
                deleted_entry.name,
                deleted_entry.description,
                serialize_labels(&deleted_entry.labels),
                deleted_entry.context,
                deleted_entry.created_at,
                deleted_entry.updated_at,
                deleted_entry.deleted_at,
                deleted_entry.deleted_by_client_common_name
            ],
        )
        .with_context(|| format!("failed to archive deleted entry {}", deleted_entry.name))?;

    for history in deleted_history {
        transaction
            .execute(
                "INSERT INTO deleted_message_history (
                    entry_key,
                    operation_id,
                    client_common_name,
                    agent_name,
                    host_name,
                    reason,
                    appended_content,
                    created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    deleted_entry.entry_key,
                    history.operation_id,
                    history.client_common_name,
                    history.agent_name,
                    history.host_name,
                    history.reason,
                    history.appended_content,
                    history.created_at
                ],
            )
            .context("failed to archive deleted history entry")?;
    }
    Ok(())
}

fn persist_library_metadata_transaction(
    transaction: &Transaction<'_>,
    session_id: i64,
    shelf_name: &str,
    shelf_description: &str,
    book_name: &str,
    book_description: &str,
) -> anyhow::Result<()> {
    transaction
        .execute(
            "INSERT INTO shelves (
                session_id,
                shelf_name,
                description
             ) VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id, shelf_name) DO UPDATE SET
                description = CASE
                    WHEN excluded.description != '' THEN excluded.description
                    ELSE shelves.description
                END,
                updated_at = CURRENT_TIMESTAMP",
            params![session_id, shelf_name, shelf_description],
        )
        .with_context(|| format!("failed to persist shelf metadata for {shelf_name}"))?;
    transaction
        .execute(
            "INSERT INTO books (
                session_id,
                shelf_name,
                book_name,
                description
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id, shelf_name, book_name) DO UPDATE SET
                description = CASE
                    WHEN excluded.description != '' THEN excluded.description
                    ELSE books.description
                END,
                updated_at = CURRENT_TIMESTAMP",
            params![session_id, shelf_name, book_name, book_description],
        )
        .with_context(|| format!("failed to persist book metadata for {shelf_name}/{book_name}"))?;
    Ok(())
}

fn persist_snapshot_transaction(
    transaction: &Transaction<'_>,
    snapshot: &Snapshot,
) -> anyhow::Result<()> {
    for (session_id, session) in &snapshot.sessions {
        transaction
            .execute(
                "UPDATE sessions
                 SET description = ?2,
                     owner = ?3,
                     labels = ?4,
                     visibility = ?5,
                     purpose = ?6,
                     is_active = ?7,
                     last_started_at = ?8,
                     last_stopped_at = ?9
                 WHERE id = ?1",
                params![
                    session_id,
                    session.metadata.description,
                    session.metadata.owner,
                    serialize_labels(&session.metadata.labels),
                    session.metadata.visibility,
                    session.metadata.purpose,
                    if session.is_active { 1 } else { 0 },
                    session.last_started_at,
                    session.last_stopped_at
                ],
            )
            .with_context(|| {
                format!("failed to update session {}", session.metadata.session_name)
            })?;
    }

    transaction
        .execute("DELETE FROM books", [])
        .context("failed to clear books during snapshot")?;
    transaction
        .execute("DELETE FROM shelves", [])
        .context("failed to clear shelves during snapshot")?;
    for (session_id, session) in &snapshot.sessions {
        for (shelf_name, shelf) in &session.shelves {
            transaction
                .execute(
                    "INSERT INTO shelves (
                        session_id,
                        shelf_name,
                        description
                    ) VALUES (?1, ?2, ?3)",
                    params![session_id, shelf_name, shelf.description],
                )
                .with_context(|| format!("failed to persist shelf {shelf_name}"))?;
        }
        for ((shelf_name, book_name), book) in &session.books {
            transaction
                .execute(
                    "INSERT INTO books (
                        session_id,
                        shelf_name,
                        book_name,
                        description
                    ) VALUES (?1, ?2, ?3, ?4)",
                    params![session_id, shelf_name, book_name, book.description],
                )
                .with_context(|| format!("failed to persist book {shelf_name}/{book_name}"))?;
        }
    }

    for (token, grant) in &snapshot.auth_tokens {
        transaction
            .execute(
                "UPDATE auth_tokens
                 SET last_used_at = ?2
                 WHERE token_hash = ?1",
                params![grant.token_hash, grant.last_used_at],
            )
            .with_context(|| format!("failed to update auth token {token}"))?;
    }

    transaction
        .execute(
            "DELETE FROM issued_client_certs WHERE revoked_at IS NULL",
            [],
        )
        .context("failed to clear issued client certs during snapshot")?;
    for (common_name, grant) in &snapshot.cert_grants {
        transaction
            .execute(
                "INSERT INTO issued_client_certs (
                    session_id,
                    common_name,
                    access_level,
                    cert_pem,
                    created_at,
                    expires_at,
                    revoked_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
                params![
                    grant.session_id,
                    common_name,
                    grant.access_level,
                    grant.cert_pem,
                    grant.created_at,
                    grant.expires_at
                ],
            )
            .with_context(|| format!("failed to persist issued cert {common_name}"))?;
    }

    transaction
        .execute("DELETE FROM message_history", [])
        .context("failed to clear message history during snapshot")?;
    transaction
        .execute("DELETE FROM message_packs", [])
        .context("failed to clear message packs during snapshot")?;

    for (session_id, session) in &snapshot.sessions {
        for entry in session.entries.values() {
            transaction
                .execute(
                    "INSERT INTO message_packs (
                        session_id,
                        name,
                        shelf_name,
                        book_name,
                        description,
                        labels,
                        context,
                        created_at,
                        updated_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        session_id,
                        entry.name(),
                        entry.path.shelf_name(),
                        entry.path.book_name(),
                        &entry.description,
                        serialize_labels(&entry.labels),
                        &entry.context,
                        &entry.created_at,
                        &entry.updated_at
                    ],
                )
                .with_context(|| format!("failed to persist message pack {}", entry.name()))?;

            let message_pack_id: i64 = transaction
                .query_row(
                    "SELECT id FROM message_packs WHERE session_id = ?1 AND shelf_name = ?2 AND book_name = ?3 AND name = ?4",
                    params![
                        session_id,
                        entry.path.shelf_name(),
                        entry.path.book_name(),
                        entry.name()
                    ],
                    |row| row.get(0),
                )
                .context("failed to reload message pack id during snapshot")?;

            for history in &entry.history {
                transaction
                    .execute(
                        "INSERT INTO message_history (
                            message_pack_id,
                            operation_id,
                            client_common_name,
                            agent_name,
                            host_name,
                            reason,
                            appended_content,
                            created_at
                        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        params![
                            message_pack_id,
                            history.operation_id,
                            history.client_common_name,
                            history.agent_name,
                            history.host_name,
                            history.reason,
                            history.appended_content,
                            history.created_at
                        ],
                    )
                    .context("failed to persist message history entry")?;
            }
        }
    }
    Ok(())
}

fn load_sessions(connection: &Connection) -> anyhow::Result<HashMap<i64, SessionCache>> {
    let mut sessions = HashMap::new();
    let mut stmt = connection.prepare(
        "SELECT id, name, description, owner, labels, visibility, purpose, is_active, last_started_at, last_stopped_at
         FROM sessions",
    )?;
    let rows = stmt.query_map([], |row| {
        let labels: String = row.get(4)?;
        let mut session = SessionCache::new(
            SessionMetadata {
                session_id: row.get(0)?,
                session_name: row.get(1)?,
                description: row.get(2)?,
                owner: row.get(3)?,
                labels: parse_labels(&labels),
                visibility: row.get(5)?,
                purpose: row.get(6)?,
            },
            row.get::<_, i64>(7)? != 0,
        );
        session.last_started_at = row.get(8)?;
        session.last_stopped_at = row.get(9)?;
        Ok((row.get::<_, i64>(0)?, session))
    })?;

    for row in rows {
        let (id, session) = row?;
        sessions.insert(id, session);
    }
    Ok(sessions)
}

fn load_message_packs(
    connection: &Connection,
    sessions: &mut HashMap<i64, SessionCache>,
) -> anyhow::Result<()> {
    let mut stmt = connection.prepare(
        "SELECT session_id, name, shelf_name, book_name, description, labels, context, created_at, updated_at
         FROM message_packs",
    )?;
    let rows = stmt.query_map([], |row| {
        let name: String = row.get(1)?;
        let shelf_name: String = row.get(2)?;
        let book_name: String = row.get(3)?;
        let (path, key) = entry_path_for(&name, Some(&shelf_name), Some(&book_name));
        Ok((
            row.get::<_, i64>(0)?,
            key,
            path,
            row.get::<_, String>(4)?,
            parse_labels(&row.get::<_, String>(5)?),
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
        ))
    })?;

    for row in rows {
        let (session_id, key, path, description, labels, context, created_at, updated_at) = row?;
        if let Some(session) = sessions.get_mut(&session_id) {
            session.upsert_library_metadata(path.shelf_name(), path.book_name(), None, None);
            let entry = session.build_entry(
                path,
                description,
                labels,
                context,
                created_at,
                updated_at,
                Vec::new(),
            );
            session.insert_entry(key, entry);
        }
    }
    Ok(())
}

fn load_shelves(
    connection: &Connection,
    sessions: &mut HashMap<i64, SessionCache>,
) -> anyhow::Result<()> {
    let mut stmt = connection.prepare(
        "SELECT session_id, shelf_name, description
         FROM shelves",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    for row in rows {
        let (session_id, shelf_name, description) = row?;
        if let Some(session) = sessions.get_mut(&session_id) {
            session
                .shelves
                .insert(shelf_name.clone(), CachedShelf { description });
            session
                .shelf_book_counts
                .entry(shelf_name.clone())
                .or_insert(0);
            session.shelf_entry_counts.entry(shelf_name).or_insert(0);
        }
    }
    Ok(())
}

fn load_books(
    connection: &Connection,
    sessions: &mut HashMap<i64, SessionCache>,
) -> anyhow::Result<()> {
    let mut stmt = connection.prepare(
        "SELECT session_id, shelf_name, book_name, description
         FROM books",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;

    for row in rows {
        let (session_id, shelf_name, book_name, description) = row?;
        if let Some(session) = sessions.get_mut(&session_id) {
            let key = (shelf_name.clone(), book_name.clone());
            let was_new_book = !session.books.contains_key(&key);
            session
                .books
                .insert(key.clone(), CachedBook { description });
            if was_new_book {
                *session.shelf_book_counts.entry(shelf_name).or_insert(0) += 1;
            }
            session.book_entry_counts.entry(key).or_insert(0);
        }
    }
    Ok(())
}

fn load_message_history(
    connection: &Connection,
    sessions: &mut HashMap<i64, SessionCache>,
) -> anyhow::Result<()> {
    let mut stmt = connection.prepare(
        "SELECT mp.session_id,
                mp.name,
                mp.shelf_name,
                mp.book_name,
                mh.operation_id,
                mh.client_common_name,
                mh.agent_name,
                mh.host_name,
                mh.reason,
                mh.appended_content,
                mh.created_at
         FROM message_history mh
         INNER JOIN message_packs mp ON mp.id = mh.message_pack_id
         ORDER BY mh.id",
    )?;
    let rows = stmt.query_map([], |row| {
        let session_id = row.get::<_, i64>(0)?;
        let name: String = row.get(1)?;
        let shelf_name: String = row.get(2)?;
        let book_name: String = row.get(3)?;
        let (_, key) = entry_path_for(&name, Some(&shelf_name), Some(&book_name));
        Ok((
            session_id,
            key,
            MessageHistoryEntry {
                operation_id: row.get(4)?,
                client_common_name: row.get(5)?,
                agent_name: row.get(6)?,
                host_name: row.get(7)?,
                reason: row.get(8)?,
                appended_content: row.get(9)?,
                created_at: row.get(10)?,
            },
        ))
    })?;

    for row in rows {
        let (session_id, entry_name, history_entry) = row?;
        if let Some(session) = sessions.get_mut(&session_id)
            && let Some(entry) = session.entries.get_mut(&entry_name)
        {
            entry.history.push(history_entry);
        }
    }
    Ok(())
}

fn load_auth_tokens(connection: &Connection) -> anyhow::Result<HashMap<String, AuthGrant>> {
    let mut stmt = connection.prepare(
        "SELECT auth_tokens.session_id,
                auth_tokens.access_level,
                auth_tokens.token_nonce,
                auth_tokens.token_hash,
                auth_tokens.last_used_at,
                sessions.name,
                sessions.description,
                sessions.owner,
                sessions.labels,
                sessions.visibility,
                sessions.purpose
         FROM auth_tokens
         INNER JOIN sessions ON sessions.id = auth_tokens.session_id
         WHERE sessions.is_active = 1
           AND (auth_tokens.expires_at IS NULL OR auth_tokens.expires_at > CURRENT_TIMESTAMP)",
    )?;

    let rows = stmt.query_map([], |row| {
        let labels: String = row.get(8)?;
        let session_id: i64 = row.get(0)?;
        let access_level: String = row.get(1)?;
        let token_nonce = row.get::<_, Option<String>>(2)?.unwrap_or_default();
        let token_hash: String = row.get(3)?;
        Ok(AuthGrant {
            session_id,
            access_level,
            metadata: SessionMetadata {
                session_name: row.get(5)?,
                session_id,
                description: row.get(6)?,
                owner: row.get(7)?,
                labels: parse_labels(&labels),
                visibility: row.get(9)?,
                purpose: row.get(10)?,
            },
            token_nonce,
            token_hash,
            last_used_at: row.get(4)?,
        })
    })?;

    let mut auth_tokens = HashMap::new();
    for row in rows {
        let grant = row?;
        if grant.token_nonce.trim().is_empty() {
            continue;
        }
        let token = derive_auth_token(grant.session_id, &grant.access_level, &grant.token_nonce)?;
        auth_tokens.insert(token, grant);
    }
    Ok(auth_tokens)
}

fn load_cert_grants(connection: &Connection) -> anyhow::Result<HashMap<String, CertGrant>> {
    let mut stmt = connection.prepare(
        "SELECT common_name, session_id, access_level, cert_pem, created_at, expires_at
         FROM issued_client_certs
         WHERE revoked_at IS NULL",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            CertGrant {
                session_id: row.get(1)?,
                access_level: row.get(2)?,
                cert_pem: row.get(3)?,
                created_at: row.get(4)?,
                expires_at: row.get(5)?,
            },
        ))
    })?;

    let mut cert_grants = HashMap::new();
    for row in rows {
        let (common_name, grant) = row?;
        cert_grants.insert(common_name, grant);
    }
    Ok(cert_grants)
}

fn load_revoked_cert_common_names(connection: &Connection) -> anyhow::Result<HashSet<String>> {
    let mut stmt = connection.prepare(
        "SELECT common_name
         FROM issued_client_certs
         WHERE revoked_at IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;

    let mut revoked = HashSet::new();
    for row in rows {
        revoked.insert(row?);
    }
    Ok(revoked)
}
