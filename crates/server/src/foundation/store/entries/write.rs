// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

struct VersionWrite<'a> {
    head: EntrySummary,
    content: EntryContent,
    delta: Option<&'a str>,
}

pub(super) fn apply(
    tx: &Transaction<'_>,
    actor: Actor,
    workspace: Uuid,
    request: &EntryMutation,
    now: i64,
) -> anyhow::Result<EntryReceipt> {
    let write = match &request.change {
        EntryChange::Create {
            book_id,
            name,
            content,
        } => {
            content::book(tx, workspace, *book_id)?;
            let owner = match actor {
                Actor::Credential(id) => Some(authenticated_client(tx, id, now)?),
                _ => None,
            };
            let head = EntrySummary {
                id: Uuid::new_v4(),
                workspace_id: workspace,
                book_id: *book_id,
                name: validate_name(name)?.to_owned(),
                owner_client_id: owner,
                revision: 1,
                archived: false,
                created_at: now,
                updated_at: now,
            };
            tx.execute("INSERT INTO ccp_content_entries(id,workspace_id,book_id,name,owner_client_id,revision,archived,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,1,0,?6,?6)",params![head.id.to_string(),workspace.to_string(),book_id.to_string(),head.name,owner.map(|v|v.to_string()),now]).map_err(content_conflict)?;
            VersionWrite {
                head,
                content: content.clone(),
                delta: None,
            }
        }
        _ => change(tx, actor, workspace, request, now)?,
    };
    insert_version(tx, actor, request, &write, now)?;
    Ok(EntryReceipt {
        entry_id: write.head.id,
        revision: write.head.revision,
    })
}

fn change<'a>(
    tx: &Transaction<'_>,
    actor: Actor,
    workspace: Uuid,
    request: &'a EntryMutation,
    now: i64,
) -> anyhow::Result<VersionWrite<'a>> {
    let (id, expected) = match &request.change {
        EntryChange::Append {
            id,
            expected_revision,
            ..
        }
        | EntryChange::Revise {
            id,
            expected_revision,
            ..
        }
        | EntryChange::Archive {
            id,
            expected_revision,
        }
        | EntryChange::Restore {
            id,
            expected_revision,
        } => (*id, *expected_revision),
        EntryChange::Create { .. } => return Err(ContentError::InvalidInput.into()),
    };
    let mut head = head(tx, workspace, id)?;
    if !matches!(request.change, EntryChange::Append { .. }) {
        require_owner(tx, actor, &head, now)?;
    }
    if head.revision != expected
        || head.archived != matches!(request.change, EntryChange::Restore { .. })
    {
        return Err(ContentError::Conflict.into());
    }
    let mut content = read::snapshot(tx, &head, head.revision)?.content;
    let delta = match &request.change {
        EntryChange::Append { text, .. } => {
            if content.text.len().saturating_add(text.len()) > BODY_BYTES {
                return Err(ContentError::InvalidInput.into());
            }
            content.text.push_str(text);
            Some(text.as_str())
        }
        EntryChange::Revise {
            content: replacement,
            ..
        } => {
            content = replacement.clone();
            None
        }
        _ => None,
    };
    head.revision += 1;
    head.archived = matches!(request.change, EntryChange::Archive { .. });
    head.updated_at = now;
    let changed = tx.execute("UPDATE ccp_content_entries SET revision=?1,archived=?2,updated_at=?3 WHERE id=?4 AND workspace_id=?5 AND revision=?6",params![i64::try_from(head.revision)?,head.archived,now,id.to_string(),workspace.to_string(),i64::try_from(expected)?]).map_err(content_conflict)?;
    if changed != 1 {
        return Err(ContentError::Conflict.into());
    }
    Ok(VersionWrite {
        head,
        content,
        delta,
    })
}

fn require_owner(
    db: &Connection,
    actor: Actor,
    head: &EntrySummary,
    now: i64,
) -> anyhow::Result<()> {
    // WriteContext was already checked in this transaction. A credential never
    // becomes an admin through attribution; an admin key never becomes a client.
    if let Actor::Credential(id) = actor
        && Some(authenticated_client(db, id, now)?) != head.owner_client_id
    {
        return Err(ContentError::Forbidden.into());
    }
    Ok(())
}

fn insert_version(
    tx: &Transaction<'_>,
    actor: Actor,
    request: &EntryMutation,
    write: &VersionWrite<'_>,
    now: i64,
) -> anyhow::Result<()> {
    let (kind, id) = actor_parts(actor);
    let (client, admin) = match actor {
        Actor::Credential(id) => (Some(authenticated_client(tx, id, now)?), None),
        Actor::Admin(id) => (None, Some(id)),
        Actor::AdminKey(id) => {
            let principal: String = tx.query_row(
                "SELECT admin_id FROM ccp_admin_keys WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )?;
            (None, Some(parse_id(principal)?))
        }
    };
    tx.execute("INSERT INTO ccp_content_versions(entry_id,workspace_id,revision,action,actor_kind,actor_id,client_id,admin_principal_id,agent,recorded_at,content_json,appended_text) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![write.head.id.to_string(),write.head.workspace_id.to_string(),i64::try_from(write.head.revision)?,change_name(&request.change),kind,id.to_string(),client.map(|v|v.to_string()),admin.map(|v|v.to_string()),request.agent,now,serde_json::to_string(&write.content)?,write.delta])?;
    Ok(())
}
