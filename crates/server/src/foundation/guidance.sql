CREATE TABLE IF NOT EXISTS ccp_guidance_versions (
    board_key TEXT NOT NULL,
    workspace_id TEXT REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    revision INTEGER NOT NULL CHECK(revision > 0),
    operation_id TEXT NOT NULL,
    text TEXT NOT NULL CHECK(length(CAST(text AS BLOB)) <= 8192),
    publisher_kind TEXT NOT NULL CHECK(publisher_kind IN ('admin','key')),
    publisher_id TEXT NOT NULL,
    principal_id TEXT NOT NULL REFERENCES ccp_admin_principals(id) ON DELETE RESTRICT,
    recorded_at INTEGER NOT NULL CHECK(recorded_at >= 0),
    PRIMARY KEY(board_key, revision),
    UNIQUE(board_key, operation_id),
    CHECK((board_key='global' AND workspace_id IS NULL) OR
          (workspace_id IS NOT NULL AND board_key=workspace_id))
);
CREATE TRIGGER IF NOT EXISTS ccp_immutable_guidance_version
BEFORE UPDATE ON ccp_guidance_versions
BEGIN SELECT RAISE(ABORT, 'guidance revisions are immutable'); END;
CREATE TABLE IF NOT EXISTS ccp_guidance_heads (
    board_key TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK(revision > 0),
    FOREIGN KEY(board_key, revision) REFERENCES ccp_guidance_versions(board_key, revision) ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS ccp_guidance_operations (
    board_key TEXT NOT NULL,
    workspace_id TEXT REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    operation_id TEXT NOT NULL,
    publisher_kind TEXT NOT NULL CHECK(publisher_kind IN ('admin','key')),
    publisher_id TEXT NOT NULL,
    expected_revision INTEGER NOT NULL CHECK(expected_revision >= 0),
    request_hash BLOB NOT NULL CHECK(length(request_hash)=32),
    revision INTEGER NOT NULL CHECK(revision>expected_revision AND revision=expected_revision+1),
    PRIMARY KEY(board_key, operation_id),
    CHECK((board_key='global' AND workspace_id IS NULL) OR
          (workspace_id IS NOT NULL AND board_key=workspace_id))
);
CREATE TRIGGER IF NOT EXISTS ccp_validate_guidance_operation
BEFORE INSERT ON ccp_guidance_operations
WHEN NOT EXISTS(SELECT 1 FROM ccp_guidance_versions v
    WHERE v.board_key=NEW.board_key AND v.revision=NEW.revision
    AND v.operation_id=NEW.operation_id AND v.publisher_kind=NEW.publisher_kind
    AND v.publisher_id=NEW.publisher_id)
BEGIN SELECT RAISE(ABORT, 'guidance receipt requires its published revision'); END;
CREATE TRIGGER IF NOT EXISTS ccp_immutable_guidance_operation
BEFORE UPDATE ON ccp_guidance_operations
BEGIN SELECT RAISE(ABORT, 'guidance receipts are immutable'); END;
CREATE TRIGGER IF NOT EXISTS ccp_retain_guidance_operation
BEFORE DELETE ON ccp_guidance_operations
BEGIN SELECT RAISE(ABORT, 'guidance receipts must be retained'); END;
