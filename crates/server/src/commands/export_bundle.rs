// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use sha2::{Digest, Sha256};

use protocol::{TransferBundle, TransferScope, TransferSelector};

use super::super::*;

impl ServerState {
    pub async fn export_bundle(
        &self,
        session_id: i64,
        selector: &TransferSelector,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<TransferBundle> {
        self.ensure_read_access(session_id, auth_context).await?;
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(&session_id)
            .with_context(|| format!("unknown session id {session_id}"))?;

        let mut entries = session
            .entries
            .values()
            .filter(|entry| scope_matches(&selector.scope, entry))
            .map(|entry| BundleEntry {
                name: entry.name().to_string(),
                description: entry.description.clone(),
                labels: entry.labels.clone(),
                context: entry.context.clone(),
                shelf_name: entry.path.shelf_name.clone(),
                book_name: entry.path.book_name.clone(),
                shelf_description: session.shelf_description(entry.path.shelf_name()),
                book_description: session
                    .book_description(entry.path.shelf_name(), entry.path.book_name()),
                history: if selector.include_history {
                    entry.history.clone()
                } else {
                    Vec::new()
                },
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| {
            (&a.shelf_name, &a.book_name, &a.name).cmp(&(&b.shelf_name, &b.book_name, &b.name))
        });

        let entry_count = entries.len();
        let entries_bytes =
            serde_json::to_vec(&entries).context("failed to serialise entries for hash")?;
        let bundle_sha256: String = Sha256::digest(&entries_bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();

        let exported_at = current_timestamp_string()?;
        let scope_json =
            serde_json::to_string(&selector.scope).context("failed to serialise scope")?;

        let bundle = TransferBundle {
            session: session.metadata.clone(),
            selector: selector.clone(),
            exported_at,
            entries,
            bundle_sha256: bundle_sha256.clone(),
        };

        drop(sessions);

        self.journal.append(JournalEntry::TransferExported {
            session_id,
            scope_json: scope_json.clone(),
            bundle_sha256: bundle_sha256.clone(),
            entry_count,
        })?;

        super::super::database::persist_transfer_log(
            session_id,
            "export",
            &scope_json,
            &bundle_sha256,
            None,
            "ok",
            entry_count,
            &bundle.exported_at,
            &auth_context.common_name,
        )?;

        Ok(bundle)
    }
}

fn scope_matches(scope: &TransferScope, entry: &CachedMessagePack) -> bool {
    match scope {
        TransferScope::Session => true,
        TransferScope::Shelf { shelf } => entry.path.shelf_name() == shelf,
        TransferScope::Book { shelf, book } => {
            entry.path.shelf_name() == shelf && entry.path.book_name() == book
        }
        TransferScope::Entries {
            shelf,
            book,
            entries,
        } => {
            entry.path.shelf_name() == shelf
                && entry.path.book_name() == book
                && entries.iter().any(|e| e == entry.path.chapter_name())
        }
    }
}
