// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::instance_tls::VerifiedPeer;

impl AccessStore<'_> {
    pub(crate) fn authenticate_peer(
        &mut self,
        peer: &VerifiedPeer,
        now: i64,
    ) -> anyhow::Result<Actor> {
        let transaction = self.connection.transaction()?;
        peer.require_registered(&transaction, now, false)?;
        transaction.commit()?;
        Ok(Actor::Credential(peer.credential()))
    }
}
