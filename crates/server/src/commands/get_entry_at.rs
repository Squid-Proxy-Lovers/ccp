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

        let at_timestamp = at_timestamp
            .parse::<u64>()
            .context("invalid at_timestamp: must be Unix seconds")?;
        let sessions = self.sessions.read().await;
        let Some(session) = sessions.get(&session_id) else {
            return Ok(None);
        };
        let (_path, key) = entry_path_for(name, shelf_name, book_name);
        let Some(entry) = session.entries.get(&key) else {
            return Ok(None);
        };

        let created_at = entry
            .created_at
            .parse::<u64>()
            .context("invalid entry creation timestamp: must be Unix seconds")?;
        if created_at > at_timestamp {
            return Ok(None);
        }
        let context = context_at(&entry.context, &entry.history, at_timestamp)?;

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

// Undo actual append separators instead of trimming initial text. Imported
// history-only merges may not describe the content; reject that ambiguity.
fn context_at(
    context: &str,
    history: &[MessageHistoryEntry],
    at_timestamp: u64,
) -> anyhow::Result<String> {
    let mut initial = context;
    for row in history.iter().rev() {
        if initial == row.appended_content {
            initial = "";
        } else if let Some(prefix) = initial.strip_suffix(&format!("\n{}", row.appended_content)) {
            initial = prefix;
        } else {
            bail!("invalid history: content cannot be reconstructed at the requested timestamp");
        }
    }
    let mut result = initial.to_string();
    for row in history {
        let timestamp = row
            .created_at
            .parse::<u64>()
            .context("invalid history timestamp: must be Unix seconds")?;
        if timestamp <= at_timestamp {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&row.appended_content);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(content: &str, timestamp: &str) -> MessageHistoryEntry {
        MessageHistoryEntry {
            operation_id: "operation".to_string(),
            client_common_name: "worker".to_string(),
            agent_name: None,
            host_name: None,
            reason: None,
            appended_content: content.to_string(),
            created_at: timestamp.to_string(),
        }
    }

    #[test]
    fn reconstruction_preserves_initial_and_appended_newlines() {
        let history = vec![row("first\n", "9"), row("second", "10")];
        let context = "initial\n\n\nfirst\n\nsecond";
        assert_eq!(context_at(context, &history, 8).unwrap(), "initial\n\n");
        assert_eq!(
            context_at(context, &history, 9).unwrap(),
            "initial\n\n\nfirst\n"
        );
        assert_eq!(context_at(context, &history, 10).unwrap(), context);
    }

    #[test]
    fn empty_appends_follow_the_actual_append_separator_rule() {
        let history = vec![row("", "8"), row("first", "9"), row("", "10")];
        assert_eq!(context_at("first\n", &history, 8).unwrap(), "");
        assert_eq!(context_at("first\n", &history, 9).unwrap(), "first");
        assert_eq!(context_at("first\n", &history, 10).unwrap(), "first\n");
    }

    #[test]
    fn unrelated_imported_history_cannot_fabricate_a_snapshot() {
        assert!(context_at("current", &[row("unrelated", "9")], 10).is_err());
        assert!(context_at("first", &[row("first", "yesterday")], 10).is_err());
    }
}
