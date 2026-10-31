// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
#[derive(Default)]
pub(super) struct ReplayProgress {
    last_add: HashMap<(i64, String), usize>,
    operations: HashMap<(i64, String), HashSet<String>>,
}
impl ReplayProgress {
    pub(super) fn from_entries(entries: &[JournalEntry]) -> Self {
        let mut result = Self::default();
        for (index, entry) in entries.iter().enumerate() {
            if let JournalEntry::AddEntry {
                session_id,
                name,
                shelf_name,
                book_name,
                ..
            } = entry
            {
                let (_, key) = entry_path_for(name, Some(shelf_name), Some(book_name));
                result.last_add.insert((*session_id, key), index);
            }
        }
        result
    }
    pub(super) fn accepts(&self, index: usize, entry: &JournalEntry) -> bool {
        match entry {
            JournalEntry::AddEntry {
                session_id,
                name,
                shelf_name,
                book_name,
                ..
            }
            | JournalEntry::AppendEntry {
                session_id,
                name,
                shelf_name,
                book_name,
                ..
            } => {
                let (_, key) = entry_path_for(name, Some(shelf_name), Some(book_name));
                self.last_add
                    .get(&(*session_id, key))
                    .is_none_or(|last| index >= *last)
            }
            _ => true,
        }
    }
}
impl ServerState {
    pub(super) async fn replay_issued(&self, record: JournalEntry, _replay: &mut ReplayProgress) {
        let JournalEntry::IssuedCert {
            session_id,
            common_name,
            access_level,
            cert_pem,
            created_at,
            expires_at,
        } = record
        else {
            return;
        };
        if self
            .revoked_cert_common_names
            .read()
            .await
            .contains(&common_name)
        {
            return;
        }

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
    pub(super) async fn replay_add(&self, record: JournalEntry) {
        let JournalEntry::AddEntry {
            session_id,
            name,
            description,
            labels,
            context,
            shelf_name,
            book_name,
            created_at,
            updated_at,
        } = record
        else {
            return;
        };

        if let Some(session) = self.sessions.write().await.get_mut(&session_id) {
            let (path, key) = entry_path_for(&name, Some(&shelf_name), Some(&book_name));
            if session.entries.contains_key(&key) {
                return;
            }
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
    pub(super) async fn replay_append(&self, record: JournalEntry, replay: &mut ReplayProgress) {
        let JournalEntry::AppendEntry {
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
        } = record
        else {
            return;
        };

        let mut sessions = self.sessions.write().await;
        let Some(session) = sessions.get_mut(&session_id) else {
            return;
        };
        let (_, key) = entry_path_for(&name, Some(&shelf_name), Some(&book_name));
        let Some(entry) = session.entries.get_mut(&key) else {
            return;
        };
        let operations = replay
            .operations
            .entry((session_id, key))
            .or_insert_with(|| {
                entry
                    .history
                    .iter()
                    .map(|item| item.operation_id.clone())
                    .collect()
            });
        if !operation_id.is_empty() && !operations.insert(operation_id.clone()) {
            return;
        }
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
