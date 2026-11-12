// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

/// Explicit host-operator server-leaf renewal. The pinned CA and instance stay
/// identical; complete immutable generation publication precedes the pin commit.
/// No listener is stopped, no CA is rotated and no trust store is installed.
pub fn renew(directory: &Path, names: Option<&[String]>) -> anyhow::Result<TrustInfo> {
    let names = names.map(bundle::normalize_names).transpose()?;
    require_database(directory)?;
    let lease = DirectoryLease::maintenance(directory)?;
    let mut connection = database(&lease, true)?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let instance = initialized_instance(&tx)?;
    let pin = anchor(&tx)?.context("TLS identity is not pinned; run workspace-tls-init first")?;
    let bytes = files::read_pinned(lease.directory(), &pin.hash)?;
    let current = decode_bundle(&bytes)?;
    check_pin(
        &pin,
        instance,
        &Sha256::digest(&bytes),
        &Sha256::digest(current.ca_der()?.as_ref()),
    )?;
    let timestamp = now()?;
    let next = current.renew_server(
        instance,
        names.unwrap_or_else(|| current.names.clone()),
        timestamp,
    )?;
    let trust = next.validate(instance, timestamp)?;
    let bytes = serde_json::to_vec(&next)?;
    let hash = Sha256::digest(&bytes).to_vec();
    files::publish_generation(lease.directory(), &bytes)?;
    let changed = tx.execute("UPDATE ccp_instance_tls SET bundle_hash = ?1 WHERE singleton = 1 AND instance_id = ?2 AND bundle_hash = ?3 AND ca_fingerprint = ?4", params![hash, instance.to_string(), pin.hash, pin.ca_hash])?;
    if changed != 1 {
        bail!("TLS identity changed during renewal");
    }
    tx.execute("INSERT INTO ccp_access_audit(actor_kind, actor_id, action, target_id, recorded_at) VALUES ('local_operator', 'local', 'renew_instance_tls', ?1, ?2)", params![instance.to_string(), timestamp])?;
    tx.commit().context(
        "failed to commit server certificate renewal; inspect workspace-tls-info before retrying",
    )?;
    Ok(trust)
}

#[cfg(test)]
mod tests;
