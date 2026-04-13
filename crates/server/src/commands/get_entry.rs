// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::super::*;

impl ServerState {
    pub async fn get_entry(
        &self,
        session_id: i64,
        name: &str,
        shelf_name: Option<&str>,
        book_name: Option<&str>,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<Option<MessageEntry>> {
        self.ensure_read_access(session_id, auth_context).await?;
        let sessions = self.sessions.read().await;
        let Some(session) = sessions.get(&session_id) else {
            return Ok(None);
        };
        let (_path, key) = entry_path_for(name, shelf_name, book_name);
        Ok(session.entries.get(&key).map(message_entry_from))
    }
}
