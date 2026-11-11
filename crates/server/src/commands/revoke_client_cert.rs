// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::super::*;

impl ServerState {
    pub async fn revoke_client_cert(
        &self,
        session_id: i64,
        auth_context: &ConnectionAuthContext,
        target_client_common_name: &str,
    ) -> anyhow::Result<RevokeCertResult> {
        self.ensure_write_access(session_id, auth_context).await?;
        let revoking_self = auth_context.common_name == target_client_common_name;
        if !revoking_self && !auth_context.can_revoke_others {
            bail!("only admin tokens can revoke other client certificates");
        }
        let revoked_at = current_timestamp_string()?;

        // Hold the grant lock across the check, the revocation mark and the
        // persist so a retried or concurrent revoke of the same CN cannot
        // undo another call's revocation.
        {
            let mut grants = self.cert_grants.write().await;
            let grant = grants
                .get(target_client_common_name)
                .filter(|grant| grant.session_id == session_id)
                .cloned()
                .with_context(|| {
                    format!("client certificate '{target_client_common_name}' not found")
                })?;

            // Mark revoked before removing the grant (fail-closed: deny access
            // before removing the grant that authorizes it). Release the
            // revoked-set lock before the blocking persist below — every
            // ping, read and write takes a brief read lock on this same set,
            // and the cert_grants lock above already serializes concurrent
            // revokes, so holding it across the persist isn't needed for
            // correctness and only stalls unrelated requests behind SQLite.
            let newly_revoked = self
                .revoked_cert_common_names
                .write()
                .await
                .insert(target_client_common_name.to_string());
            grants.remove(target_client_common_name);
            #[cfg(test)]
            self.test_event("revoke_marked");

            if let Err(error) = super::super::database::persist_revoked_cert(
                session_id,
                target_client_common_name,
                &grant,
                &revoked_at,
            ) {
                // DB failed — undo both changes, but only remove the
                // revocation mark if this call is the one that set it.
                grants.insert(target_client_common_name.to_string(), grant);
                if newly_revoked {
                    self.revoked_cert_common_names
                        .write()
                        .await
                        .remove(target_client_common_name);
                }
                return Err(error);
            }
        }

        // Release both grant and revoked-set locks before waiting for the
        // exclusive checkpoint boundary. No journal producer may complete
        // between snapshot capture and journal retirement.
        {
            let _snapshot_guard = self.native_checkpoint.write().await;
            checkpoint_journal(self).await;
        }
        Ok(RevokeCertResult {
            client_common_name: target_client_common_name.to_string(),
            revoked_at,
        })
    }
}
