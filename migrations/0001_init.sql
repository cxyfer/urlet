-- ---------------------------------------------------------------------------
-- urlet schema, version 1
--
-- Conventions
--   * timestamps  : TEXT, ISO-8601 UTC, fixed width `YYYY-MM-DDTHH:MM:SS.sssZ`
--                   (fixed width keeps lexicographic comparison correct)
--   * booleans    : INTEGER 0/1
--   * enumerations: TEXT + CHECK constraint
--   * deletions   : soft delete via `deleted_at` where history matters
--   * slug/tag/email comparisons are case-insensitive (COLLATE NOCASE)
-- ---------------------------------------------------------------------------

-- == users ------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS users (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    email             TEXT    NOT NULL,
    email_normalized  TEXT    NOT NULL UNIQUE,
    password_hash     TEXT    NOT NULL,
    display_name      TEXT,
    role              TEXT    NOT NULL DEFAULT 'user'   CHECK (role   IN ('user', 'admin')),
    status            TEXT    NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'suspended')),
    email_verified_at TEXT,
    last_login_at     TEXT,
    created_at        TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL,
    deleted_at        TEXT
);
CREATE INDEX IF NOT EXISTS idx_users_role_status ON users (role, status);
CREATE INDEX IF NOT EXISTS idx_users_created_at  ON users (created_at DESC);

-- == sessions ---------------------------------------------------------------
CREATE TABLE IF NOT EXISTS sessions (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id      INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash   TEXT    NOT NULL UNIQUE,
    user_agent   TEXT,
    ip_hash      TEXT,
    created_at   TEXT    NOT NULL,
    last_seen_at TEXT    NOT NULL,
    expires_at   TEXT    NOT NULL,
    revoked_at   TEXT
);
CREATE INDEX IF NOT EXISTS idx_sessions_user    ON sessions (user_id, expires_at);
CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions (expires_at);

-- == api_tokens -------------------------------------------------------------
CREATE TABLE IF NOT EXISTS api_tokens (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id      INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name         TEXT    NOT NULL,
    token_prefix TEXT    NOT NULL,
    token_hash   TEXT    NOT NULL UNIQUE,
    scopes       TEXT    NOT NULL DEFAULT '["urls:read","urls:write"]',
    last_used_at TEXT,
    expires_at   TEXT,
    created_at   TEXT    NOT NULL,
    revoked_at   TEXT
);
CREATE INDEX IF NOT EXISTS idx_api_tokens_user ON api_tokens (user_id, created_at DESC);

