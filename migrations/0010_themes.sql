-- ---------------------------------------------------------------------------
-- urlet schema, version 10
--
-- Switching themes. A theme is compiled into the binary (templates plus a CSS
-- asset); this table decides which ones are offered, which one is the default
-- for visitors without a preference, and in which order the picker lists them.
-- The default row is protected by a partial unique index, just like the default
-- user group.
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS themes (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    key        TEXT    NOT NULL UNIQUE,
    is_enabled INTEGER NOT NULL DEFAULT 1 CHECK (is_enabled IN (0, 1)),
    is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_themes_single_default
    ON themes (is_default) WHERE is_default = 1;

-- Verdant is the design that shipped with v1.2, so it stays the default. The
-- five earlier proposals are offered as alternatives from day one; an
-- administrator can disable any of them without touching the binary.
INSERT OR IGNORE INTO themes (key, is_enabled, is_default, sort_order, created_at, updated_at)
VALUES
    ('verdant', 1, 1, 0,
     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
     strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('aurora', 1, 0, 10,
     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
     strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('paper', 1, 0, 20,
     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
     strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('signal', 1, 0, 30,
     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
     strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('gilded', 1, 0, 40,
     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
     strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('swiss', 1, 0, 50,
     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

-- Who may change their own theme: everyone (subject to the group toggle) or
-- administrators only. Administrators can always preview any enabled theme.
INSERT OR IGNORE INTO settings (key, value, updated_at, updated_by)
VALUES ('theme_switching', 'all', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);
