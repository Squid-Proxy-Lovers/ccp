// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use tokio::sync::{RwLockReadGuard, RwLockWriteGuard};

impl ServerState {
    pub(super) async fn native_mutation(
        &self,
        session: i64,
        actor: &ConnectionAuthContext,
    ) -> anyhow::Result<RwLockReadGuard<'_, ()>> {
        self.ensure_write_access(session, actor).await?;
        let guard = self.native_checkpoint.read().await;
        self.ensure_write_access(session, actor).await?;
        Ok(guard)
    }
    pub(super) async fn native_exclusive_mutation(
        &self,
        session: i64,
        actor: &ConnectionAuthContext,
    ) -> anyhow::Result<RwLockWriteGuard<'_, ()>> {
        self.ensure_write_access(session, actor).await?;
        let guard = self.native_checkpoint.write().await;
        self.ensure_write_access(session, actor).await?;
        Ok(guard)
    }
    pub(super) async fn checkpoint_before_mutation(
        &self,
        _guard: &RwLockWriteGuard<'_, ()>,
    ) -> anyhow::Result<()> {
        database::persist_snapshot(database::Snapshot::capture(self).await)?;
        let journal = Arc::clone(&self.journal);
        tokio::task::spawn_blocking(move || journal.checkpoint_blocking()).await??;
        Ok(())
    }
    pub async fn finish_native_shutdown(&self) -> anyhow::Result<()> {
        let _guard = self.native_checkpoint.write().await;
        self.mark_sessions_stopped_inner().await?;
        self.journal.shutdown()?;
        database::persist_snapshot(database::Snapshot::capture(self).await)?;
        self.journal.truncate_blocking()
    }
}
