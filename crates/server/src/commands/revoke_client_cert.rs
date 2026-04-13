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

        // Add to revocation set first (fail-closed: deny access before removing grant)
        self.revoked_cert_common_names
            .write()
            .await
            .insert(target_client_common_name.to_string());

        // Single write lock for the entire grant removal + validation
        let grant = {
            let mut grants = self.cert_grants.write().await;
            let removed = grants.remove(target_client_common_name);
            let Some(grant) = removed else {
                // Not found — undo revocation mark
                self.revoked_cert_common_names
                    .write()
                    .await
                    .remove(target_client_common_name);
                bail!("client certificate '{target_client_common_name}' not found");
            };
            if grant.session_id != session_id {
                grants.insert(target_client_common_name.to_string(), grant);
                self.revoked_cert_common_names
                    .write()
                    .await
                    .remove(target_client_common_name);
                bail!("client certificate '{target_client_common_name}' not found");
            }
            grant
        };

        if let Err(error) = super::super::database::persist_revoked_cert(
            session_id,
            target_client_common_name,
            &grant,
            &revoked_at,
        ) {
            // DB failed — undo both changes
            self.cert_grants
                .write()
                .await
                .insert(target_client_common_name.to_string(), grant);
            self.revoked_cert_common_names
                .write()
                .await
                .remove(target_client_common_name);
            return Err(error);
        }

        checkpoint_journal(&self.journal);
        Ok(RevokeCertResult {
            client_common_name: target_client_common_name.to_string(),
            revoked_at,
        })
    }
}
