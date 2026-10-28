// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::management::require_reader;
use super::*;
use crate::foundation::{
    AccessCounts, ContentTotals, DailyActivity, OVERVIEW_MAX_DAYS, WorkspaceOverview,
    WriterActivity,
};

const DAY: i64 = 86_400;
const EXPIRY_HORIZON: i64 = 7 * DAY;
const TOP_WRITERS: i64 = 8;

impl AccessStore<'_> {
    /// Aggregate metrics under the same administrative read authority as
    /// workspace inspection, from one read snapshot. Every query is scoped to
    /// the workspace and returns a bounded number of grouped rows.
    pub fn workspace_overview(
        &mut self,
        actor: Actor,
        workspace: Uuid,
        days: u32,
        now: i64,
    ) -> anyhow::Result<WorkspaceOverview> {
        if !(1..=OVERVIEW_MAX_DAYS).contains(&days) {
            bail!("overview window must be 1..{OVERVIEW_MAX_DAYS} days");
        }
        let tx = self.connection.transaction()?;
        require_reader(&tx, actor, workspace, now)?;
        let window = Window::ending_today(days, now)?;
        let overview = WorkspaceOverview {
            workspace_id: workspace,
            generated_at: now,
            days,
            totals: totals(&tx, workspace)?,
            activity: activity(&tx, workspace, &window)?,
            writers: writers(&tx, workspace, &window)?,
            access: access(&tx, workspace, now)?,
        };
        tx.commit()?;
        Ok(overview)
    }
}

/// Whole UTC days `[start, end)` ending with the day containing `now`.
struct Window {
    start: i64,
    end: i64,
    days: usize,
}

impl Window {
    fn ending_today(days: u32, now: i64) -> anyhow::Result<Self> {
        let end = now
            .checked_sub(now.rem_euclid(DAY))
            .and_then(|today| today.checked_add(DAY))
            .context("overview window end overflows")?;
        let start = i64::from(days)
            .checked_mul(DAY)
            .and_then(|span| end.checked_sub(span))
            .context("overview window start overflows")?;
        Ok(Self {
            start,
            end,
            days: usize::try_from(days)?,
        })
    }
}

fn totals(tx: &Connection, workspace: Uuid) -> anyhow::Result<ContentTotals> {
    Ok(tx.query_row(
        "SELECT
         (SELECT COUNT(*) FROM ccp_content_shelves WHERE workspace_id=?1 AND archived=0),
         (SELECT COUNT(*) FROM ccp_content_books WHERE workspace_id=?1),
         (SELECT COUNT(*) FROM ccp_content_entries WHERE workspace_id=?1 AND archived=0),
         (SELECT COUNT(*) FROM ccp_content_entries WHERE workspace_id=?1 AND archived=1)",
        [workspace.to_string()],
        |r| {
            Ok(ContentTotals {
                shelves: r.get(0)?,
                books: r.get(1)?,
                entries: r.get(2)?,
                archived_entries: r.get(3)?,
            })
        },
    )?)
}

fn activity(
    tx: &Connection,
    workspace: Uuid,
    window: &Window,
) -> anyhow::Result<Vec<DailyActivity>> {
    let mut days = (0..window.days)
        .map(|offset| {
            let day = i64::try_from(offset)?
                .checked_mul(DAY)
                .and_then(|elapsed| window.start.checked_add(elapsed))
                .context("overview day overflows")?;
            Ok(DailyActivity {
                day,
                ..DailyActivity::default()
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    // At most days x five actions grouped rows are returned.
    let rows = tx
        .prepare(
            "SELECT (recorded_at-?2)/86400, action, COUNT(*) FROM ccp_content_versions
             WHERE workspace_id=?1 AND recorded_at>=?2 AND recorded_at<?3
             GROUP BY 1, action",
        )?
        .query_map(
            params![workspace.to_string(), window.start, window.end],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u64>(2)?,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (offset, action, count) in rows {
        let day = days
            .get_mut(usize::try_from(offset)?)
            .context("content version outside the overview window")?;
        let slot = match action.as_str() {
            "create" => &mut day.create,
            "append" => &mut day.append,
            "revise" => &mut day.revise,
            "archive" => &mut day.archive,
            "restore" => &mut day.restore,
            _ => bail!("invalid persisted content action"),
        };
        *slot = count;
    }
    Ok(days)
}

fn writers(
    tx: &Connection,
    workspace: Uuid,
    window: &Window,
) -> anyhow::Result<Vec<WriterActivity>> {
    // Administrative versions have no client and form one group.
    let rows = tx
        .prepare(
            "SELECT v.client_id, c.name, COUNT(*) AS writes, MAX(v.recorded_at) AS last
             FROM ccp_content_versions v LEFT JOIN ccp_clients c ON c.id=v.client_id
             WHERE v.workspace_id=?1 AND v.recorded_at>=?2 AND v.recorded_at<?3
             GROUP BY v.client_id, c.name
             ORDER BY writes DESC, last DESC, v.client_id LIMIT ?4",
        )?
        .query_map(
            params![workspace.to_string(), window.start, window.end, TOP_WRITERS],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, u64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(client, name, writes, last_write_at)| {
            Ok(WriterActivity {
                client_id: client.map(parse_id).transpose()?,
                client_name: name,
                writes,
                last_write_at,
            })
        })
        .collect()
}

fn access(tx: &Connection, workspace: Uuid, now: i64) -> anyhow::Result<AccessCounts> {
    let horizon = now
        .checked_add(EXPIRY_HORIZON)
        .context("credential expiry horizon overflows")?;
    // Active clients and live credentials follow the grant listing and data
    // authorization predicates; pending invitations follow redemption closure.
    Ok(tx.query_row(
        "SELECT
         (SELECT COUNT(*) FROM ccp_grants g JOIN ccp_clients c ON c.id=g.client_id
          WHERE g.workspace_id=?1 AND g.revoked_at IS NULL AND c.disabled=0),
         (SELECT COUNT(DISTINCT k.id) FROM ccp_credential_grants b
          JOIN ccp_grants g ON g.id=b.grant_id AND g.client_id=b.client_id
          JOIN ccp_credentials k ON k.id=b.credential_id AND k.client_id=b.client_id
          JOIN ccp_clients c ON c.id=k.client_id
          WHERE g.workspace_id=?1 AND g.revoked_at IS NULL AND c.disabled=0
          AND k.revoked_at IS NULL AND k.expires_at>?2 AND k.expires_at<=?3
          AND NOT EXISTS(SELECT 1 FROM ccp_credential_renewals r WHERE r.previous_credential_id=k.id)
          AND NOT EXISTS(SELECT 1 FROM ccp_binding_revocations v
              WHERE v.credential_id=b.credential_id AND v.grant_id=b.grant_id)),
         (SELECT COUNT(*) FROM ccp_invitations i
          WHERE i.workspace_id=?1 AND i.revoked_at IS NULL
          AND (i.max_uses IS NULL OR i.uses<i.max_uses)
          AND (i.expires_at IS NULL OR i.expires_at>?2)
          AND NOT EXISTS(SELECT 1 FROM ccp_invitation_closures e WHERE e.invitation_id=i.id)
          AND (i.kind='individual' OR EXISTS(SELECT 1 FROM ccp_initial_enrollment w
              WHERE w.workspace_id=i.workspace_id AND w.invitation_id=i.id AND w.closed_at IS NULL)))",
        params![workspace.to_string(), now, horizon],
        |r| {
            Ok(AccessCounts {
                active_clients: r.get(0)?,
                credentials_expiring_within_7d: r.get(1)?,
                pending_invitations: r.get(2)?,
            })
        },
    )?)
}
