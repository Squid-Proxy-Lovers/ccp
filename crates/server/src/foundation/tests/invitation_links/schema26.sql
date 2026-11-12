-- Historical schema26 table, copied from60fb3b5 to exercise the actual old constraint.
CREATE TABLE IF NOT EXISTS ccp_invitations (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    kind TEXT NOT NULL CHECK (kind IN ('initial_group', 'individual')),
    role TEXT NOT NULL CHECK (role IN ('read', 'read_write')),
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    issuer_kind TEXT NOT NULL CHECK (issuer_kind IN ('admin', 'admin_key')),
    issuer_id TEXT NOT NULL,
    max_uses INTEGER CHECK (max_uses > 0),
    uses INTEGER NOT NULL DEFAULT 0 CHECK (uses >= 0 AND (max_uses IS NULL OR uses <= max_uses)),
    expires_at INTEGER CHECK (expires_at >= 0),
    revoked_at INTEGER CHECK (revoked_at >= 0),
    CHECK (max_uses IS NOT NULL OR expires_at IS NOT NULL),
    CHECK (kind != 'individual' OR (max_uses IS NOT NULL AND max_uses = 1 AND expires_at IS NOT NULL)),
    UNIQUE (id, workspace_id)
);
