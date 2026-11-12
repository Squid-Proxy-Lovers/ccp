// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) mod cli;
mod disk;

/// Caller preferences, never evidence of current server permissions.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub instance_id: String,
    pub client_id: String,
    pub workspace_ids: Vec<String>,
    pub default_workspace_id: Option<String>,
    pub revision: u64,
}

pub struct SelectionUpdate {
    pub workspace_ids: Vec<String>,
    pub default_workspace_id: Option<String>,
    pub expected_revision: u64,
}

#[derive(Serialize)]
pub struct Target {
    pub instance_id: String,
    pub client_id: String,
    pub workspace: api::Workspace,
}

impl Selection {
    fn validate(&self, profile: &WorkspaceProfile) -> anyhow::Result<()> {
        if self.instance_id != profile.trust.instance_id
            || self.client_id != profile.selection_client_id()?
            || self.revision > i64::MAX.unsigned_abs()
        {
            bail!("caller selection belongs to another instance/client or has an invalid revision");
        }
        validate_ids(&self.workspace_ids, self.default_workspace_id.as_deref())
    }
}

fn validate_ids(ids: &[String], default: Option<&str>) -> anyhow::Result<()> {
    if ids.len() > 100 {
        bail!("select at most 100 workspaces");
    }
    let mut seen = BTreeSet::new();
    for id in ids {
        crypto::canonical_id(id)?;
        if !seen.insert(id) {
            bail!("duplicate workspace selection");
        }
    }
    if default.is_some_and(|id| !ids.iter().any(|selected| selected == id)) {
        bail!("the default workspace must be explicitly selected");
    }
    Ok(())
}

impl WorkspaceProfile {
    fn selection_client_id(&self) -> anyhow::Result<&str> {
        Ok(&self
            .credential
            .as_ref()
            .context("selection requires an enrolled profile")?
            .receipt
            .client_id)
    }

    pub fn initialize_selection(&self, directory: &Path) -> anyhow::Result<Selection> {
        let state = Selection {
            instance_id: self.trust.instance_id.clone(),
            client_id: self.selection_client_id()?.to_owned(),
            workspace_ids: Vec::new(),
            default_workspace_id: None,
            revision: 0,
        };
        disk::create(directory, &state)?;
        Ok(state)
    }

    pub fn selection(&self, directory: &Path) -> anyhow::Result<Selection> {
        let (state, _raw) = disk::read(&disk::open(directory)?)?;
        state.validate(self)?;
        Ok(state)
    }

    pub async fn set_selection(
        &mut self,
        directory: &Path,
        update: SelectionUpdate,
    ) -> anyhow::Result<Selection> {
        validate_ids(
            &update.workspace_ids,
            update.default_workspace_id.as_deref(),
        )?;
        let previous = self.selection(directory)?;
        if previous.revision != update.expected_revision {
            bail!("caller selection changed; inspect it before retrying");
        }
        if !update.workspace_ids.is_empty() {
            self.authorized_targets(&update.workspace_ids).await?;
        }
        let revision = update
            .expected_revision
            .checked_add(1)
            .filter(|v| *v <= i64::MAX.unsigned_abs())
            .context("selection revision exhausted")?;
        let state = Selection {
            instance_id: previous.instance_id,
            client_id: previous.client_id,
            workspace_ids: update.workspace_ids,
            default_workspace_id: update.default_workspace_id,
            revision,
        };
        disk::replace(directory, &state, update.expected_revision)?;
        Ok(state)
    }

    /// Fails the whole resolution if any selected target is unavailable.
    pub async fn selected_workspaces(&mut self, directory: &Path) -> anyhow::Result<Vec<Target>> {
        let state = self.selection(directory)?;
        if state.workspace_ids.is_empty() {
            bail!("no workspaces selected; select them explicitly");
        }
        self.authorized_targets(&state.workspace_ids).await
    }

    /// An explicit target does not read or rewrite caller preferences.
    pub async fn target(
        &mut self,
        directory: Option<&Path>,
        explicit: Option<&str>,
    ) -> anyhow::Result<Target> {
        let id = match explicit {
            Some(id) => id.to_owned(),
            None => self
                .selection(
                    directory.context("a caller selection directory is required for a default")?,
                )?
                .default_workspace_id
                .context("no default workspace; specify a target or configure a default")?,
        };
        let mut targets = self.authorized_targets(&[id]).await?;
        targets.pop().context("target is unavailable")
    }

    async fn authorized_targets(&mut self, ids: &[String]) -> anyhow::Result<Vec<Target>> {
        validate_ids(ids, None)?;
        let mut wanted: BTreeSet<_> = ids.iter().cloned().collect();
        let mut found = BTreeMap::new();
        let mut cursors = BTreeSet::new();
        let mut after: Option<String> = None;
        for _ in 0..20 {
            let page = self.workspaces(after.as_deref(), 500).await?;
            for workspace in page.workspaces {
                if wanted.remove(&workspace.id) {
                    found.insert(workspace.id.clone(), workspace);
                }
            }
            if wanted.is_empty() {
                return ids
                    .iter()
                    .map(|id| {
                        Ok(Target {
                            instance_id: self.trust.instance_id.clone(),
                            client_id: self.selection_client_id()?.to_owned(),
                            workspace: found.remove(id).context("workspace resolution changed")?,
                        })
                    })
                    .collect();
            }
            match page.next_cursor {
                None => bail!(
                    "selected target unavailable to this credential; inspect access and select deliberately"
                ),
                Some(cursor) if cursors.insert(cursor.clone()) => after = Some(cursor),
                Some(_) => bail!("workspace discovery repeated its cursor"),
            }
        }
        bail!("workspace discovery exceeded the 20-page resolution limit")
    }
}
