-- Shelf visibility is an atomic parent overlay; descendant rows stay intact.
CREATE TABLE IF NOT EXISTS ccp_shelf_lifecycle (
    shelf_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    FOREIGN KEY(shelf_id, workspace_id)
        REFERENCES ccp_content_shelves(id, workspace_id) ON DELETE RESTRICT
);
INSERT OR IGNORE INTO ccp_shelf_lifecycle(shelf_id,workspace_id,revision)
SELECT id,workspace_id,1 FROM ccp_content_shelves;
CREATE TABLE IF NOT EXISTS ccp_shelf_operations (
    workspace_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    actor_kind TEXT NOT NULL CHECK(actor_kind IN ('admin','key')),
    actor_id TEXT NOT NULL,
    shelf_id TEXT NOT NULL,
    expected_revision INTEGER NOT NULL CHECK(expected_revision > 0),
    archived INTEGER NOT NULL CHECK(archived IN (0,1)),
    revision INTEGER NOT NULL CHECK(revision > expected_revision AND revision=expected_revision+1),
    PRIMARY KEY(workspace_id, operation_id),
    FOREIGN KEY(shelf_id, workspace_id)
        REFERENCES ccp_content_shelves(id, workspace_id) ON DELETE RESTRICT
);
CREATE TRIGGER IF NOT EXISTS ccp_immutable_shelf_operation
BEFORE UPDATE ON ccp_shelf_operations
BEGIN SELECT RAISE(ABORT, 'shelf operation receipts are immutable'); END;
