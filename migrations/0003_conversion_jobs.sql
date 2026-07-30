ALTER TABLE document_versions
ADD COLUMN original_media_type TEXT NOT NULL DEFAULT 'application/octet-stream';

UPDATE document_versions SET original_media_type = CASE
    WHEN lower(original_filename) LIKE '%.pdf' THEN 'application/pdf'
    WHEN lower(original_filename) LIKE '%.jpg' OR lower(original_filename) LIKE '%.jpeg' THEN 'image/jpeg'
    WHEN lower(original_filename) LIKE '%.png' THEN 'image/png'
    WHEN lower(original_filename) LIKE '%.tif' OR lower(original_filename) LIKE '%.tiff' THEN 'image/tiff'
    WHEN lower(original_filename) LIKE '%.txt' THEN 'text/plain'
    WHEN lower(original_filename) LIKE '%.docx' THEN 'application/vnd.openxmlformats-officedocument.wordprocessingml.document'
    WHEN lower(original_filename) LIKE '%.xlsx' THEN 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'
    WHEN lower(original_filename) LIKE '%.pptx' THEN 'application/vnd.openxmlformats-officedocument.presentationml.presentation'
    WHEN lower(original_filename) LIKE '%.odt' THEN 'application/vnd.oasis.opendocument.text'
    WHEN lower(original_filename) LIKE '%.ods' THEN 'application/vnd.oasis.opendocument.spreadsheet'
    WHEN lower(original_filename) LIKE '%.odp' THEN 'application/vnd.oasis.opendocument.presentation'
    ELSE original_media_type
END;

CREATE TABLE conversion_jobs (
    id TEXT PRIMARY KEY NOT NULL,
    document_version_id TEXT NOT NULL UNIQUE REFERENCES document_versions(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued', 'processing', 'completed', 'failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    available_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    lease_expires_at TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at TEXT
);

CREATE INDEX idx_conversion_jobs_claim
ON conversion_jobs(status, available_at, lease_expires_at, created_at);

INSERT INTO conversion_jobs (id, document_version_id)
SELECT lower(hex(randomblob(16))), id
FROM document_versions
WHERE pdf_storage_key IS NULL;
