// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

pub(crate) mod cli;

impl WorkspaceProfile {
    pub async fn content_shelf_state(
        &mut self,
        request: &NodeTarget,
    ) -> anyhow::Result<ShelfState> {
        validate::ids(&[&request.workspace_id, &request.node_id])?;
        self.content_read("shelves/state", request, |value: &ShelfState| {
            validate::shelf_state(value, request)
        })
        .await
    }

    pub async fn content_archived_shelves(
        &mut self,
        request: &ShelfPage,
    ) -> anyhow::Result<Page<ArchivedShelf>> {
        validate::ids(&[&request.workspace_id])?;
        validate::page_input(request.after.as_deref(), request.limit)?;
        self.content_read(
            "shelves/archived",
            request,
            |value: &Page<ArchivedShelf>| {
                validate::page(
                    value,
                    request.after.as_deref(),
                    request.limit,
                    |v| &v.id,
                    |v| validate::archived_shelf(v, &request.workspace_id),
                )
            },
        )
        .await
    }
}
