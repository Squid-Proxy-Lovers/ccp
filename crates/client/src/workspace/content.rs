// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Explicit workspace content reads using the profile's current pinned mTLS.
pub use protocol::workspace::content::{
    Book, BookPage, ContentActor, EntryContent, EntryPage, EntryRevision, EntrySnapshot,
    EntrySummary, EntryTarget, History, HistoryPage, NodeTarget, Page, Shelf, ShelfPage,
};

use super::*;
use serde::de::DeserializeOwned;

pub(crate) mod cli;
mod validate;

#[cfg(test)]
mod tests;

impl WorkspaceProfile {
    pub async fn content_shelf(&mut self, request: &NodeTarget) -> anyhow::Result<Shelf> {
        validate::ids(&[&request.workspace_id, &request.node_id])?;
        self.content_read("shelves/get", request, |value: &Shelf| {
            validate::shelf(value, &request.workspace_id)?;
            validate::same(&value.id, &request.node_id)
        })
        .await
    }

    pub async fn content_book(&mut self, request: &NodeTarget) -> anyhow::Result<Book> {
        validate::ids(&[&request.workspace_id, &request.node_id])?;
        self.content_read("books/get", request, |value: &Book| {
            validate::book(value, &request.workspace_id, None)?;
            validate::same(&value.id, &request.node_id)
        })
        .await
    }

    pub async fn content_shelves(&mut self, request: &ShelfPage) -> anyhow::Result<Page<Shelf>> {
        validate::ids(&[&request.workspace_id])?;
        validate::page_input(request.after.as_deref(), request.limit)?;
        self.content_read("shelves/list", request, |value: &Page<Shelf>| {
            validate::page(
                value,
                request.after.as_deref(),
                request.limit,
                |v| &v.id,
                |v| validate::shelf(v, &request.workspace_id),
            )
        })
        .await
    }

    pub async fn content_books(&mut self, request: &BookPage) -> anyhow::Result<Page<Book>> {
        validate::ids(&[&request.workspace_id, &request.shelf_id])?;
        validate::page_input(request.after.as_deref(), request.limit)?;
        self.content_read("books/list", request, |value: &Page<Book>| {
            validate::page(
                value,
                request.after.as_deref(),
                request.limit,
                |v| &v.id,
                |v| validate::book(v, &request.workspace_id, Some(&request.shelf_id)),
            )
        })
        .await
    }

    pub async fn content_entry(&mut self, request: &EntryTarget) -> anyhow::Result<EntrySnapshot> {
        validate::ids(&[&request.workspace_id, &request.entry_id])?;
        if let Some(revision) = request.revision {
            validate::revision_number(revision)?;
        }
        self.content_read("entries/get", request, |value: &EntrySnapshot| {
            validate::snapshot(value, request)
        })
        .await
    }

    pub async fn content_entries(
        &mut self,
        request: &EntryPage,
    ) -> anyhow::Result<Page<EntrySummary>> {
        validate::ids(&[&request.workspace_id, &request.book_id])?;
        validate::page_input(request.after.as_deref(), request.limit)?;
        self.content_read("entries/list", request, |value: &Page<EntrySummary>| {
            validate::page(
                value,
                request.after.as_deref(),
                request.limit,
                |v| &v.id,
                |v| {
                    validate::summary(v, &request.workspace_id)?;
                    validate::same(&v.book_id, &request.book_id)?;
                    if v.archived != request.archived {
                        bail!("content response archive state does not match target");
                    }
                    Ok(())
                },
            )
        })
        .await
    }

    pub async fn content_history(&mut self, request: &HistoryPage) -> anyhow::Result<History> {
        validate::ids(&[&request.workspace_id, &request.entry_id])?;
        validate::page_input(None, request.limit)?;
        if request.after.is_some_and(|n| n > i64::MAX.unsigned_abs()) {
            bail!("invalid content revision cursor");
        }
        self.content_read("entries/history", request, |value: &History| {
            validate::history(value, request)
        })
        .await
    }

    async fn content_read<Q: Serialize, T: DeserializeOwned>(
        &mut self,
        path: &str,
        request: &Q,
        check: impl Fn(&T) -> anyhow::Result<()>,
    ) -> anyhow::Result<T> {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.operation == Operation::Renewal)
        {
            self.renew().await?;
        }
        let value = self.fetch_content(path, request).await?;
        check(&value)?;
        // An authorized, validated target response proves a usable grant even
        // when its page is empty. No renewal is attempted after a denied read.
        if self.pending.is_none() && self.renewal_due(crypto::now()?)? {
            self.renew().await?;
            let value = self.fetch_content(path, request).await?;
            check(&value)?;
            return Ok(value);
        }
        Ok(value)
    }

    async fn fetch_content<Q: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        request: &Q,
    ) -> anyhow::Result<T> {
        let material = self
            .credential
            .as_ref()
            .context("content reads require an enrolled profile")?;
        crypto::credential(&self.trust, material, crypto::now()?)?;
        let http = network::client(&self.trust, Some(material))?;
        network::health(&http, &self.trust.mtls_origin, &self.trust.instance_id).await?;
        let request = http
            .post(format!("{}api/v1/content/{path}", self.trust.mtls_origin))
            .json(request);
        if path == "entries/get" {
            network::snapshot_json(request).await
        } else {
            network::json(request).await
        }
    }
}
