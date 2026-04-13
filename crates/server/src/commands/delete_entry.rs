// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::super::*;

impl ServerState {
    pub async fn delete_entry(
        &self,
        session_id: i64,
        name: &str,
        shelf_name: Option<&str>,
        book_name: Option<&str>,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<DeleteResult> {
        self.ensure_write_access(session_id, auth_context).await?;

        let deleted_at = current_precise_timestamp_string()?;
        let (_path, key) = entry_path_for(name, shelf_name, book_name);
        let (deleted_entry, deleted_history, deleted_result) = {
            let mut sessions = self.sessions.write().await;
            let session = sessions
                .get_mut(&session_id)
                .with_context(|| format!("unknown session id {session_id}"))?;
            let entry = session
                .remove_entry(&key)
                .with_context(|| format!("entry '{name}' not found"))?;
            let entry_key = deleted_entry_key(
                entry.name(),
                &entry.path.shelf_name,
                &entry.path.book_name,
                &deleted_at,
            );
            let shelf_description = session.shelf_description(entry.path.shelf_name());
            let book_description =
                session.book_description(entry.path.shelf_name(), entry.path.book_name());
            let deleted_entry = DeletedMessagePack {
                entry_key: entry_key.clone(),
                session_id,
                shelf_name: entry.path.shelf_name.clone(),
                book_name: entry.path.book_name.clone(),
                shelf_description,
                book_description,
                name: entry.name().to_string(),
                description: entry.description.clone(),
                labels: entry.labels.clone(),
                context: entry.context.clone(),
                created_at: entry.created_at.clone(),
                updated_at: entry.updated_at.clone(),
                deleted_at: deleted_at.clone(),
                deleted_by_client_common_name: auth_context.common_name.clone(),
            };
            let deleted_history = entry.history.clone();
            let deleted_result = DeleteResult {
                entry_key,
                name: entry.name().to_string(),
                deleted_at,
            };
            (deleted_entry, deleted_history, deleted_result)
        };

        let persist_result = super::super::database::persist_deleted_entry(
            &deleted_entry,
            deleted_history.as_slice(),
        );
        if let Err(error) = persist_result {
            let mut sessions = self.sessions.write().await;
            let session = sessions
                .get_mut(&session_id)
                .with_context(|| format!("unknown session id {session_id}"))?;
            let (path, key) = entry_path_for(
                &deleted_entry.name,
                Some(&deleted_entry.shelf_name),
                Some(&deleted_entry.book_name),
            );
            let restored_entry = session.build_entry(
                path,
                deleted_entry.description.clone(),
                deleted_entry.labels.clone(),
                deleted_entry.context.clone(),
                deleted_entry.created_at.clone(),
                deleted_entry.updated_at.clone(),
                deleted_history,
            );
            session.insert_entry(key, restored_entry);
            return Err(error);
        }
        self.remove_append_lock(session_id, &key).await;

        checkpoint_journal(&self.journal);
        Ok(deleted_result)
    }
}
