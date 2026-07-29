-- Accounts, sessions, settings, and the append-only audit trail.
--
-- Conventions used throughout Elrond's schema:
--   * Identifiers are UUIDv7 stored as 16-byte BLOBs. Time-ordered keys keep
--     B-tree inserts at the right-hand edge of the index instead of scattering
--     them, which matters once the library holds tens of thousands of rows.
--   * Timestamps are RFC 3339 TEXT in UTC. Text sorts lexicographically in the
--     same order it sorts chronologically, so range scans work without a
--     conversion function, and the values stay legible in a raw dump.
--   * Tables are STRICT so SQLite rejects a mistyped bind instead of silently
--     coercing it.
--   * Enumerations carry CHECK constraints mirroring the Rust enums, so a bug in
--     the mapping layer surfaces as a constraint violation rather than as
--     unreadable data.

CREATE TABLE users (
    id            BLOB    PRIMARY KEY NOT NULL,
    email         TEXT    NOT NULL,
    display_name  TEXT    NOT NULL,
    role          TEXT    NOT NULL CHECK (role IN ('viewer', 'reviewer', 'editor', 'admin')),
    password_hash TEXT    NOT NULL,
    is_active     INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at    TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL
) STRICT;

-- Addresses are normalized to lowercase before they reach the database, so a
-- plain unique index is enough to prevent case-variant duplicate accounts.
CREATE UNIQUE INDEX users_email_key ON users (email);

CREATE TABLE sessions (
    id                BLOB PRIMARY KEY NOT NULL,
    user_id           BLOB NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Only the fingerprint of the bearer token is stored. A database dump
    -- therefore yields no usable session cookies.
    token_fingerprint TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    last_seen_at      TEXT NOT NULL,
    expires_at        TEXT NOT NULL
) STRICT;

CREATE UNIQUE INDEX sessions_token_fingerprint_key ON sessions (token_fingerprint);
CREATE INDEX sessions_user_id_idx ON sessions (user_id);
CREATE INDEX sessions_expires_at_idx ON sessions (expires_at);

CREATE TABLE app_settings (
    key        TEXT PRIMARY KEY NOT NULL,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE audit_events (
    id           BLOB PRIMARY KEY NOT NULL,
    occurred_at  TEXT NOT NULL,
    -- Intentionally NOT a foreign key. An audit record must outlive the account
    -- it refers to, and any ON DELETE action would have to mutate this table,
    -- which the append-only triggers below forbid.
    actor_user_id BLOB,
    actor_label  TEXT NOT NULL,
    action       TEXT NOT NULL,
    subject_type TEXT NOT NULL,
    subject_id   TEXT,
    -- JSON object. Must never contain credentials or document contents.
    detail       TEXT NOT NULL DEFAULT '{}'
) STRICT;

CREATE INDEX audit_events_occurred_at_idx ON audit_events (occurred_at);
CREATE INDEX audit_events_subject_idx ON audit_events (subject_type, subject_id);
CREATE INDEX audit_events_actor_idx ON audit_events (actor_user_id);

-- Append-only is enforced in the database rather than only in Rust, so a future
-- maintenance script or an unrelated code path cannot quietly rewrite history.
CREATE TRIGGER audit_events_forbid_update
BEFORE UPDATE ON audit_events
BEGIN
    SELECT RAISE(ABORT, 'audit_events is append-only');
END;

CREATE TRIGGER audit_events_forbid_delete
BEFORE DELETE ON audit_events
BEGIN
    SELECT RAISE(ABORT, 'audit_events is append-only');
END;
