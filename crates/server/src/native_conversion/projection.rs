// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::foundation::{EntryContent, NativeRow, NativeSqlValue};
use anyhow::{Context, bail};
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && name.trim() == name
        && !name.chars().any(char::is_control)
}
pub(super) fn text<'a>(row: &'a NativeRow, column: &str) -> anyhow::Result<&'a str> {
    let index = row
        .columns
        .iter()
        .position(|name| name == column)
        .context("source field missing")?;
    match row.values.get(index) {
        Some(NativeSqlValue::Text(value)) => Ok(value),
        _ => bail!("source field has unsupported type"),
    }
}
pub(super) fn integer(row: &NativeRow, column: &str) -> anyhow::Result<i64> {
    let index = row
        .columns
        .iter()
        .position(|name| name == column)
        .context("source field missing")?;
    match row.values.get(index) {
        Some(NativeSqlValue::Integer(value)) => Ok(*value),
        _ => bail!("source field has unsupported type"),
    }
}
fn name(row: &NativeRow, column: &str) -> anyhow::Result<String> {
    let value = text(row, column)?;
    if !valid_name(value) {
        bail!("source name cannot be preserved");
    }
    Ok(value.to_owned())
}
fn description(row: &NativeRow) -> anyhow::Result<String> {
    let value = text(row, "description")?;
    if value.len() > 4096 {
        bail!("source description exceeds conversion bound");
    }
    Ok(value.to_owned())
}

pub(super) struct Projection<'a> {
    db: &'a Connection,
    workspace: Uuid,
    operator: Uuid,
    now: i64,
    shelves: BTreeMap<String, Uuid>,
    books: BTreeMap<(String, String), Uuid>,
}
impl<'a> Projection<'a> {
    pub fn new(db: &'a Connection, workspace: Uuid, operator: Uuid, now: i64) -> Self {
        Self {
            db,
            workspace,
            operator,
            now,
            shelves: BTreeMap::new(),
            books: BTreeMap::new(),
        }
    }
    pub fn physical_shelf(&mut self, row: &NativeRow) -> anyhow::Result<()> {
        let name = name(row, "shelf_name")?;
        if self.shelves.contains_key(&name) {
            bail!("duplicate source shelf");
        }
        self.insert_shelf(name, description(row)?)?;
        Ok(())
    }
    pub fn physical_book(&mut self, row: &NativeRow) -> anyhow::Result<()> {
        let shelf = name(row, "shelf_name")?;
        let book = name(row, "book_name")?;
        if self.books.contains_key(&(shelf.clone(), book.clone())) {
            bail!("duplicate source book");
        }
        self.insert_book(shelf, book, description(row)?)?;
        Ok(())
    }
    fn insert_shelf(&mut self, name: String, description: String) -> anyhow::Result<Uuid> {
        let id = Uuid::new_v4();
        self.db.execute("INSERT INTO ccp_content_shelves(id,workspace_id,name,description,created_at) VALUES(?1,?2,?3,?4,?5)",params![id.to_string(),self.workspace.to_string(),name,description,self.now])?;
        self.shelves.insert(name, id);
        Ok(id)
    }
    fn ensure_shelf(&mut self, name: String) -> anyhow::Result<Uuid> {
        if let Some(id) = self.shelves.get(&name) {
            Ok(*id)
        } else {
            self.insert_shelf(name, String::new())
        }
    }
    fn insert_book(
        &mut self,
        shelf: String,
        name: String,
        description: String,
    ) -> anyhow::Result<Uuid> {
        let shelf_id = self.ensure_shelf(shelf.clone())?;
        let id = Uuid::new_v4();
        self.db.execute("INSERT INTO ccp_content_books(id,workspace_id,shelf_id,name,description,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![id.to_string(),self.workspace.to_string(),shelf_id.to_string(),name,description,self.now])?;
        self.books.insert((shelf, name), id);
        Ok(id)
    }
    pub fn entry(&mut self, row: &NativeRow, archived: bool) -> anyhow::Result<Uuid> {
        let shelf = name(row, "shelf_name")?;
        let book = name(row, "book_name")?;
        let book_id = if let Some(id) = self.books.get(&(shelf.clone(), book.clone())) {
            *id
        } else {
            self.insert_book(shelf, book, String::new())?
        };
        let name = name(row, "name")?;
        let content = content(row)?;
        let id = Uuid::new_v4();
        self.db.execute("INSERT INTO ccp_content_entries(id,workspace_id,book_id,name,owner_client_id,revision,archived,created_at,updated_at) VALUES(?1,?2,?3,?4,NULL,1,?5,?6,?6)",params![id.to_string(),self.workspace.to_string(),book_id.to_string(),name,archived,self.now])?;
        self.db.execute("INSERT INTO ccp_content_versions(entry_id,workspace_id,revision,action,actor_kind,actor_id,admin_principal_id,agent,recorded_at,content_json) VALUES(?1,?2,1,?3,'admin',?4,?4,'native conversion',?5,?6)",params![id.to_string(),self.workspace.to_string(),if archived {"archive"} else {"create"},self.operator.to_string(),self.now,serde_json::to_string(&content)?])?;
        Ok(id)
    }
}
fn content(row: &NativeRow) -> anyhow::Result<EntryContent> {
    let body = text(row, "context")?;
    let description = description(row)?;
    let raw = text(row, "labels")?;
    let labels = if raw.is_empty() {
        Vec::new()
    } else {
        raw.split(',').map(str::to_owned).collect::<Vec<_>>()
    };
    if body.len() > 256 * 1024
        || labels.len() > 32
        || labels
            .iter()
            .any(|v| v.is_empty() || v.len() > 128 || v.chars().any(char::is_control))
        || labels.iter().collect::<BTreeSet<_>>().len() != labels.len()
    {
        bail!("source content cannot be preserved within target bounds");
    }
    Ok(EntryContent {
        text: body.to_owned(),
        description,
        labels,
    })
}
