// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::super::*;

impl ServerState {
    pub async fn list_entries(
        &self,
        session_id: i64,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<Vec<EntrySummary>> {
        self.ensure_read_access(session_id, auth_context).await?;
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(&session_id)
            .with_context(|| format!("unknown session id {session_id}"))?;
        Ok(session.list_entries_cache.clone())
    }
}
