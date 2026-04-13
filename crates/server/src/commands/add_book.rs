// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::super::*;

impl ServerState {
    pub async fn add_book(
        &self,
        session_id: i64,
        shelf_name: &str,
        book_name: &str,
        description: &str,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<AddBookResult> {
        self.ensure_write_access(session_id, auth_context).await?;
        let shelf_name = normalize_segment(Some(shelf_name), "");
        let book_name = normalize_segment(Some(book_name), "");
        if shelf_name.is_empty() {
            bail!("shelf name is required for add_book");
        }
        if book_name.is_empty() {
            bail!("book name is required for add_book");
        }
        validate_segment(&shelf_name)?;
        validate_segment(&book_name)?;
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .with_context(|| format!("unknown session id {session_id}"))?;
        if !session.shelves.contains_key(&shelf_name) {
            bail!("shelf '{shelf_name}' does not exist");
        }

        self.journal.append(JournalEntry::AddBook {
            session_id,
            shelf_name: shelf_name.clone(),
            book_name: book_name.clone(),
            description: normalize_optional_text(Some(description)).unwrap_or_default(),
        })?;
        let result = session.add_book(&shelf_name, &book_name, Some(description))?;
        session.refresh_library_metadata(&shelf_name, Some(&book_name));
        Ok(result)
    }
}
