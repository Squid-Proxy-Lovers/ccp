// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use uuid::Uuid;

pub struct NodeDraft {
    pub name: String,
    pub description: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct WorkspaceShelf {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub description: String,
    pub created_at: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct WorkspaceBook {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub shelf_id: Uuid,
    pub name: String,
    pub description: String,
    pub created_at: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct HierarchyPage<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<Uuid>,
}
