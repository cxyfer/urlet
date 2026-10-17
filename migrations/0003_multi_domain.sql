-- ---------------------------------------------------------------------------
-- urlet schema, version 3
--
-- Multi-domain support.
--
--   * domains gains a lifecycle (pending/active/disabled), an origin
--     (system/env/ui) and updated_at.
--   * urls.domain_id becomes mandatory and the slug namespace becomes
--     per-domain: UNIQUE (domain_id, slug).
--
-- SQLite cannot drop the inline UNIQUE on urls.slug, so the table is rebuilt.
-- The migration runner runs this file on a dedicated connection with
-- `PRAGMA foreign_keys = OFF` outside the transaction: clicks, url_tags and
-- url_daily_stats reference urls with ON DELETE CASCADE, and DROP TABLE under
-- foreign_keys=ON performs an implicit DELETE that could cascade into them.
-- ---------------------------------------------------------------------------

-- == domains ----------------------------------------------------------------
UPDATE domains SET hostname = rtrim(lower(trim(hostname)), '.');

ALTER TABLE domains ADD COLUMN status TEXT NOT NULL DEFAULT 'active'
    CHECK (status IN ('pending', 'active', 'disabled'));
ALTER TABLE domains ADD COLUMN source TEXT NOT NULL DEFAULT 'ui'
    CHECK (source IN ('system', 'env', 'ui'));
ALTER TABLE domains ADD COLUMN updated_at TEXT;

UPDATE domains SET updated_at = created_at WHERE updated_at IS NULL;

-- In v1/v2 the only domain was the canonical host seeded from URLET_BASE_URL.
-- Mark the default row as the system domain; bootstrap re-asserts this.
UPDATE domains
   SET source = 'system', status = 'active', is_verified = 1
 WHERE is_default = 1;

CREATE UNIQUE INDEX IF NOT EXISTS idx_domains_hostname_nocase
    ON domains (hostname COLLATE NOCASE);

-- == urls rebuild -----------------------------------------------------------
CREATE TABLE urls_new (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id            INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    domain_id          INTEGER NOT NULL REFERENCES domains (id) ON DELETE RESTRICT,
    slug               TEXT    NOT NULL COLLATE NOCASE,
    target_url         TEXT    NOT NULL,
    title              TEXT,
    notes              TEXT,
    password_hash      TEXT,
    expires_at         TEXT,
    max_clicks         INTEGER CHECK (max_clicks IS NULL OR max_clicks > 0),
    click_count        INTEGER NOT NULL DEFAULT 0,
    unique_click_count INTEGER NOT NULL DEFAULT 0,
    redirect_type      INTEGER NOT NULL DEFAULT 302
                       CHECK (redirect_type IN (301, 302, 307, 308)),
    status             TEXT    NOT NULL DEFAULT 'active'
                       CHECK (status IN ('active', 'disabled')),
    is_custom_slug     INTEGER NOT NULL DEFAULT 0,
    last_clicked_at    TEXT,
    created_at         TEXT    NOT NULL,
    updated_at         TEXT    NOT NULL,
    deleted_at         TEXT,
    UNIQUE (domain_id, slug)
);

INSERT INTO urls_new (
    id, user_id, domain_id, slug, target_url, title, notes, password_hash,
    expires_at, max_clicks, click_count, unique_click_count, redirect_type,
    status, is_custom_slug, last_clicked_at, created_at, updated_at, deleted_at
)
SELECT
    id, user_id,
    COALESCE(domain_id, (SELECT id FROM domains WHERE is_default = 1 LIMIT 1)),
    slug, target_url, title, notes, password_hash,
    expires_at, max_clicks, click_count, unique_click_count, redirect_type,
    status, is_custom_slug, last_clicked_at, created_at, updated_at, deleted_at
FROM urls;

DROP TABLE urls;
ALTER TABLE urls_new RENAME TO urls;

CREATE INDEX IF NOT EXISTS idx_urls_user_created ON urls (user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_urls_created      ON urls (created_at DESC);
CREATE INDEX IF NOT EXISTS idx_urls_status       ON urls (status);
CREATE INDEX IF NOT EXISTS idx_urls_expires
    ON urls (expires_at) WHERE expires_at IS NOT NULL;
