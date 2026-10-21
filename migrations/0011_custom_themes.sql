-- ---------------------------------------------------------------------------
-- urlet schema, version 11
--
-- Uploaded themes. A custom theme is a stylesheet the administrator uploads;
-- it reuses the base templates and is served from the data directory, never
-- from the binary. The metadata lives here and the actual CSS file sits in
-- URLET_THEMES_DIR (default: <data dir>/themes/<key>.css), so the database and
-- the persistent volume stay in sync across restarts.
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS custom_themes (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    key               TEXT    NOT NULL UNIQUE,
    name              TEXT    NOT NULL,
    description       TEXT    NOT NULL DEFAULT '',
    is_dark_only      INTEGER NOT NULL DEFAULT 0 CHECK (is_dark_only IN (0, 1)),
    css_version       TEXT    NOT NULL,
    css_size          INTEGER NOT NULL,
    original_filename TEXT    NOT NULL DEFAULT '',
    created_by        INTEGER REFERENCES users (id) ON DELETE SET NULL,
    created_at        TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_custom_themes_created_by
    ON custom_themes (created_by);
