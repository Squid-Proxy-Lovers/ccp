// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::foundation::AccessStore;
use rusqlite::{Connection, OpenFlags};
use tokio::time::{MissedTickBehavior, interval};
use uuid::Uuid;

const WORKSPACES: usize = 16;
const ENTRIES: usize = 8;

struct Cycle {
    after: Option<Uuid>,
    failures: usize,
    deletion_after: Option<Uuid>,
}

pub(super) async fn run(
    state: Arc<State>,
    mut stop: watch::Receiver<bool>,
    budget: Arc<Semaphore>,
) -> anyhow::Result<()> {
    let mut tick = interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut after = None;
    let mut deletion_after = None;
    loop {
        tokio::select! {
            biased;
            _ = stop.changed() => return Ok(()),
            _ = tick.tick() => {},
        }
        let Ok(permit) = budget.clone().try_acquire_owned() else {
            continue;
        };
        let job_state = state.clone();
        let result = tokio::task::spawn_blocking(move || {
            // Captures survive cancellation of the async waiter. Keep the
            // permit and State/directory lease until all blocking SQL ends.
            let _permit = permit;
            cycle(&job_state, after, deletion_after)
        })
        .await;
        match result {
            Ok(Ok(cycle)) => {
                after = cycle.after;
                deletion_after = cycle.deletion_after;
                if cycle.failures > 0 {
                    eprintln!(
                        "workspace archive cleanup failed for {} targets; will retry later",
                        cycle.failures
                    );
                }
            }
            _ => eprintln!("workspace archive cleanup unavailable; will retry later"),
        }
    }
}

fn cycle(
    state: &State,
    after: Option<Uuid>,
    deletion_after: Option<Uuid>,
) -> anyhow::Result<Cycle> {
    let mut db = Connection::open_with_flags(&state.database, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.busy_timeout(Duration::from_secs(2))?;
    db.pragma_update(None, "synchronous", "FULL")?;
    let mut store = AccessStore::new(&mut db)?;
    let workspaces = store.timed_retention_workspaces(after, WORKSPACES)?;
    let next = if workspaces.len() == WORKSPACES {
        workspaces.last().copied()
    } else {
        None
    };
    let mut failures = 0;
    for workspace in workspaces {
        // Read the server clock for each protected transaction, not once for
        // an arbitrarily delayed queue of targets. Never echo stored content.
        let now = crate::instance_tls::now()?;
        if store
            .purge_due_entry_archives(workspace, ENTRIES, now)
            .is_err()
        {
            failures += 1;
        }
    }
    let deletions = store.pending_workspace_deletions(deletion_after, WORKSPACES)?;
    let deletion_next = if deletions.len() == WORKSPACES {
        deletions.last().copied()
    } else {
        None
    };
    for workspace in deletions {
        let now = crate::instance_tls::now()?;
        if store
            .advance_workspace_deletion(workspace, ENTRIES, now)
            .is_err()
        {
            failures += 1;
        }
    }
    Ok(Cycle {
        after: next,
        deletion_after: deletion_next,
        failures,
    })
}

#[cfg(test)]
mod tests;
