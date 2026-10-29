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
CREATE TABLE IF NOT EXISTS ccp_admin_keys (
    id TEXT PRIMARY KEY,
    admin_id TEXT NOT NULL REFERENCES ccp_admin_principals(id) ON DELETE RESTRICT,
    workspace_id TEXT REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    expires_at INTEGER CHECK (expires_at >= 0),
    revoked_at INTEGER CHECK (revoked_at >= 0)
);
CREATE TABLE IF NOT EXISTS ccp_access_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    actor_kind TEXT NOT NULL CHECK (actor_kind IN ('local_operator', 'admin', 'credential')),
    actor_id TEXT NOT NULL,
    action TEXT NOT NULL,
    workspace_id TEXT,
    target_id TEXT NOT NULL,
    recorded_at INTEGER NOT NULL CHECK (recorded_at >= 0),
    admin_key_id TEXT REFERENCES ccp_admin_keys(id) ON DELETE RESTRICT
);
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
CREATE TABLE IF NOT EXISTS ccp_initial_enrollment (
    workspace_id TEXT PRIMARY KEY REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    invitation_id TEXT UNIQUE,
    closed_at INTEGER CHECK (closed_at >= 0),
    CHECK (invitation_id IS NOT NULL OR closed_at IS NOT NULL),
    FOREIGN KEY (invitation_id, workspace_id)
        REFERENCES ccp_invitations(id, workspace_id) ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS ccp_enrollment_events (
    id INTEGER PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    event TEXT NOT NULL CHECK (event IN ('opened', 'manual_close', 'deadline_close', 'capacity_close')),
    recorded_at INTEGER NOT NULL CHECK (recorded_at >= 0)
);
CREATE UNIQUE INDEX IF NOT EXISTS ccp_grant_workspace_identity ON ccp_grants(id, workspace_id);
CREATE TABLE IF NOT EXISTS ccp_credential_certificates (
    credential_id TEXT PRIMARY KEY REFERENCES ccp_credentials(id) ON DELETE RESTRICT,
    public_key_hash BLOB NOT NULL CHECK (length(public_key_hash) = 32),
    certificate_fingerprint BLOB NOT NULL UNIQUE CHECK (length(certificate_fingerprint) = 32),
    certificate_pem TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS ccp_invitation_receipts (
    invitation_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    public_key_hash BLOB NOT NULL CHECK (length(public_key_hash) = 32),
    previous_credential_id TEXT,
    client_id TEXT NOT NULL REFERENCES ccp_clients(id) ON DELETE RESTRICT,
    credential_id TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    recorded_at INTEGER NOT NULL CHECK (recorded_at >= 0),
    PRIMARY KEY (invitation_id, request_id),
    FOREIGN KEY (invitation_id, workspace_id) REFERENCES ccp_invitations(id, workspace_id) ON DELETE RESTRICT,
    FOREIGN KEY (credential_id, client_id) REFERENCES ccp_credentials(id, client_id) ON DELETE RESTRICT,
    FOREIGN KEY (previous_credential_id, client_id) REFERENCES ccp_credentials(id, client_id) ON DELETE RESTRICT,
    FOREIGN KEY (grant_id, client_id) REFERENCES ccp_grants(id, client_id) ON DELETE RESTRICT,
    FOREIGN KEY (grant_id, workspace_id) REFERENCES ccp_grants(id, workspace_id) ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS ccp_invitation_closures (
    invitation_id TEXT PRIMARY KEY REFERENCES ccp_invitations(id) ON DELETE RESTRICT,
    expired_at INTEGER NOT NULL CHECK (expired_at >= 0)
);

-- Supersession is distinct from administrative revocation. Old credentials may
-- retrieve only their exact renewal receipt; they cannot authorize data calls.
CREATE TABLE IF NOT EXISTS ccp_credential_renewals (
    previous_credential_id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL,
    credential_id TEXT NOT NULL UNIQUE,
    client_id TEXT NOT NULL,
    public_key_hash BLOB NOT NULL CHECK (length(public_key_hash) = 32),
    recorded_at INTEGER NOT NULL CHECK (recorded_at >= 0),
    CHECK (previous_credential_id != credential_id),
    UNIQUE (previous_credential_id, client_id),
    FOREIGN KEY (previous_credential_id, client_id)
        REFERENCES ccp_credentials(id, client_id) ON DELETE RESTRICT,
    FOREIGN KEY (credential_id, client_id)
        REFERENCES ccp_credentials(id, client_id) ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS ccp_renewal_grants (
    previous_credential_id TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    PRIMARY KEY (previous_credential_id, grant_id),
    FOREIGN KEY (previous_credential_id, client_id)
        REFERENCES ccp_credential_renewals(previous_credential_id, client_id) ON DELETE RESTRICT,
    FOREIGN KEY (grant_id, client_id)
        REFERENCES ccp_grants(id, client_id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS ccp_recovery_invitations (
    id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES ccp_clients(id) ON DELETE RESTRICT,
    workspace_id TEXT REFERENCES ccp_workspaces(id) ON DELETE RESTRICT,
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    issuer_kind TEXT NOT NULL CHECK (issuer_kind IN ('admin', 'admin_key')),
    issuer_id TEXT NOT NULL,
    expires_at INTEGER NOT NULL CHECK (expires_at >= 0),
    revoked_at INTEGER CHECK (revoked_at >= 0),
    expired_at INTEGER CHECK (expired_at >= 0),
    UNIQUE (id, client_id)
);
CREATE TABLE IF NOT EXISTS ccp_recovery_grants (
    invitation_id TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    PRIMARY KEY (invitation_id, grant_id),
    FOREIGN KEY (invitation_id, client_id)
        REFERENCES ccp_recovery_invitations(id, client_id) ON DELETE RESTRICT,
    FOREIGN KEY (grant_id, client_id)
        REFERENCES ccp_grants(id, client_id) ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS ccp_binding_revocations (
    credential_id TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    revoked_at INTEGER NOT NULL CHECK (revoked_at >= 0),
    recovery_invitation_id TEXT NOT NULL REFERENCES ccp_recovery_invitations(id) ON DELETE RESTRICT,
    PRIMARY KEY (credential_id, grant_id),
    FOREIGN KEY (credential_id, grant_id)
        REFERENCES ccp_credential_grants(credential_id, grant_id) ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS ccp_recovery_receipts (
    invitation_id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL,
    credential_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    public_key_hash BLOB NOT NULL CHECK (length(public_key_hash) = 32),
    recorded_at INTEGER NOT NULL CHECK (recorded_at >= 0),
    UNIQUE (invitation_id, credential_id),
    FOREIGN KEY (invitation_id, client_id)
        REFERENCES ccp_recovery_invitations(id, client_id) ON DELETE RESTRICT,
    FOREIGN KEY (credential_id, client_id)
        REFERENCES ccp_credentials(id, client_id) ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS ccp_recovery_receipt_grants (
    invitation_id TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    credential_id TEXT NOT NULL,
    PRIMARY KEY (invitation_id, grant_id),
    FOREIGN KEY (invitation_id, credential_id)
        REFERENCES ccp_recovery_receipts(invitation_id, credential_id) ON DELETE RESTRICT,
    FOREIGN KEY (credential_id, grant_id)
        REFERENCES ccp_credential_grants(credential_id, grant_id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS ccp_readmission_targets (
    invitation_id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES ccp_clients(id) ON DELETE RESTRICT,
    workspace_id TEXT NOT NULL,
    UNIQUE (invitation_id, client_id),
    UNIQUE (invitation_id, workspace_id),
    FOREIGN KEY (invitation_id, workspace_id)
        REFERENCES ccp_invitations(id, workspace_id) ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS ccp_readmission_history (
    invitation_id TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    PRIMARY KEY (invitation_id, grant_id),
    FOREIGN KEY (invitation_id, client_id)
        REFERENCES ccp_readmission_targets(invitation_id, client_id) ON DELETE RESTRICT,
    FOREIGN KEY (invitation_id, workspace_id)
        REFERENCES ccp_readmission_targets(invitation_id, workspace_id) ON DELETE RESTRICT,
    FOREIGN KEY (grant_id, client_id)
        REFERENCES ccp_grants(id, client_id) ON DELETE RESTRICT,
    FOREIGN KEY (grant_id, workspace_id)
        REFERENCES ccp_grants(id, workspace_id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS ccp_runtime_mode (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    mode TEXT NOT NULL CHECK (mode IN ('native', 'workspace'))
);
INSERT OR IGNORE INTO ccp_runtime_mode(singleton, mode) VALUES (1, 'native');

-- Public pin for the independently stored private instance TLS bundle.
CREATE TABLE IF NOT EXISTS ccp_instance_tls (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    instance_id TEXT NOT NULL UNIQUE REFERENCES ccp_instance(instance_id) ON DELETE RESTRICT,
    bundle_hash BLOB NOT NULL CHECK (length(bundle_hash) = 32),
    ca_fingerprint BLOB NOT NULL CHECK (length(ca_fingerprint) = 32)
);

-- Local-only provenance for a first-boot instance with no owner yet (schema26).
CREATE TABLE IF NOT EXISTS ccp_setup_reservation (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    instance_id TEXT NOT NULL UNIQUE REFERENCES ccp_instance(instance_id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    claimed_at INTEGER CHECK (claimed_at >= 0)
);
