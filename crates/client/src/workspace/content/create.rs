// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

pub(crate) mod cli;

const MAX_REQUEST: usize = 64 * 1024;

impl WorkspaceProfile {
    /// Create one shelf. There is no operation receipt or automatic retry;
    /// inspect workspace shelves if the outcome cannot be confirmed.
    pub async fn content_create_shelf(&mut self, request: &CreateShelf) -> anyhow::Result<Shelf> {
        input(
            &request.workspace_id,
            &request.name,
            &request.description,
            request,
        )?;
        self.prepare_creation().await?;
        self.content_shelves(&ShelfPage {
            workspace_id: request.workspace_id.clone(),
            after: None,
            limit: Some(1),
        })
        .await?;
        self.create_once("shelves/create", request, |value: &Shelf| {
            validate::shelf(value, &request.workspace_id)?;
            result(
                &value.name,
                &value.description,
                &request.name,
                &request.description,
            )
        })
        .await
    }

    /// Create one book under an explicit shelf, without retrying an uncertain POST.
    pub async fn content_create_book(&mut self, request: &CreateBook) -> anyhow::Result<Book> {
        input(
            &request.workspace_id,
            &request.name,
            &request.description,
            request,
        )?;
        validate::ids(&[&request.shelf_id])?;
        self.prepare_creation().await?;
        self.content_shelf(&NodeTarget {
            workspace_id: request.workspace_id.clone(),
            node_id: request.shelf_id.clone(),
        })
        .await?;
        self.create_once("books/create", request, |value: &Book| {
            validate::book(value, &request.workspace_id, Some(&request.shelf_id))?;
            result(
                &value.name,
                &value.description,
                &request.name,
                &request.description,
            )
        })
        .await
    }

    async fn prepare_creation(&mut self) -> anyhow::Result<()> {
        self.require_no_content_pending()?;
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.operation == Operation::Renewal)
        {
            self.renew().await?;
        }
        if self.pending.is_some() {
            bail!("finish the pending credential operation before creating a shelf or book");
        }
        Ok(())
    }

    async fn create_once<Q: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        request: &Q,
        check: impl Fn(&T) -> anyhow::Result<()>,
    ) -> anyhow::Result<T> {
        let value = self.content_request(path, request).await.map_err(|error| {
            anyhow::anyhow!("creation outcome unconfirmed ({error}); inspect the explicit parent listing before retrying")
        })?;
        check(&value).map_err(|_response| anyhow::anyhow!(
            "creation response differs from the request; inspect the explicit parent listing before retrying"
        ))?;
        Ok(value)
    }
}

fn input(
    workspace: &str,
    name: &str,
    description: &str,
    request: &impl Serialize,
) -> anyhow::Result<()> {
    validate::ids(&[workspace])?;
    let normalized = name.trim();
    if name.len() > MAX_REQUEST
        || normalized.is_empty()
        || normalized.len() > 256
        || normalized.chars().any(char::is_control)
        || description.len() > 4096
        || serde_json::to_vec(request)?.len() > MAX_REQUEST
    {
        bail!("invalid shelf or book name, description or encoded request size");
    }
    Ok(())
}

fn result(
    name: &str,
    description: &str,
    expected_name: &str,
    expected_description: &str,
) -> anyhow::Result<()> {
    if name != expected_name.trim() || description != expected_description {
        bail!("creation metadata differs from the request");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
