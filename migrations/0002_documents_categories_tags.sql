-- Documents, immutable versions, the category tree, tags, and full-text search.
--
-- Follows the conventions established in 0001: UUIDv7 blobs for identifiers,
-- RFC 3339 UTC text for timestamps, STRICT tables, and CHECK constraints
-- mirroring the Rust enums.

-- ------------------------------------------------------------------ categories

CREATE TABLE categories (
    id         BLOB    PRIMARY KEY NOT NULL,
    parent_id  BLOB    REFERENCES categories (id) ON DELETE RESTRICT,
    name       TEXT    NOT NULL,
    -- Lowercased name, so sibling uniqueness is case-insensitive and the ZIP
    -- importer reuses a matching folder rather than creating a near-duplicate.
    name_key   TEXT    NOT NULL,
    position   INTEGER NOT NULL DEFAULT 0,
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,

    -- The cheapest cycle to create is a one-node loop, so it is refused here as
    -- well as in the domain. Longer cycles need the whole tree to detect and are
    -- checked by CategoryTree before any write.
    CHECK (parent_id IS NULL OR parent_id <> id)
) STRICT;

-- Two partial indexes rather than one on (parent_id, name_key): SQLite treats
-- NULLs as distinct in a unique index, so a single index would let unlimited
-- root categories share a name.
CREATE UNIQUE INDEX categories_sibling_name_key
    ON categories (parent_id, name_key)
    WHERE parent_id IS NOT NULL;

CREATE UNIQUE INDEX categories_root_name_key
    ON categories (name_key)
    WHERE parent_id IS NULL;

CREATE INDEX categories_parent_idx ON categories (parent_id, position);

-- ------------------------------------------------------------------------ tags

