// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::super::*;

impl ServerState {
    pub async fn add_shelf(
        &self,
        session_id: i64,
        shelf_name: &str,
        description: &str,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<AddShelfResult> {
        self.ensure_write_access(session_id, auth_context).await?;
        let shelf_name = normalize_segment(Some(shelf_name), "");
        if shelf_name.is_empty() {
            bail!("shelf name is required for add_shelf");
        }
        validate_segment(&shelf_name)?;
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .with_context(|| format!("unknown session id {session_id}"))?;

        self.journal.append(JournalEntry::AddShelf {
            session_id,
            shelf_name: shelf_name.clone(),
            description: normalize_optional_text(Some(description)).unwrap_or_default(),
        })?;
        let result = session.add_shelf(&shelf_name, Some(description));
        session.refresh_library_metadata(&shelf_name, None);
        Ok(result)
    }
}
