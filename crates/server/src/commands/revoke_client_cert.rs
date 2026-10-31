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

        let _mutation = self
            .native_exclusive_mutation(session_id, auth_context)
            .await?;
        let mut grants = self.cert_grants.write().await;
        let grant = grants
            .get(target_client_common_name)
            .filter(|grant| grant.session_id == session_id)
            .cloned()
            .with_context(|| {
                format!("client certificate '{target_client_common_name}' not found")
            })?;
        let mut revoked = self.revoked_cert_common_names.write().await;
        let newly_revoked = revoked.insert(target_client_common_name.to_string());
        grants.remove(target_client_common_name);

        if let Err(error) = super::super::database::persist_revoked_cert(
            session_id,
            target_client_common_name,
            &grant,
            &revoked_at,
        ) {
            grants.insert(target_client_common_name.to_string(), grant);
            if newly_revoked {
                revoked.remove(target_client_common_name);
            }
            return Err(error);
        }

        Ok(RevokeCertResult {
            client_common_name: target_client_common_name.to_string(),
            revoked_at,
        })
    }
}