CREATE TABLE tags (
    id         BLOB PRIMARY KEY NOT NULL,
    label      TEXT NOT NULL,
    -- Lowercased label. Uniqueness is on this, so "Board Minutes" and
    -- "board minutes" are one tag rather than two that look identical in a list.
    label_key  TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;

CREATE UNIQUE INDEX tags_label_key ON tags (label_key);

-- ------------------------------------------------------------------- documents

CREATE TABLE documents (
    id                 BLOB    PRIMARY KEY NOT NULL,
    title              TEXT    NOT NULL,
    -- RESTRICT rather than CASCADE: deleting a category must never silently take
    -- its documents with it. The application moves or refuses instead.
    category_id        BLOB    NOT NULL REFERENCES categories (id) ON DELETE RESTRICT,
    lifecycle          TEXT    NOT NULL
        CHECK (lifecycle IN ('draft', 'in_review', 'published', 'superseded', 'archived')),
    -- Deliberately not a foreign key. documents and document_versions reference
    -- each other, and a circular constraint would make insert order depend on
    -- deferred-constraint behaviour. The application maintains this inside the
    -- same transaction as the version insert, and the CHECK below keeps it
    -- consistent with version_count.
    current_version_id BLOB,
    version_count      INTEGER NOT NULL DEFAULT 0 CHECK (version_count >= 0),
    -- Folder-relative path a bulk import came from, preserved as provenance.
    source_path        TEXT,
    review_due_at      TEXT,
    created_by         BLOB    NOT NULL,
    created_at         TEXT    NOT NULL,
    updated_at         TEXT    NOT NULL,

    CHECK ((version_count = 0) = (current_version_id IS NULL))
) STRICT;

CREATE INDEX documents_category_idx ON documents (category_id, id);
CREATE INDEX documents_lifecycle_idx ON documents (lifecycle, id);
-- Partial: only rows with a review date are ever scanned for the review queues.
CREATE INDEX documents_review_due_idx ON documents (review_due_at)
    WHERE review_due_at IS NOT NULL;

-- ----------------------------------------------------------------- versions

CREATE TABLE document_versions (
    id                  BLOB    PRIMARY KEY NOT NULL,
    document_id         BLOB    NOT NULL REFERENCES documents (id) ON DELETE CASCADE,
    number              INTEGER NOT NULL CHECK (number >= 1),
    -- Display label only. It never influences a storage path; storage_key is
    -- derived from the content checksum.
    original_filename   TEXT    NOT NULL,
    media_type          TEXT    NOT NULL,
    byte_size           INTEGER NOT NULL CHECK (byte_size >= 0),
    checksum            TEXT    NOT NULL,
    storage_key         TEXT    NOT NULL,
    derivative_checksum TEXT,
    derivative_key      TEXT,
    created_by          BLOB    NOT NULL,
    note                TEXT,
    created_at          TEXT    NOT NULL,

    -- A derivative has both parts or neither.
    CHECK ((derivative_key IS NULL) = (derivative_checksum IS NULL))
) STRICT;

CREATE UNIQUE INDEX document_versions_sequence_key
    ON document_versions (document_id, number);

-- Deduplication looks content up by checksum before writing a new blob.
CREATE INDEX document_versions_checksum_idx ON document_versions (checksum);
CREATE INDEX document_versions_derivative_idx ON document_versions (derivative_checksum)
    WHERE derivative_checksum IS NOT NULL;

-- Immutability is enforced in the database, not only in Rust. A binder release
-- pins version identifiers and must rebuild byte-identically, so a version's
-- content can never be edited in place; replacing content appends a new version.
CREATE TRIGGER document_versions_content_is_immutable
BEFORE UPDATE OF
    id, document_id, number, original_filename, media_type,
    byte_size, checksum, storage_key, created_by, created_at
ON document_versions
BEGIN
    SELECT RAISE(ABORT, 'document versions are immutable; append a new version instead');
END;

-- The generated PDF is filled in once, after conversion. Allowing it to be
-- replaced would change what an existing binder release renders.
CREATE TRIGGER document_versions_derivative_is_write_once
BEFORE UPDATE OF derivative_key, derivative_checksum ON document_versions
WHEN OLD.derivative_key IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a generated derivative cannot be replaced');
END;

-- ---------------------------------------------------------------- document tags

CREATE TABLE document_tags (
    document_id BLOB NOT NULL REFERENCES documents (id) ON DELETE CASCADE,
    tag_id      BLOB NOT NULL REFERENCES tags (id) ON DELETE CASCADE,

    PRIMARY KEY (document_id, tag_id)
) STRICT, WITHOUT ROWID;

-- The reverse direction, for "every document with this tag".
CREATE INDEX document_tags_tag_idx ON document_tags (tag_id, document_id);

-- --------------------------------------------------------------- blob registry

-- Reference counting for content-addressed blobs.
--
-- Deduplication means two versions can share one file, so a version being
-- deleted must not delete the bytes another version still points at. The count is
-- maintained in the same transaction as the version insert or delete.
CREATE TABLE blobs (
    storage_key    TEXT PRIMARY KEY NOT NULL,
    checksum       TEXT NOT NULL,
    byte_size      INTEGER NOT NULL CHECK (byte_size >= 0),
    reference_count INTEGER NOT NULL DEFAULT 0 CHECK (reference_count >= 0),
    created_at     TEXT NOT NULL
) STRICT, WITHOUT ROWID;

CREATE INDEX blobs_checksum_idx ON blobs (checksum);
-- Partial index over exactly the rows a sweeper cares about.
CREATE INDEX blobs_unreferenced_idx ON blobs (reference_count)
    WHERE reference_count = 0;

-- ------------------------------------------------------------ full-text search

-- Maintained by the application rather than by triggers, because the indexed text
-- is assembled from four tables plus extracted document content that arrives
-- asynchronously after conversion.
--
-- `unicode61 remove_diacritics 2` folds accents, so "resume" finds "résumé".
-- `prefix '2 3'` builds prefix indexes so type-ahead search does not have to
-- fall back to a full scan.
CREATE VIRTUAL TABLE documents_fts USING fts5(
    document_id UNINDEXED,
    title,
    filename,
    tags,
    content,
    tokenize = "unicode61 remove_diacritics 2",
    prefix = "2 3"
);
