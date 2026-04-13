// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::super::*;

impl ServerState {
    pub async fn get_entry_at(
        &self,
        session_id: i64,
        name: &str,
        shelf_name: Option<&str>,
        book_name: Option<&str>,
        at_timestamp: &str,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<Option<MessageEntry>> {
        self.ensure_read_access(session_id, auth_context).await?;

        let sessions = self.sessions.read().await;
        let Some(session) = sessions.get(&session_id) else {
            return Ok(None);
        };
        let (_path, key) = entry_path_for(name, shelf_name, book_name);
        let Some(entry) = session.entries.get(&key) else {
            return Ok(None);
        };

        // If the requested timestamp is before the entry was created, nothing existed yet.
        if entry.created_at.as_str() > at_timestamp {
            return Ok(None);
        }

        // Rebuild: initial context + appends where created_at <= at_timestamp
        let mut parts: Vec<String> = Vec::new();
        let mut history_at: Vec<MessageHistoryEntry> = Vec::new();

        // The initial content was set at entry creation time — count it if we got past
        // the created_at check above. We use the raw context minus any appended content.
        // Since appends join with '\n', the initial context is everything before the
        // first append. But we can't reliably split that. Instead, reconstruct from
        // history: the full context at time T is:
        //   initial_description_context + join(appends where created_at <= T)
        // The initial context doesn't have a history entry, so we need to figure out
        // what it was. The simplest approach: the context at creation = context minus
        // all appended content.
        let mut appended_total = String::new();
        for hist in &entry.history {
            if !appended_total.is_empty() {
                appended_total.push('\n');
            }
            appended_total.push_str(&hist.appended_content);
        }
        let initial = if entry.context.ends_with(&appended_total) && !appended_total.is_empty() {
            let prefix_len = entry.context.len() - appended_total.len();
            let prefix = &entry.context[..prefix_len];
            prefix.trim_end_matches('\n').to_string()
        } else if entry.history.is_empty() {
            entry.context.clone()
        } else {
            // can't reliably separate — return full context at creation
            String::new()
        };

        if !initial.is_empty() {
            parts.push(initial);
        }

        for hist in &entry.history {
            if hist.created_at.as_str() <= at_timestamp {
                parts.push(hist.appended_content.clone());
                history_at.push(hist.clone());
            }
        }

        let context = parts.join("\n");

        Ok(Some(MessageEntry {
            name: entry.summary.name.clone(),
            description: entry.summary.description.clone(),
            labels: entry.summary.labels.clone(),
            context,
            shelf_name: entry.summary.shelf_name.clone(),
            book_name: entry.summary.book_name.clone(),
            shelf_description: entry.summary.shelf_description.clone(),
            book_description: entry.summary.book_description.clone(),
        }))
    }
}
