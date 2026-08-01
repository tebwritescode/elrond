CREATE TABLE document_tags (
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    tag TEXT NOT NULL COLLATE NOCASE,
    PRIMARY KEY (document_id, tag)
);

CREATE UNIQUE INDEX idx_categories_root_name_nocase
    ON categories(name COLLATE NOCASE)
    WHERE parent_id IS NULL;

CREATE UNIQUE INDEX idx_categories_sibling_name_nocase
    ON categories(parent_id, name COLLATE NOCASE)
    WHERE parent_id IS NOT NULL;

CREATE INDEX idx_document_tags_document ON document_tags(document_id, tag COLLATE NOCASE);
