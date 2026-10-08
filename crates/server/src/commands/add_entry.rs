// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use protocol::DuplicateWarning;

use super::super::*;

impl ServerState {
    pub async fn add_entry(
        &self,
        session_id: i64,
        name: &str,
        description: &str,
        labels: &[String],
        context: &str,
        shelf_name: &str,
        book_name: &str,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<(MessageEntry, Option<DuplicateWarning>)> {
        self.ensure_write_access(session_id, auth_context).await?;
        if name.trim().is_empty() {
            bail!("name is required for add_entry");
        }
        validate_segment(name)?;

        let (path, key) = entry_path_for(name, Some(shelf_name), Some(book_name));

        let created_at = current_timestamp_string()?;
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .with_context(|| format!("unknown session id {session_id}"))?;
        if !session.shelves.contains_key(path.shelf_name()) {
            bail!("shelf '{}' does not exist", path.shelf_name());
        }
        if !session
            .books
            .contains_key(&(path.shelf_name().to_string(), path.book_name().to_string()))
        {
            bail!(
                "book '{}/{}' does not exist",
                path.shelf_name(),
                path.book_name()
            );
        }
        if session.entries.contains_key(&key) {
            bail!("entry '{name}' already exists");
        }

        // Duplicate detection: find the most similar existing entry name.
        let mut best_warning: Option<(f64, DuplicateWarning)> = None;
        for existing in session.entries.values() {
            let score = normalized_levenshtein(name, existing.name());
            if score > 0.8 {
                let dominated = best_warning
                    .as_ref()
                    .is_none_or(|(best_score, _)| score > *best_score);
                if dominated {
                    best_warning = Some((
                        score,
                        DuplicateWarning {
                            existing_name: existing.name().to_string(),
                            existing_shelf: existing.path.shelf_name().to_string(),
                            existing_book: existing.path.book_name().to_string(),
                            similarity: format!("{:.2}", score),
                        },
                    ));
                }
            }
        }
        let duplicate_warning = best_warning.map(|(_, warning)| warning);

        self.journal.append(JournalEntry::AddEntry {
            session_id,
            name: name.to_string(),
            description: description.to_string(),
            labels: labels.to_vec(),
            context: context.to_string(),
            shelf_name: path.shelf_name.clone(),
            book_name: path.book_name.clone(),
            created_at: created_at.clone(),
            updated_at: created_at.clone(),
        })?;
        let entry = session.build_entry(
            path.clone(),
            description.to_string(),
            labels.to_vec(),
            context.to_string(),
            created_at.clone(),
            created_at.clone(),
            Vec::new(),
        );
        session.insert_entry(key.clone(), entry);

        let entry = session
            .entries
            .get(&key)
            .with_context(|| format!("entry '{name}' missing after insert"))?;
        Ok((message_entry_from(entry), duplicate_warning))
    }
}
