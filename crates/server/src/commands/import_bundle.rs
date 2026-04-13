// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::HashSet;

use sha2::{Digest, Sha256};

use protocol::{ConflictPolicy, ImportBundleResult, TransferBundle};

use super::super::*;

impl ServerState {
    pub async fn import_bundle(
        &self,
        session_id: i64,
        bundle: &TransferBundle,
        policy: &ConflictPolicy,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<ImportBundleResult> {
        self.ensure_write_access(session_id, auth_context).await?;

        // Verify bundle integrity.
        let entries_bytes = serde_json::to_vec(&bundle.entries)
            .context("failed to serialise entries for hash verification")?;
        let computed_sha256: String = Sha256::digest(&entries_bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if computed_sha256 != bundle.bundle_sha256 {
            bail!("bundle integrity check failed: hash mismatch");
        }

        let scope_json =
            serde_json::to_string(&bundle.selector.scope).context("failed to serialise scope")?;
        let policy_str = policy_to_str(policy);

        // Validate all names up-front before taking the write lock.
        for entry in &bundle.entries {
            validate_segment(&entry.name)?;
            validate_segment(&entry.shelf_name)?;
            validate_segment(&entry.book_name)?;
        }

        // For Error policy: check all collisions before any mutation.
        if matches!(policy, ConflictPolicy::Error) {
            let sessions = self.sessions.read().await;
            let session = sessions
                .get(&session_id)
                .with_context(|| format!("session id {session_id} not found; create it first"))?;
            for entry in &bundle.entries {
                let (_, key) =
                    entry_path_for(&entry.name, Some(&entry.shelf_name), Some(&entry.book_name));
                if session.entries.contains_key(&key) {
                    bail!(
                        "entry '{}' already exists in shelf '{}' / book '{}' (policy: error)",
                        entry.name,
                        entry.shelf_name,
                        entry.book_name,
                    );
                }
            }
        }

        // Snapshots for rollback.
        let shelves_snapshot;
        let books_snapshot;
        let book_counts_snapshot;
        let shelf_book_counts_snapshot;
        let shelf_entry_counts_snapshot;
        let mut previous_entries: HashMap<String, Option<CachedMessagePack>> = HashMap::new();

        let mut imported_entries = 0usize;
        let mut overwritten_entries = 0usize;
        let mut skipped_entries = 0usize;

        {
            let mut sessions = self.sessions.write().await;
            let session = sessions
                .get_mut(&session_id)
                .with_context(|| format!("session id {session_id} not found; create it first"))?;

            shelves_snapshot = session.shelves.clone();
            books_snapshot = session.books.clone();
            book_counts_snapshot = session.book_entry_counts.clone();
            shelf_book_counts_snapshot = session.shelf_book_counts.clone();
            shelf_entry_counts_snapshot = session.shelf_entry_counts.clone();

            for entry in &bundle.entries {
                let (path, key) =
                    entry_path_for(&entry.name, Some(&entry.shelf_name), Some(&entry.book_name));

                let created_at = entry
                    .history
                    .first()
                    .map(|h| h.created_at.clone())
                    .unwrap_or_else(|| {
                        current_timestamp_string().unwrap_or_else(|_| "0".to_string())
                    });
                let updated_at = entry
                    .history
                    .last()
                    .map(|h| h.created_at.clone())
                    .unwrap_or_else(|| {
                        current_timestamp_string().unwrap_or_else(|_| "0".to_string())
                    });

                match policy {
                    ConflictPolicy::Error => {
                        // Pre-checked above; just insert.
                        session.upsert_library_metadata(
                            path.shelf_name(),
                            path.book_name(),
                            Some(&entry.shelf_description),
                            Some(&entry.book_description),
                        );
                        session.refresh_library_metadata(path.shelf_name(), Some(path.book_name()));
                        previous_entries.insert(key.clone(), None);
                        let imported = session.build_entry(
                            path,
                            entry.description.clone(),
                            entry.labels.clone(),
                            entry.context.clone(),
                            created_at,
                            updated_at,
                            entry.history.clone(),
                        );
                        session.insert_entry(key, imported);
                        imported_entries += 1;
                    }

                    ConflictPolicy::Overwrite => {
                        session.upsert_library_metadata(
                            path.shelf_name(),
                            path.book_name(),
                            Some(&entry.shelf_description),
                            Some(&entry.book_description),
                        );
                        session.refresh_library_metadata(path.shelf_name(), Some(path.book_name()));
                        let prior = session.entries.get(&key).cloned();
                        if prior.is_some() {
                            overwritten_entries += 1;
                        } else {
                            imported_entries += 1;
                        }
                        previous_entries.insert(key.clone(), prior);
                        let imported = session.build_entry(
                            path,
                            entry.description.clone(),
                            entry.labels.clone(),
                            entry.context.clone(),
                            created_at,
                            updated_at,
                            entry.history.clone(),
                        );
                        session.insert_entry(key, imported);
                    }

                    ConflictPolicy::Skip => {
                        if session.entries.contains_key(&key) {
                            skipped_entries += 1;
                            continue;
                        }
                        session.upsert_library_metadata(
                            path.shelf_name(),
                            path.book_name(),
                            Some(&entry.shelf_description),
                            Some(&entry.book_description),
                        );
                        session.refresh_library_metadata(path.shelf_name(), Some(path.book_name()));
                        previous_entries.insert(key.clone(), None);
                        let imported = session.build_entry(
                            path,
                            entry.description.clone(),
                            entry.labels.clone(),
                            entry.context.clone(),
                            created_at,
                            updated_at,
                            entry.history.clone(),
                        );
                        session.insert_entry(key, imported);
                        imported_entries += 1;
                    }

                    ConflictPolicy::MergeHistory => {
                        if let Some(existing) = session.entries.get_mut(&key) {
                            // Keep existing context/description/labels; union history rows.
                            previous_entries.insert(key.clone(), Some(existing.clone()));
                            let known_op_ids: HashSet<_> = existing
                                .history
                                .iter()
                                .map(|h| h.operation_id.clone())
                                .collect();
                            let new_rows: Vec<_> = entry
                                .history
                                .iter()
                                .filter(|h| !known_op_ids.contains(&h.operation_id))
                                .cloned()
                                .collect();
                            if !new_rows.is_empty() {
                                existing.history.extend(new_rows);
                                existing
                                    .history
                                    .sort_by(|a, b| a.created_at.cmp(&b.created_at));
                            }
                            // History-only change; invalidate search caches explicitly.
                            session.invalidate_entry_search_results();
                            session.invalidate_context_search_results();
                        } else {
                            session.upsert_library_metadata(
                                path.shelf_name(),
                                path.book_name(),
                                Some(&entry.shelf_description),
                                Some(&entry.book_description),
                            );
                            session.refresh_library_metadata(
                                path.shelf_name(),
                                Some(path.book_name()),
                            );
                            previous_entries.insert(key.clone(), None);
                            let imported = session.build_entry(
                                path,
                                entry.description.clone(),
                                entry.labels.clone(),
                                entry.context.clone(),
                                created_at,
                                updated_at,
                                entry.history.clone(),
                            );
                            session.insert_entry(key, imported);
                            imported_entries += 1;
                        }
                    }
                }
            }

            // Flush query caches for all policies (covers the latent v1 bug too).
            session.invalidate_entry_search_results();
            session.invalidate_context_search_results();
        }

        let created_at = current_timestamp_string()?;
        let total_affected = imported_entries + overwritten_entries;

        if let Err(error) = super::super::database::persist_snapshot(
            super::super::database::Snapshot::capture(self).await,
        ) {
            // Revert in-memory state.
            let mut sessions = self.sessions.write().await;
            if let Some(session) = sessions.get_mut(&session_id) {
                for (key, maybe_previous) in previous_entries {
                    match maybe_previous {
                        Some(previous) => {
                            session.insert_entry(key, previous);
                        }
                        None => {
                            session.remove_entry(&key);
                        }
                    }
                }
                let affected_shelves: Vec<String> = session.shelves.keys().cloned().collect();
                session.shelves = shelves_snapshot;
                session.books = books_snapshot;
                session.book_entry_counts = book_counts_snapshot;
                session.shelf_book_counts = shelf_book_counts_snapshot;
                session.shelf_entry_counts = shelf_entry_counts_snapshot;
                for shelf in &affected_shelves {
                    session.refresh_library_metadata(shelf, None);
                }
                session.rebuild_list_entries_cache();
            }

            let reason = error.to_string();
            let _ = self.journal.append(JournalEntry::TransferImportFailed {
                session_id,
                bundle_sha256: bundle.bundle_sha256.clone(),
                reason: reason.clone(),
            });
            let _ = super::super::database::persist_transfer_log(
                session_id,
                "import",
                &scope_json,
                &bundle.bundle_sha256,
                Some(policy_str),
                &format!("rolled_back:{reason}"),
                total_affected,
                &created_at,
                &auth_context.common_name,
            );
            return Err(error);
        }

        self.journal.append(JournalEntry::TransferImported {
            session_id,
            scope_json: scope_json.clone(),
            policy: policy_str.to_string(),
            bundle_sha256: bundle.bundle_sha256.clone(),
            entry_count: total_affected,
        })?;

        super::super::database::persist_transfer_log(
            session_id,
            "import",
            &scope_json,
            &bundle.bundle_sha256,
            Some(policy_str),
            "ok",
            total_affected,
            &created_at,
            &auth_context.common_name,
        )?;

        checkpoint_journal(&self.journal);

        Ok(ImportBundleResult {
            imported_entries,
            overwritten_entries,
            skipped_entries,
        })
    }
}

fn policy_to_str(policy: &ConflictPolicy) -> &'static str {
    match policy {
        ConflictPolicy::Error => "error",
        ConflictPolicy::Overwrite => "overwrite",
        ConflictPolicy::Skip => "skip",
        ConflictPolicy::MergeHistory => "merge-history",
    }
}
