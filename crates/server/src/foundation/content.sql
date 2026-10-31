-- New workspace content is independent of the retained native session payload.
CREATE TABLE IF NOT EXISTS ccp_content_shelves (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    name TEXT NOT NULL CHECK(length(CAST(name AS BLOB)) BETWEEN 1 AND 256),
    description TEXT NOT NULL CHECK(length(CAST(description AS BLOB)) <= 4096),
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0,1)),
    UNIQUE(id, workspace_id)
);
CREATE UNIQUE INDEX IF NOT EXISTS ccp_active_shelf_name
ON ccp_content_shelves(workspace_id, name) WHERE archived=0;
CREATE TABLE IF NOT EXISTS ccp_content_books (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    shelf_id TEXT NOT NULL,
    name TEXT NOT NULL CHECK(length(CAST(name AS BLOB)) BETWEEN 1 AND 256),
    description TEXT NOT NULL CHECK(length(CAST(description AS BLOB)) <= 4096),
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    UNIQUE(id, workspace_id),
    UNIQUE(workspace_id, shelf_id, name),
    FOREIGN KEY(shelf_id, workspace_id)
        REFERENCES ccp_content_shelves(id, workspace_id) ON DELETE RESTRICT
);