-- == domains (reserved for multi-domain support) ----------------------------
CREATE TABLE IF NOT EXISTS domains (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    hostname    TEXT    NOT NULL UNIQUE,
    is_default  INTEGER NOT NULL DEFAULT 0,
    is_verified INTEGER NOT NULL DEFAULT 0,
    created_by  INTEGER REFERENCES users (id) ON DELETE SET NULL,
    created_at  TEXT    NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_domains_single_default
    ON domains (is_default) WHERE is_default = 1;

-- == urls -------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS urls (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id            INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    domain_id          INTEGER REFERENCES domains (id) ON DELETE SET NULL,
    slug               TEXT    NOT NULL COLLATE NOCASE UNIQUE,
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
    deleted_at         TEXT
);
CREATE INDEX IF NOT EXISTS idx_urls_user_created ON urls (user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_urls_created      ON urls (created_at DESC);
CREATE INDEX IF NOT EXISTS idx_urls_status       ON urls (status);
CREATE INDEX IF NOT EXISTS idx_urls_expires
    ON urls (expires_at) WHERE expires_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_urls_domain       ON urls (domain_id);

-- == tags -------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS tags (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id    INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name       TEXT    NOT NULL COLLATE NOCASE,
    color      TEXT,
    created_at TEXT    NOT NULL,
    UNIQUE (user_id, name)
);

CREATE TABLE IF NOT EXISTS url_tags (
    url_id INTEGER NOT NULL REFERENCES urls (id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    PRIMARY KEY (url_id, tag_id)
);
CREATE INDEX IF NOT EXISTS idx_url_tags_tag ON url_tags (tag_id);

-- == clicks -----------------------------------------------------------------
CREATE TABLE IF NOT EXISTS clicks (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    url_id             INTEGER NOT NULL REFERENCES urls (id) ON DELETE CASCADE,
    clicked_at         TEXT    NOT NULL,
    ip_hash            TEXT,
    visitor_hash       TEXT,
    user_agent         TEXT,
    referrer           TEXT,
    referrer_host      TEXT,
    country_code       TEXT,
    device_type        TEXT,
    browser            TEXT,
    os                 TEXT,
    is_bot             INTEGER NOT NULL DEFAULT 0,
    is_unique          INTEGER NOT NULL DEFAULT 0,
    password_protected INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_clicks_url_time ON clicks (url_id, clicked_at DESC);
CREATE INDEX IF NOT EXISTS idx_clicks_time     ON clicks (clicked_at);

-- daily rollup, kept in sync on every recorded click (fast dashboards)
CREATE TABLE IF NOT EXISTS url_daily_stats (
    url_id        INTEGER NOT NULL REFERENCES urls (id) ON DELETE CASCADE,
    day           TEXT    NOT NULL,
    clicks        INTEGER NOT NULL DEFAULT 0,
    unique_clicks INTEGER NOT NULL DEFAULT 0,
    bot_clicks    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (url_id, day)
);
CREATE INDEX IF NOT EXISTS idx_url_daily_stats_day ON url_daily_stats (day);

-- == audit ------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS audit_logs (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    actor_user_id INTEGER REFERENCES users (id) ON DELETE SET NULL,
    actor_role    TEXT,
    action        TEXT    NOT NULL,
    target_type   TEXT,
    target_id     TEXT,
    metadata      TEXT,
    ip_hash       TEXT,
    created_at    TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_created ON audit_logs (created_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_actor   ON audit_logs (actor_user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_target  ON audit_logs (target_type, target_id);
CREATE INDEX IF NOT EXISTS idx_audit_action  ON audit_logs (action, created_at DESC);

-- == settings ---------------------------------------------------------------
CREATE TABLE IF NOT EXISTS settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    updated_by INTEGER REFERENCES users (id) ON DELETE SET NULL
);

-- == invitations ------------------------------------------------------------
CREATE TABLE IF NOT EXISTS invite_codes (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    code_hash   TEXT    NOT NULL UNIQUE,
    code_prefix TEXT    NOT NULL,
    note        TEXT,
    created_by  INTEGER REFERENCES users (id) ON DELETE SET NULL,
    max_uses    INTEGER NOT NULL DEFAULT 1 CHECK (max_uses > 0),
    used_count  INTEGER NOT NULL DEFAULT 0,
    expires_at  TEXT,
    status      TEXT    NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'revoked')),
    created_at  TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_invites_prefix ON invite_codes (code_prefix);
CREATE INDEX IF NOT EXISTS idx_invites_status ON invite_codes (status);

CREATE TABLE IF NOT EXISTS invite_redemptions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    invite_id   INTEGER NOT NULL REFERENCES invite_codes (id) ON DELETE CASCADE,
    user_id     INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    redeemed_at TEXT    NOT NULL,
    UNIQUE (invite_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_invite_redemptions_user ON invite_redemptions (user_id);

-- == login attempts (brute force forensics) ---------------------------------
CREATE TABLE IF NOT EXISTS login_attempts (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    identifier_hash TEXT    NOT NULL,
    ip_hash         TEXT,
    success         INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_login_attempts_identifier
    ON login_attempts (identifier_hash, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_login_attempts_created
    ON login_attempts (created_at DESC);

-- == default settings -------------------------------------------------------
INSERT OR IGNORE INTO settings (key, value, updated_at, updated_by) VALUES
    ('site_name',            'urlet', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('registration_mode',    'open',   strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('default_slug_length',  '7',      strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('max_urls_per_user',    '1000',   strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('default_redirect_type','302',    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('analytics_enabled',    'true',   strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('click_retention_days', '0',      strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);
