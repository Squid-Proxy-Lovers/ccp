-- Cephalopod Coordination Protocol
-- Copyright (C) 2026 Squid Proxy Lovers
-- SPDX-License-Identifier: AGPL-3.0-or-later

CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER NOT NULL,
    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sessions (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    owner TEXT NOT NULL DEFAULT '',
    labels TEXT NOT NULL DEFAULT '',
    visibility TEXT NOT NULL DEFAULT 'private',
    purpose TEXT NOT NULL DEFAULT '',
    is_active INTEGER NOT NULL DEFAULT 0,
    last_started_at TEXT,
    last_stopped_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS auth_tokens (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL,
    token_nonce TEXT NOT NULL DEFAULT '',
    token_value TEXT UNIQUE,
    token_hash TEXT NOT NULL UNIQUE,
    token_prefix TEXT NOT NULL,
    access_level TEXT NOT NULL CHECK (access_level IN ('read', 'read_write', 'admin')),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_used_at TEXT,
    expires_at TEXT,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_auth_tokens_session_id
ON auth_tokens(session_id);

CREATE INDEX IF NOT EXISTS idx_auth_tokens_access_level
ON auth_tokens(access_level);

CREATE TABLE IF NOT EXISTS shelves (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL,
    shelf_name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
    UNIQUE(session_id, shelf_name)
);

CREATE INDEX IF NOT EXISTS idx_shelves_session_id
ON shelves(session_id);

CREATE TABLE IF NOT EXISTS books (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL,
    shelf_name TEXT NOT NULL,
    book_name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
    UNIQUE(session_id, shelf_name, book_name)
);

CREATE INDEX IF NOT EXISTS idx_books_session_shelf
ON books(session_id, shelf_name);

CREATE TABLE IF NOT EXISTS message_packs (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    labels TEXT NOT NULL DEFAULT '',
    context TEXT NOT NULL,
    shelf_name TEXT NOT NULL DEFAULT 'main',
    book_name TEXT NOT NULL DEFAULT 'default',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
    UNIQUE(session_id, shelf_name, book_name, name)
);

CREATE INDEX IF NOT EXISTS idx_message_packs_session_id
ON message_packs(session_id);
CREATE INDEX IF NOT EXISTS idx_message_packs_shelf_book
ON message_packs(session_id, shelf_name, book_name);

CREATE TABLE IF NOT EXISTS deleted_message_packs (
    entry_key TEXT PRIMARY KEY,
    session_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    labels TEXT NOT NULL DEFAULT '',
    context TEXT NOT NULL,
    shelf_name TEXT NOT NULL DEFAULT 'main',
    book_name TEXT NOT NULL DEFAULT 'default',
    shelf_description TEXT NOT NULL DEFAULT '',
    book_description TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    deleted_by_client_common_name TEXT NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_deleted_message_packs_session_id
ON deleted_message_packs(session_id);

CREATE INDEX IF NOT EXISTS idx_deleted_message_packs_name
ON deleted_message_packs(name);

CREATE TABLE IF NOT EXISTS deleted_message_history (
    id INTEGER PRIMARY KEY,
    entry_key TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    client_common_name TEXT NOT NULL,
    agent_name TEXT,
    host_name TEXT,
    reason TEXT,
    appended_content TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (entry_key) REFERENCES deleted_message_packs(entry_key) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_deleted_message_history_entry_key
ON deleted_message_history(entry_key);

CREATE TABLE IF NOT EXISTS issued_client_certs (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL,
    common_name TEXT NOT NULL UNIQUE,
    access_level TEXT NOT NULL CHECK (access_level IN ('read', 'read_write', 'admin')),
    cert_pem TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TEXT NOT NULL,
    revoked_at TEXT,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_issued_client_certs_session_id
ON issued_client_certs(session_id);

CREATE TABLE IF NOT EXISTS message_history (
    id INTEGER PRIMARY KEY,
    message_pack_id INTEGER NOT NULL,
    operation_id TEXT NOT NULL DEFAULT '',
    client_common_name TEXT NOT NULL,
    agent_name TEXT,
    host_name TEXT,
    reason TEXT,
    appended_content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (message_pack_id) REFERENCES message_packs(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_message_history_message_pack_id
ON message_history(message_pack_id);

CREATE TABLE IF NOT EXISTS transfer_log (
    id            INTEGER PRIMARY KEY,
    session_id    INTEGER NOT NULL,
    direction     TEXT NOT NULL CHECK (direction IN ('export', 'import')),
    scope_json    TEXT NOT NULL,
    bundle_sha256 TEXT NOT NULL,
    policy        TEXT,
    outcome       TEXT NOT NULL,
    entry_count   INTEGER NOT NULL,
    created_at    TEXT NOT NULL,
    created_by    TEXT NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_transfer_log_session_id
ON transfer_log(session_id);

CREATE VIRTUAL TABLE IF NOT EXISTS message_packs_fts USING fts5(
    name,
    description,
    shelf_name,
    book_name,
    context,
    content = 'message_packs',
    content_rowid = 'id'
);

CREATE TRIGGER IF NOT EXISTS message_packs_ai AFTER INSERT ON message_packs BEGIN
    INSERT INTO message_packs_fts(rowid, name, description, shelf_name, book_name, context)
    VALUES (
        new.id,
        new.name,
        new.description,
        new.shelf_name,
        new.book_name,
        new.context
    );
END;

CREATE TRIGGER IF NOT EXISTS message_packs_ad AFTER DELETE ON message_packs BEGIN
    INSERT INTO message_packs_fts(message_packs_fts, rowid, name, description, shelf_name, book_name, context)
    VALUES (
        'delete',
        old.id,
        old.name,
        old.description,
        old.shelf_name,
        old.book_name,
        old.context
    );
END;

CREATE TRIGGER IF NOT EXISTS message_packs_au AFTER UPDATE ON message_packs BEGIN
    INSERT INTO message_packs_fts(message_packs_fts, rowid, name, description, shelf_name, book_name, context)
    VALUES (
        'delete',
        old.id,
        old.name,
        old.description,
        old.shelf_name,
        old.book_name,
        old.context
    );
    INSERT INTO message_packs_fts(rowid, name, description, shelf_name, book_name, context)
    VALUES (
        new.id,
        new.name,
        new.description,
        new.shelf_name,
        new.book_name,
        new.context
    );
END;
