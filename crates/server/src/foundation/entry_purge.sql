CREATE TABLE IF NOT EXISTS ccp_entry_purges (
    workspace_id TEXT NOT NULL REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    operation_id TEXT NOT NULL,
    actor_kind TEXT NOT NULL CHECK(actor_kind IN ('admin','key')),
    actor_id TEXT NOT NULL,
    principal_id TEXT NOT NULL REFERENCES ccp_admin_principals(id) ON DELETE RESTRICT,
    entry_id TEXT NOT NULL UNIQUE,
    revision INTEGER NOT NULL CHECK(revision>0),
    purged_at INTEGER NOT NULL CHECK(purged_at>=0),
    PRIMARY KEY(workspace_id,operation_id)
);
CREATE TRIGGER IF NOT EXISTS ccp_validate_entry_purge
BEFORE INSERT ON ccp_entry_purges
WHEN NOT EXISTS(SELECT 1 FROM ccp_content_entries e JOIN ccp_content_versions v
    ON v.entry_id=e.id AND v.workspace_id=e.workspace_id AND v.revision=e.revision
    WHERE e.id=NEW.entry_id AND e.workspace_id=NEW.workspace_id
    AND e.revision=NEW.revision AND e.archived=1)
BEGIN SELECT RAISE(ABORT, 'entry purge requires its archived head version'); END;
CREATE TRIGGER IF NOT EXISTS ccp_immutable_entry_purge
BEFORE UPDATE ON ccp_entry_purges
BEGIN SELECT RAISE(ABORT, 'entry purge proof is immutable'); END;
CREATE TRIGGER IF NOT EXISTS ccp_retain_entry_purge
BEFORE DELETE ON ccp_entry_purges
BEGIN SELECT RAISE(ABORT, 'entry purge proof must be retained'); END;
CREATE TRIGGER IF NOT EXISTS ccp_no_purged_entry_recreation
BEFORE INSERT ON ccp_content_entries
WHEN EXISTS(SELECT 1 FROM ccp_entry_purges WHERE entry_id=NEW.id)
BEGIN SELECT RAISE(ABORT, 'purged entry identity cannot be reused'); END;
