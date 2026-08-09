// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::super::*;
use crate::init::agent_status_ttl_seconds;

const MAX_AGENT_NAME_BYTES: usize = 128;
const MAX_STATUS_BYTES: usize = 4 * 1024;

impl ServerState {
    pub async fn set_status(
        &self,
        session_id: i64,
        team: &str,
        agent_name: &str,
        status: &str,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<AgentStatus> {
        self.ensure_write_access(session_id, auth_context).await?;
        let (team, agent_name, status) = validated_status_input(team, agent_name, Some(status))?;
        // Keep a read guard until the SQLite commit so a concurrent shelf deletion
        // cannot pass the in-memory check, finish its cleanup, and then be followed
        // by an orphan status insert.
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(&session_id)
            .with_context(|| format!("unknown session id {session_id}"))?;
        if !session.shelves.contains_key(&team) {
            bail!("shelf '{team}' not found");
        }

        let now = current_unix_timestamp_millis()?;
        let ttl_millis = i64::try_from(agent_status_ttl_seconds())
            .context("agent status TTL exceeds supported range")?
            .checked_mul(1000)
            .context("agent status TTL exceeds supported range")?;
        let expires_at = now
            .checked_add(ttl_millis)
            .context("agent status expiration exceeds supported timestamp range")?;
        let mut connection = open_sqlite_connection()?;
        let transaction = connection.transaction()?;
        purge_expired(&transaction, session_id, &team, now)?;
        transaction.execute(
            "INSERT INTO agent_statuses (
                session_id, team, worker_id, agent_name, status, updated_at, expires_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(session_id, team, worker_id, agent_name) DO UPDATE SET
                status = excluded.status,
                updated_at = excluded.updated_at,
                expires_at = excluded.expires_at",
            params![
                session_id,
                team,
                auth_context.common_name,
                agent_name,
                status,
                now,
                expires_at
            ],
        )?;
        transaction.commit()?;
        drop(sessions);

        Ok(AgentStatus {
            team,
            agent_name,
            status,
            worker_id: auth_context.common_name.clone(),
            updated_at: now.to_string(),
            expires_at: expires_at.to_string(),
        })
    }

    pub async fn clear_status(
        &self,
        session_id: i64,
        team: &str,
        agent_name: &str,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<ClearStatusResult> {
        self.ensure_write_access(session_id, auth_context).await?;
        let (team, agent_name, _) = validated_status_input(team, agent_name, None)?;
        self.ensure_team_exists(session_id, &team).await?;

        let now = current_unix_timestamp_millis()?;
        let mut connection = open_sqlite_connection()?;
        let transaction = connection.transaction()?;
        purge_expired(&transaction, session_id, &team, now)?;
        let cleared = transaction.execute(
            "DELETE FROM agent_statuses
             WHERE session_id = ?1 AND team = ?2 AND worker_id = ?3 AND agent_name = ?4",
            params![session_id, team, auth_context.common_name, agent_name],
        )? > 0;
        transaction.commit()?;
        Ok(ClearStatusResult {
            team,
            agent_name,
            cleared,
        })
    }

    pub async fn list_team_status(
        &self,
        session_id: i64,
        team: &str,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<Vec<AgentStatus>> {
        self.team_statuses(session_id, team, None, auth_context)
            .await
    }

    pub async fn search_team_status(
        &self,
        session_id: i64,
        team: &str,
        query: &str,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<Vec<AgentStatus>> {
        let query = query.trim();
        if query.is_empty() {
            bail!("query is required for search_team_status");
        }
        self.team_statuses(session_id, team, Some(query), auth_context)
            .await
    }

    async fn team_statuses(
        &self,
        session_id: i64,
        team: &str,
        query: Option<&str>,
        auth_context: &ConnectionAuthContext,
    ) -> anyhow::Result<Vec<AgentStatus>> {
        self.ensure_read_access(session_id, auth_context).await?;
        let (team, _, _) = validated_status_input(team, "unused", None)?;
        self.ensure_team_exists(session_id, &team).await?;
        let now = current_unix_timestamp_millis()?;
        let connection = open_sqlite_connection()?;
        let mut statuses = Vec::new();

        if let Some(query) = query {
            let pattern = format!("%{}%", escape_like(query));
            let mut statement = connection.prepare(
                "SELECT team, agent_name, status, worker_id, updated_at, expires_at
                 FROM agent_statuses
                 WHERE session_id = ?1 AND team = ?2
                   AND expires_at > ?3
                   AND (agent_name LIKE ?4 ESCAPE '\\' COLLATE NOCASE
                        OR status LIKE ?4 ESCAPE '\\' COLLATE NOCASE)
                 ORDER BY updated_at DESC, agent_name COLLATE NOCASE, worker_id",
            )?;
            let rows =
                statement.query_map(params![session_id, team, now, pattern], status_from_row)?;
            for row in rows {
                statuses.push(row?);
            }
        } else {
            let mut statement = connection.prepare(
                "SELECT team, agent_name, status, worker_id, updated_at, expires_at
                 FROM agent_statuses
                 WHERE session_id = ?1 AND team = ?2 AND expires_at > ?3
                 ORDER BY updated_at DESC, agent_name COLLATE NOCASE, worker_id",
            )?;
            let rows = statement.query_map(params![session_id, team, now], status_from_row)?;
            for row in rows {
                statuses.push(row?);
            }
        }
        Ok(statuses)
    }

    async fn ensure_team_exists(&self, session_id: i64, team: &str) -> anyhow::Result<()> {
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(&session_id)
            .with_context(|| format!("unknown session id {session_id}"))?;
        if !session.shelves.contains_key(team) {
            bail!("shelf '{team}' not found");
        }
        Ok(())
    }
}

fn validated_status_input(
    team: &str,
    agent_name: &str,
    status: Option<&str>,
) -> anyhow::Result<(String, String, String)> {
    let team = team.trim();
    let agent_name = agent_name.trim();
    if team.is_empty() {
        bail!("team is required for agent status");
    }
    if agent_name.is_empty() {
        bail!("agent_name is required for agent status");
    }
    if agent_name.len() > MAX_AGENT_NAME_BYTES {
        bail!("agent_name must not exceed 128 bytes");
    }
    let status = match status {
        Some(status) => {
            let status = status.trim();
            if status.is_empty() {
                bail!("status is required for set_status");
            }
            if status.len() > MAX_STATUS_BYTES {
                bail!("status must not exceed 4096 bytes");
            }
            status
        }
        None => "",
    };
    Ok((team.to_string(), agent_name.to_string(), status.to_string()))
}

fn purge_expired(
    transaction: &Transaction<'_>,
    session_id: i64,
    team: &str,
    now: i64,
) -> anyhow::Result<()> {
    transaction.execute(
        "DELETE FROM agent_statuses
         WHERE session_id = ?1 AND team = ?2 AND expires_at <= ?3",
        params![session_id, team, now],
    )?;
    Ok(())
}

fn status_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentStatus> {
    Ok(AgentStatus {
        team: row.get(0)?,
        agent_name: row.get(1)?,
        status: row.get(2)?,
        worker_id: row.get(3)?,
        updated_at: row.get::<_, i64>(4)?.to_string(),
        expires_at: row.get::<_, i64>(5)?.to_string(),
    })
}

fn escape_like(query: &str) -> String {
    query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn current_unix_timestamp_millis() -> anyhow::Result<i64> {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before UNIX_EPOCH")?
            .as_millis(),
    )
    .context("system timestamp exceeds supported range")
}
