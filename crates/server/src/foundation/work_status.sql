-- Latest status text only. Clearing retains an ownership/revision tombstone.
CREATE TABLE IF NOT EXISTS ccp_work_status (
    workspace_id TEXT NOT NULL REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    id TEXT NOT NULL,
    client_id TEXT NOT NULL REFERENCES ccp_clients(id) ON DELETE RESTRICT,
    revision INTEGER NOT NULL CHECK(revision > 0),
    agent TEXT CHECK(agent IS NULL OR length(CAST(agent AS BLOB)) BETWEEN 1 AND 128),
    message TEXT CHECK(message IS NULL OR length(CAST(message AS BLOB)) BETWEEN 1 AND 4096),
    updated_at INTEGER NOT NULL CHECK(updated_at >= 0),
    CHECK(message IS NOT NULL OR agent IS NULL),
    PRIMARY KEY(workspace_id,id)
);
CREATE INDEX IF NOT EXISTS ccp_work_status_live
ON ccp_work_status(workspace_id,client_id) WHERE message IS NOT NULL;
CREATE TRIGGER IF NOT EXISTS ccp_immutable_work_status_owner
BEFORE UPDATE OF workspace_id,id,client_id ON ccp_work_status
BEGIN SELECT RAISE(ABORT,'work status identity and ownership are immutable'); END;
CREATE TRIGGER IF NOT EXISTS ccp_retain_work_status_tombstone
BEFORE DELETE ON ccp_work_status
BEGIN SELECT RAISE(ABORT,'work status ownership tombstones must be retained'); END;

-- No message, label, claimed identity or earlier full text is stored here.
CREATE TABLE IF NOT EXISTS ccp_work_status_operations (
    workspace_id TEXT NOT NULL REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    operation_id TEXT NOT NULL,
    actor_kind TEXT NOT NULL CHECK(actor_kind IN ('credential','admin','key')),
    actor_id TEXT NOT NULL,
    request_hash BLOB NOT NULL CHECK(length(request_hash)=32),
    status_id TEXT NOT NULL,
    client_id TEXT NOT NULL REFERENCES ccp_clients(id) ON DELETE RESTRICT,
    revision INTEGER NOT NULL CHECK(revision > 0),
    cleared INTEGER NOT NULL CHECK(cleared IN (0,1)),
    updated_at INTEGER NOT NULL CHECK(updated_at >= 0),
    PRIMARY KEY(workspace_id,operation_id)
);
CREATE TRIGGER IF NOT EXISTS ccp_immutable_work_status_operation
BEFORE UPDATE ON ccp_work_status_operations
BEGIN SELECT RAISE(ABORT,'work status receipts are immutable'); END;
CREATE TRIGGER IF NOT EXISTS ccp_retain_work_status_operation
BEFORE DELETE ON ccp_work_status_operations
BEGIN SELECT RAISE(ABORT,'work status receipts must be retained'); END;
CREATE TRIGGER IF NOT EXISTS ccp_validate_work_status_operation
BEFORE INSERT ON ccp_work_status_operations
WHEN NOT EXISTS(SELECT 1 FROM ccp_work_status h WHERE h.workspace_id=NEW.workspace_id
    AND h.id=NEW.status_id AND h.client_id=NEW.client_id AND h.revision=NEW.revision
    AND (h.message IS NULL)=NEW.cleared AND h.updated_at=NEW.updated_at)
BEGIN SELECT RAISE(ABORT,'work status receipt requires a matching committed head'); END;
