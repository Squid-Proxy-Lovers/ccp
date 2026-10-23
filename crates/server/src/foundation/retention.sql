CREATE TABLE IF NOT EXISTS ccp_entry_retention_versions (
    workspace_id TEXT NOT NULL REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    revision INTEGER NOT NULL CHECK(revision>0),
    operation_id TEXT NOT NULL,
    retention_seconds INTEGER CHECK(retention_seconds>0),
    actor_kind TEXT NOT NULL CHECK(actor_kind IN ('admin','key')),
    actor_id TEXT NOT NULL,
    principal_id TEXT NOT NULL REFERENCES ccp_admin_principals(id) ON DELETE RESTRICT,
    changed_at INTEGER NOT NULL CHECK(changed_at>=0),
    PRIMARY KEY(workspace_id,revision),
    UNIQUE(workspace_id,operation_id)
);
CREATE TABLE IF NOT EXISTS ccp_entry_retention_heads (
    workspace_id TEXT PRIMARY KEY REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    revision INTEGER NOT NULL CHECK(revision>0),
    FOREIGN KEY(workspace_id,revision)
        REFERENCES ccp_entry_retention_versions(workspace_id,revision) ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS ccp_entry_retention_clocks (
    entry_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    archive_revision INTEGER NOT NULL CHECK(archive_revision>0),
    policy_revision INTEGER NOT NULL CHECK(policy_revision>0),
    remaining_seconds INTEGER NOT NULL CHECK(remaining_seconds>=0),
    active_since INTEGER CHECK(active_since>=0),
    FOREIGN KEY(entry_id,workspace_id)
        REFERENCES ccp_content_entries(id,workspace_id) ON DELETE CASCADE,
    FOREIGN KEY(workspace_id,policy_revision)
        REFERENCES ccp_entry_retention_versions(workspace_id,revision) ON DELETE RESTRICT
);
CREATE INDEX IF NOT EXISTS ccp_entry_retention_due
ON ccp_entry_retention_clocks(workspace_id,active_since,entry_id);
CREATE TABLE IF NOT EXISTS ccp_entry_retention_purges (
    entry_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    archive_revision INTEGER NOT NULL CHECK(archive_revision>0),
    policy_revision INTEGER NOT NULL CHECK(policy_revision>0),
    purged_at INTEGER NOT NULL CHECK(purged_at>=0),
    FOREIGN KEY(workspace_id,policy_revision)
        REFERENCES ccp_entry_retention_versions(workspace_id,revision) ON DELETE RESTRICT
);
CREATE TRIGGER IF NOT EXISTS ccp_immutable_entry_retention_version
BEFORE UPDATE ON ccp_entry_retention_versions
BEGIN SELECT RAISE(ABORT,'retention policy history is immutable'); END;
CREATE TRIGGER IF NOT EXISTS ccp_retain_entry_retention_version
BEFORE DELETE ON ccp_entry_retention_versions
BEGIN SELECT RAISE(ABORT,'retention policy history must be retained'); END;
CREATE TRIGGER IF NOT EXISTS ccp_validate_entry_retention_clock
BEFORE INSERT ON ccp_entry_retention_clocks
WHEN NOT EXISTS(SELECT 1 FROM ccp_content_entries e
    JOIN ccp_workspaces w ON w.id=e.workspace_id
    JOIN ccp_entry_retention_heads h ON h.workspace_id=e.workspace_id
    JOIN ccp_entry_retention_versions p ON p.workspace_id=h.workspace_id AND p.revision=h.revision
    WHERE e.id=NEW.entry_id AND e.workspace_id=NEW.workspace_id
    AND e.archived=1 AND e.revision=NEW.archive_revision
    AND w.state='active' AND w.legacy_session_id IS NULL
    AND p.revision=NEW.policy_revision AND p.retention_seconds=NEW.remaining_seconds
    AND NEW.active_since IS NOT NULL)
BEGIN SELECT RAISE(ABORT,'retention clock requires current policy and archived head'); END;
CREATE TRIGGER IF NOT EXISTS ccp_validate_entry_retention_purge
BEFORE INSERT ON ccp_entry_retention_purges
WHEN NOT EXISTS(SELECT 1 FROM ccp_entry_retention_clocks c
    JOIN ccp_content_entries e ON e.id=c.entry_id AND e.workspace_id=c.workspace_id
    JOIN ccp_content_versions v ON v.entry_id=e.id AND v.workspace_id=e.workspace_id AND v.revision=e.revision
    JOIN ccp_workspaces w ON w.id=e.workspace_id
    JOIN ccp_entry_retention_heads h ON h.workspace_id=e.workspace_id
    JOIN ccp_entry_retention_versions p ON p.workspace_id=h.workspace_id AND p.revision=h.revision
    WHERE c.entry_id=NEW.entry_id AND c.workspace_id=NEW.workspace_id
    AND c.archive_revision=NEW.archive_revision AND c.policy_revision=NEW.policy_revision
    AND e.archived=1 AND e.revision=c.archive_revision AND h.revision=c.policy_revision
    AND w.state='active' AND w.legacy_session_id IS NULL AND p.retention_seconds IS NOT NULL
    AND c.active_since IS NOT NULL AND NEW.purged_at>=c.active_since
    AND NEW.purged_at-c.active_since>=c.remaining_seconds)
BEGIN SELECT RAISE(ABORT,'timed purge requires current due archive and active policy'); END;
CREATE TRIGGER IF NOT EXISTS ccp_immutable_entry_retention_purge
BEFORE UPDATE ON ccp_entry_retention_purges
BEGIN SELECT RAISE(ABORT,'timed purge proof is immutable'); END;
CREATE TRIGGER IF NOT EXISTS ccp_retain_entry_retention_purge
BEFORE DELETE ON ccp_entry_retention_purges
BEGIN SELECT RAISE(ABORT,'timed purge proof must be retained'); END;
CREATE TRIGGER IF NOT EXISTS ccp_no_timed_purged_entry_recreation
BEFORE INSERT ON ccp_content_entries
WHEN EXISTS(SELECT 1 FROM ccp_entry_retention_purges WHERE entry_id=NEW.id)
BEGIN SELECT RAISE(ABORT,'timed-purged entry identity cannot be reused'); END;
