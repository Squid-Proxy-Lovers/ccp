-- A terminal workspace identity replaces its retired per-operation content fences.
CREATE TABLE IF NOT EXISTS ccp_workspace_lifecycle (
    workspace_id TEXT PRIMARY KEY REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>0)
);
INSERT OR IGNORE INTO ccp_workspace_lifecycle SELECT id,1 FROM ccp_workspaces;
CREATE TRIGGER IF NOT EXISTS ccp_initial_workspace_lifecycle
AFTER INSERT ON ccp_workspaces
BEGIN INSERT INTO ccp_workspace_lifecycle VALUES(NEW.id,1); END;
CREATE TRIGGER IF NOT EXISTS ccp_advance_workspace_lifecycle
AFTER UPDATE OF state,name,owner_admin_id ON ccp_workspaces
WHEN OLD.state IS NOT NEW.state OR OLD.name IS NOT NEW.name OR OLD.owner_admin_id IS NOT NEW.owner_admin_id
BEGIN
    UPDATE ccp_workspace_lifecycle SET revision=revision+1 WHERE workspace_id=NEW.id;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'workspace lifecycle missing') END;
END;
CREATE TRIGGER IF NOT EXISTS ccp_monotonic_workspace_lifecycle
BEFORE UPDATE ON ccp_workspace_lifecycle
WHEN NEW.workspace_id!=OLD.workspace_id OR NEW.revision!=OLD.revision+1
BEGIN SELECT RAISE(ABORT,'workspace lifecycle must advance'); END;
CREATE TRIGGER IF NOT EXISTS ccp_retain_workspace_lifecycle
BEFORE DELETE ON ccp_workspace_lifecycle
BEGIN SELECT RAISE(ABORT,'workspace lifecycle identity must be retained'); END;
CREATE TRIGGER IF NOT EXISTS ccp_terminal_workspace_identity
BEFORE UPDATE ON ccp_workspaces
WHEN OLD.state='purged' AND (NEW.id IS NOT OLD.id OR NEW.state IS NOT OLD.state
    OR NEW.name IS NOT OLD.name OR NEW.owner_admin_id IS NOT OLD.owner_admin_id
    OR NEW.legacy_session_id IS NOT OLD.legacy_session_id)
BEGIN SELECT RAISE(ABORT,'deleted workspace identity is terminal'); END;
CREATE TRIGGER IF NOT EXISTS ccp_no_deleted_workspace_replacement
BEFORE INSERT ON ccp_workspaces
WHEN EXISTS(SELECT 1 FROM ccp_workspaces WHERE id=NEW.id AND state='purged')
BEGIN SELECT RAISE(ABORT,'deleted workspace identity cannot be reused'); END;

CREATE TABLE IF NOT EXISTS ccp_workspace_deletions (
    workspace_id TEXT PRIMARY KEY REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    operation_id TEXT NOT NULL UNIQUE,
    actor_kind TEXT NOT NULL CHECK(actor_kind IN ('admin','key')),
    actor_id TEXT NOT NULL,
    principal_id TEXT NOT NULL REFERENCES ccp_admin_principals(id) ON DELETE RESTRICT,
    expected_revision INTEGER NOT NULL CHECK(expected_revision>0),
    started_at INTEGER NOT NULL CHECK(started_at>=0),
    completed_at INTEGER CHECK(completed_at>=started_at)
);
CREATE INDEX IF NOT EXISTS ccp_pending_workspace_deletions
ON ccp_workspace_deletions(workspace_id) WHERE completed_at IS NULL;
CREATE TRIGGER IF NOT EXISTS ccp_validate_workspace_deletion
BEFORE INSERT ON ccp_workspace_deletions
WHEN NOT EXISTS(SELECT 1 FROM ccp_workspaces w JOIN ccp_workspace_lifecycle l ON l.workspace_id=w.id
    WHERE w.id=NEW.workspace_id AND w.state='archived' AND w.legacy_session_id IS NULL
    AND l.revision=NEW.expected_revision)
BEGIN SELECT RAISE(ABORT,'workspace deletion requires its archived lifecycle'); END;
CREATE TRIGGER IF NOT EXISTS ccp_immutable_workspace_deletion
BEFORE UPDATE ON ccp_workspace_deletions
WHEN NEW.workspace_id IS NOT OLD.workspace_id OR NEW.operation_id IS NOT OLD.operation_id
    OR NEW.actor_kind IS NOT OLD.actor_kind OR NEW.actor_id IS NOT OLD.actor_id
    OR NEW.principal_id IS NOT OLD.principal_id OR NEW.expected_revision IS NOT OLD.expected_revision
    OR NEW.started_at IS NOT OLD.started_at OR OLD.completed_at IS NOT NULL OR NEW.completed_at IS NULL
BEGIN SELECT RAISE(ABORT,'workspace deletion binding is immutable'); END;
CREATE TRIGGER IF NOT EXISTS ccp_retain_workspace_deletion
BEFORE DELETE ON ccp_workspace_deletions
BEGIN SELECT RAISE(ABORT,'workspace deletion proof must be retained'); END;
