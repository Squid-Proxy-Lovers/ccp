CREATE TABLE IF NOT EXISTS ccp_content_entries (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    book_id TEXT NOT NULL,
    name TEXT NOT NULL CHECK(length(CAST(name AS BLOB)) BETWEEN 1 AND 256),
    owner_client_id TEXT REFERENCES ccp_clients(id) ON DELETE RESTRICT,
    revision INTEGER NOT NULL CHECK(revision > 0),
    archived INTEGER NOT NULL CHECK(archived IN (0,1)),
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    updated_at INTEGER NOT NULL CHECK(updated_at >= 0),
    UNIQUE(id, workspace_id),
    FOREIGN KEY(book_id, workspace_id)
        REFERENCES ccp_content_books(id, workspace_id) ON DELETE RESTRICT
);
CREATE UNIQUE INDEX IF NOT EXISTS ccp_active_entry_name
ON ccp_content_entries(workspace_id, book_id, name) WHERE archived=0;
CREATE TABLE IF NOT EXISTS ccp_content_versions (
    entry_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    action TEXT NOT NULL CHECK(action IN ('create','append','revise','archive','restore')),
    actor_kind TEXT NOT NULL CHECK(actor_kind IN ('admin','key','credential')),
    actor_id TEXT NOT NULL,
    client_id TEXT REFERENCES ccp_clients(id) ON DELETE RESTRICT,
    admin_principal_id TEXT REFERENCES ccp_admin_principals(id) ON DELETE RESTRICT,
    agent TEXT,
    recorded_at INTEGER NOT NULL CHECK(recorded_at >= 0),
    content_json TEXT NOT NULL,
    appended_text TEXT,
    PRIMARY KEY(entry_id, revision),
    UNIQUE(entry_id, revision, workspace_id),
    FOREIGN KEY(entry_id, workspace_id)
        REFERENCES ccp_content_entries(id, workspace_id) ON DELETE RESTRICT
);
-- Bounded overview reads scan one workspace's recent versions by time.
CREATE INDEX IF NOT EXISTS ccp_content_versions_recorded
    ON ccp_content_versions(workspace_id, recorded_at);
CREATE TRIGGER IF NOT EXISTS ccp_immutable_content_version
BEFORE UPDATE ON ccp_content_versions
BEGIN SELECT RAISE(ABORT, 'content versions are immutable'); END;
CREATE TRIGGER IF NOT EXISTS ccp_immutable_entry_identity
BEFORE UPDATE OF id,workspace_id,book_id,name,owner_client_id,created_at ON ccp_content_entries
BEGIN SELECT RAISE(ABORT, 'entry identity and ownership are immutable'); END;
-- Body-free historical proof survives later payload/version removal.
CREATE TABLE IF NOT EXISTS ccp_content_operations (
    workspace_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    actor_kind TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    request_hash BLOB NOT NULL CHECK(length(request_hash)=32),
    entry_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    PRIMARY KEY(workspace_id, operation_id),
    FOREIGN KEY(workspace_id) REFERENCES ccp_workspaces(id) ON DELETE RESTRICT
);
CREATE TRIGGER IF NOT EXISTS ccp_immutable_content_operation
BEFORE UPDATE ON ccp_content_operations
BEGIN SELECT RAISE(ABORT, 'content operation receipts are immutable'); END;
CREATE TRIGGER IF NOT EXISTS ccp_retain_content_operation
BEFORE DELETE ON ccp_content_operations
BEGIN SELECT RAISE(ABORT, 'content operation receipts must be retained'); END;
CREATE TRIGGER IF NOT EXISTS ccp_validate_content_operation
BEFORE INSERT ON ccp_content_operations
WHEN NOT EXISTS(SELECT 1 FROM ccp_content_versions v
    WHERE v.entry_id=NEW.entry_id AND v.revision=NEW.revision AND v.workspace_id=NEW.workspace_id)
BEGIN SELECT RAISE(ABORT, 'content receipt requires a matching committed version'); END;
