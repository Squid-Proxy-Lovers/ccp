-- Source labels and records are provenance, never operational authority.
CREATE TABLE IF NOT EXISTS ccp_native_imports (
    workspace_id TEXT PRIMARY KEY REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    source_sha256 TEXT NOT NULL CHECK(length(source_sha256)=64 AND source_sha256 NOT GLOB '*[^0-9a-f]*'),
    source_schema INTEGER NOT NULL CHECK(source_schema BETWEEN 0 AND 4294967295),
    legacy_session_id INTEGER NOT NULL,
    recorded_by TEXT NOT NULL REFERENCES ccp_admin_principals(id) ON DELETE RESTRICT,
    recorded_at INTEGER NOT NULL CHECK(recorded_at >= 0),
    sealed INTEGER NOT NULL DEFAULT 0 CHECK(sealed IN (0,1))
);
CREATE TABLE IF NOT EXISTS ccp_native_entry_rows (
    workspace_id TEXT NOT NULL REFERENCES ccp_native_imports(workspace_id) ON DELETE RESTRICT,
    entry_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK(ordinal > 0),
    source_table TEXT NOT NULL CHECK(source_table IN ('message_packs','message_history','deleted_message_packs','deleted_message_history')),
    source_key TEXT NOT NULL CHECK(length(CAST(source_key AS BLOB)) BETWEEN 1 AND 8192),
    row_json TEXT NOT NULL CHECK(length(CAST(row_json AS BLOB)) BETWEEN 1 AND 2097152),
    PRIMARY KEY(entry_id,ordinal),
    UNIQUE(workspace_id,source_table,source_key),
    FOREIGN KEY(entry_id,workspace_id) REFERENCES ccp_content_entries(id,workspace_id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS ccp_native_workspace_rows (
    workspace_id TEXT NOT NULL REFERENCES ccp_native_imports(workspace_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK(ordinal > 0),
    source_table TEXT NOT NULL CHECK(source_table IN ('sessions','shelves','books','issued_client_certs','auth_tokens','transfer_log')),
    source_key TEXT NOT NULL CHECK(length(CAST(source_key AS BLOB)) BETWEEN 1 AND 8192),
    row_json TEXT NOT NULL CHECK(length(CAST(row_json AS BLOB)) BETWEEN 1 AND 2097152),
    PRIMARY KEY(workspace_id,ordinal),
    UNIQUE(workspace_id,source_table,source_key)
);
CREATE TRIGGER IF NOT EXISTS ccp_seal_native_import
BEFORE UPDATE ON ccp_native_imports
WHEN OLD.sealed!=0 OR NEW.sealed!=1
    OR NEW.workspace_id IS NOT OLD.workspace_id OR NEW.source_sha256 IS NOT OLD.source_sha256
    OR NEW.source_schema IS NOT OLD.source_schema OR NEW.legacy_session_id IS NOT OLD.legacy_session_id
    OR NEW.recorded_by IS NOT OLD.recorded_by OR NEW.recorded_at IS NOT OLD.recorded_at
    OR EXISTS(SELECT 1 FROM ccp_workspaces WHERE id=NEW.workspace_id AND state='purged')
BEGIN SELECT RAISE(ABORT,'native import provenance is immutable'); END;
CREATE TRIGGER IF NOT EXISTS ccp_no_replaced_native_import
BEFORE INSERT ON ccp_native_imports
WHEN EXISTS(SELECT 1 FROM ccp_native_imports WHERE workspace_id=NEW.workspace_id)
BEGIN SELECT RAISE(ABORT,'native import provenance cannot be replaced'); END;
CREATE TRIGGER IF NOT EXISTS ccp_sealed_native_entry_rows
BEFORE INSERT ON ccp_native_entry_rows
WHEN EXISTS(SELECT 1 FROM ccp_native_imports WHERE workspace_id=NEW.workspace_id AND sealed=1)
    OR EXISTS(SELECT 1 FROM ccp_native_entry_rows WHERE (entry_id=NEW.entry_id AND ordinal=NEW.ordinal)
        OR (workspace_id=NEW.workspace_id AND source_table=NEW.source_table AND source_key=NEW.source_key))
BEGIN SELECT RAISE(ABORT,'native import is sealed'); END;
CREATE TRIGGER IF NOT EXISTS ccp_sealed_native_workspace_rows
BEFORE INSERT ON ccp_native_workspace_rows
WHEN EXISTS(SELECT 1 FROM ccp_native_imports WHERE workspace_id=NEW.workspace_id AND sealed=1)
    OR EXISTS(SELECT 1 FROM ccp_native_workspace_rows WHERE workspace_id=NEW.workspace_id
        AND (ordinal=NEW.ordinal OR (source_table=NEW.source_table AND source_key=NEW.source_key)))
BEGIN SELECT RAISE(ABORT,'native import is sealed'); END;
CREATE TRIGGER IF NOT EXISTS ccp_immutable_native_entry_row
BEFORE UPDATE ON ccp_native_entry_rows
BEGIN SELECT RAISE(ABORT,'native entry provenance is immutable'); END;
CREATE TRIGGER IF NOT EXISTS ccp_immutable_native_workspace_row
BEFORE UPDATE ON ccp_native_workspace_rows
BEGIN SELECT RAISE(ABORT,'native workspace provenance is immutable'); END;
-- A parent entry purge cascades after the parent row is gone. Otherwise only
-- an explicitly confirmed whole-workspace job may retire its raw records.
CREATE TRIGGER IF NOT EXISTS ccp_retain_native_entry_row
BEFORE DELETE ON ccp_native_entry_rows
WHEN EXISTS(SELECT 1 FROM ccp_content_entries WHERE id=OLD.entry_id AND workspace_id=OLD.workspace_id)
    AND NOT EXISTS(SELECT 1 FROM ccp_workspace_deletions d JOIN ccp_workspaces w
        ON w.id=d.workspace_id WHERE d.workspace_id=OLD.workspace_id AND w.state='purged')
BEGIN SELECT RAISE(ABORT,'native entry provenance must follow its parent purge'); END;
