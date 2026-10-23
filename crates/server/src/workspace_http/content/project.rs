// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

pub(super) fn shelf(value: domain::WorkspaceShelf) -> api::Shelf {
    api::Shelf {
        id: value.id.to_string(),
        workspace_id: value.workspace_id.to_string(),
        name: value.name,
        description: value.description,
        created_at: value.created_at,
    }
}

pub(super) fn book(value: domain::WorkspaceBook) -> api::Book {
    api::Book {
        id: value.id.to_string(),
        workspace_id: value.workspace_id.to_string(),
        shelf_id: value.shelf_id.to_string(),
        name: value.name,
        description: value.description,
        created_at: value.created_at,
    }
}

pub(super) fn summary(value: domain::EntrySummary) -> api::EntrySummary {
    api::EntrySummary {
        id: value.id.to_string(),
        workspace_id: value.workspace_id.to_string(),
        book_id: value.book_id.to_string(),
        name: value.name,
        owner_client_id: value.owner_client_id.map(|v| v.to_string()),
        revision: value.revision,
        archived: value.archived,
        created_at: value.created_at,
        updated_at: value.updated_at,
    }
}

pub(super) fn revision(value: domain::EntryRevision) -> api::EntryRevision {
    let actor = match value.actor {
        Actor::Admin(id) => api::ContentActor::Admin(id.to_string()),
        Actor::AdminKey(id) => api::ContentActor::AdminKey(id.to_string()),
        Actor::Credential(id) => api::ContentActor::Credential(id.to_string()),
    };
    api::EntryRevision {
        revision: value.revision,
        action: value.action,
        actor,
        client_id: value.client_id.map(|v| v.to_string()),
        admin_principal_id: value.admin_principal_id.map(|v| v.to_string()),
        agent: value.agent,
        recorded_at: value.recorded_at,
    }
}

pub(super) fn snapshot(value: domain::EntrySnapshot) -> api::EntrySnapshot {
    api::EntrySnapshot {
        entry: summary(value.entry),
        revision: revision(value.revision),
        content: api::EntryContent {
            text: value.content.text,
            description: value.content.description,
            labels: value.content.labels,
        },
        appended_text: value.appended_text,
    }
}
