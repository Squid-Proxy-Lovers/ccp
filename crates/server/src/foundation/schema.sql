-- Additive identity/workspace foundations. Legacy transport tables are untouched.
CREATE TABLE IF NOT EXISTS ccp_instance (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    instance_id TEXT NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS ccp_admin_principals (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1))
);
CREATE TABLE IF NOT EXISTS ccp_server_admins (
    admin_id TEXT PRIMARY KEY REFERENCES ccp_admin_principals(id) ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS ccp_workspaces (
    id TEXT PRIMARY KEY,
    legacy_session_id INTEGER UNIQUE REFERENCES sessions(id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'archived', 'purged')),
    owner_admin_id TEXT REFERENCES ccp_admin_principals(id) ON DELETE RESTRICT
);
CREATE UNIQUE INDEX IF NOT EXISTS ccp_workspace_name
ON ccp_workspaces(name) WHERE state != 'purged' AND legacy_session_id IS NULL;
CREATE TABLE IF NOT EXISTS ccp_workspace_admins (
    workspace_id TEXT NOT NULL REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    admin_id TEXT NOT NULL REFERENCES ccp_admin_principals(id) ON DELETE RESTRICT,
    PRIMARY KEY (workspace_id, admin_id)
);
CREATE TABLE IF NOT EXISTS ccp_clients (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    disabled INTEGER NOT NULL DEFAULT 0 CHECK (disabled IN (0, 1))
);
CREATE TABLE IF NOT EXISTS ccp_credentials (
    id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES ccp_clients(id) ON DELETE RESTRICT,
    expires_at INTEGER NOT NULL CHECK (expires_at >= 0),
    revoked_at INTEGER CHECK (revoked_at >= 0),
    UNIQUE (id, client_id)
);
CREATE TABLE IF NOT EXISTS ccp_grants (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    client_id TEXT NOT NULL REFERENCES ccp_clients(id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role IN ('read', 'read_write')),
    revoked_at INTEGER CHECK (revoked_at >= 0),
    UNIQUE (id, client_id)
);
CREATE UNIQUE INDEX IF NOT EXISTS ccp_live_client_grant
ON ccp_grants(client_id, workspace_id) WHERE revoked_at IS NULL;
CREATE TABLE IF NOT EXISTS ccp_credential_grants (
    credential_id TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    PRIMARY KEY (credential_id, grant_id),
    FOREIGN KEY (credential_id, client_id)
        REFERENCES ccp_credentials(id, client_id) ON DELETE RESTRICT,
    FOREIGN KEY (grant_id, client_id)
        REFERENCES ccp_grants(id, client_id) ON DELETE RESTRICT
);
CREATE INDEX IF NOT EXISTS ccp_grants_workspace ON ccp_grants(workspace_id);
CREATE TABLE IF NOT EXISTS ccp_access_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    actor_kind TEXT NOT NULL CHECK (actor_kind IN ('local_operator', 'admin', 'credential')),
    actor_id TEXT NOT NULL,
    action TEXT NOT NULL,
    workspace_id TEXT,
    target_id TEXT NOT NULL,
    recorded_at INTEGER NOT NULL CHECK (recorded_at >= 0)
);
