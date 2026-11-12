-- Body-free terminal fences belong to a client, never to a caller's agent name.
CREATE TABLE IF NOT EXISTS ccp_content_cancellations (
    workspace_id TEXT NOT NULL REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    operation_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    credential_id TEXT NOT NULL,
    request_hash BLOB NOT NULL CHECK(length(request_hash)=32),
    cancelled_at INTEGER NOT NULL CHECK(cancelled_at>=0),
    PRIMARY KEY(workspace_id,operation_id,client_id),
    FOREIGN KEY(credential_id,client_id)
        REFERENCES ccp_credentials(id,client_id) ON DELETE RESTRICT
);
CREATE TRIGGER IF NOT EXISTS ccp_immutable_content_cancellation
BEFORE UPDATE ON ccp_content_cancellations
BEGIN SELECT RAISE(ABORT,'content cancellation bindings are immutable'); END;
CREATE TRIGGER IF NOT EXISTS ccp_cancellation_excludes_own_commit
BEFORE INSERT ON ccp_content_cancellations
WHEN EXISTS(SELECT 1 FROM ccp_content_operations o JOIN ccp_credentials c ON c.id=o.actor_id
    WHERE o.workspace_id=NEW.workspace_id AND o.operation_id=NEW.operation_id
    AND o.actor_kind='credential' AND c.client_id=NEW.client_id)
BEGIN SELECT RAISE(ABORT,'client operation is already committed'); END;
CREATE TRIGGER IF NOT EXISTS ccp_operation_excludes_own_cancellation
BEFORE INSERT ON ccp_content_operations
WHEN NEW.actor_kind='credential' AND EXISTS(
    SELECT 1 FROM ccp_content_cancellations x JOIN ccp_credentials c ON c.client_id=x.client_id
    WHERE x.workspace_id=NEW.workspace_id AND x.operation_id=NEW.operation_id AND c.id=NEW.actor_id)
BEGIN SELECT RAISE(ABORT,'client operation is cancelled'); END;
