// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::super::*;

impl ServerState {
    pub async fn append_to_entry(
        &self,
        session_id: i64,
        name: &str,
        shelf_name: Option<&str>,
        book_name: Option<&str>,
        auth_context: &ConnectionAuthContext,
        appended_content: &str,
        metadata: AppendMetadata,
    ) -> anyhow::Result<AppendResult> {
        self.ensure_write_access(session_id, auth_context).await?;
        let (path, key) = entry_path_for(name, shelf_name, book_name);
        let entry_lock = self.append_lock(session_id, &key).await;
        let _entry_guard = entry_lock.lock().await;

        // Validate entry exists under read lock
        {
            let sessions = self.sessions.read().await;
            let session = sessions
                .get(&session_id)
                .with_context(|| format!("unknown session id {session_id}"))?;
            if !session.entries.contains_key(&key) {
                bail!("entry '{name}' not found");
            }
        }

        let timestamp = current_timestamp_string()?;
        let operation_id = Uuid::new_v4().to_string();
        self.journal.append(JournalEntry::AppendEntry {
            session_id,
            name: name.to_string(),
            operation_id: operation_id.clone(),
            client_common_name: auth_context.common_name.clone(),
            agent_name: metadata.agent_name.clone(),
            host_name: metadata.host_name.clone(),
            reason: metadata.reason.clone(),
            appended_content: appended_content.to_string(),
            shelf_name: path.shelf_name.clone(),
            book_name: path.book_name.clone(),
            created_at: timestamp.clone(),
        })?;

        // Brief write lock for in-memory update only
        let updated_length = {
            let mut sessions = self.sessions.write().await;
            let session = sessions
                .get_mut(&session_id)
                .with_context(|| format!("unknown session id {session_id}"))?;
            let updated_length = {
                let entry = session
                    .entries
                    .get_mut(&key)
                    .with_context(|| format!("entry '{name}' not found"))?;

                SessionCache::refresh_appended_context(entry, appended_content);
                entry.updated_at = timestamp.clone();
                entry.history.push(MessageHistoryEntry {
                    operation_id: operation_id.clone(),
                    client_common_name: auth_context.common_name.clone(),
                    agent_name: metadata.agent_name,
                    host_name: metadata.host_name,
                    reason: metadata.reason,
                    appended_content: appended_content.to_string(),
                    created_at: timestamp.clone(),
                });
                entry.context.len()
            };
            session.invalidate_context_search_results();
            updated_length
        };

        Ok(AppendResult {
            operation_id,
            name: name.to_string(),
            appended_bytes: appended_content.len(),
            updated_context_length: updated_length,
        })
    }
}
