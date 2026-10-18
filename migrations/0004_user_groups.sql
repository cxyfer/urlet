-- ---------------------------------------------------------------------------
-- urlet schema, version 4
--
-- User groups: per-group link/API-token quotas and feature toggles. A NULL
-- quota inherits the site-wide setting, 0 means unlimited. The seeded default
-- group has both quotas NULL and every toggle enabled, so an upgrade changes no
-- behaviour; every pre-existing user is moved into it.
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS user_groups (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    name              TEXT    NOT NULL COLLATE NOCASE UNIQUE,
    description       TEXT,
    max_urls          INTEGER CHECK (max_urls IS NULL OR max_urls >= 0),
    max_api_tokens    INTEGER CHECK (max_api_tokens IS NULL OR max_api_tokens >= 0),
    allow_custom_slug INTEGER NOT NULL DEFAULT 1 CHECK (allow_custom_slug IN (0, 1)),
    allow_password    INTEGER NOT NULL DEFAULT 1 CHECK (allow_password IN (0, 1)),
    allow_expiry      INTEGER NOT NULL DEFAULT 1 CHECK (allow_expiry IN (0, 1)),
    allow_max_clicks  INTEGER NOT NULL DEFAULT 1 CHECK (allow_max_clicks IN (0, 1)),
    allow_tags        INTEGER NOT NULL DEFAULT 1 CHECK (allow_tags IN (0, 1)),
    allow_api_tokens  INTEGER NOT NULL DEFAULT 1 CHECK (allow_api_tokens IN (0, 1)),
    allow_domain_selection INTEGER NOT NULL DEFAULT 1
                              CHECK (allow_domain_selection IN (0, 1)),
    is_default        INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    created_at        TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL
);

-- At most one default group; the partial index is what enforces the invariant.
CREATE UNIQUE INDEX IF NOT EXISTS idx_user_groups_single_default
    ON user_groups (is_default) WHERE is_default = 1;

ALTER TABLE users ADD COLUMN group_id INTEGER REFERENCES user_groups (id) ON DELETE SET NULL;
ALTER TABLE invite_codes ADD COLUMN group_id INTEGER REFERENCES user_groups (id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_users_group   ON users (group_id);
CREATE INDEX IF NOT EXISTS idx_invites_group ON invite_codes (group_id);

-- The site-wide token quota is introduced together with groups. 20 is a
-- deliberate behaviour change: existing tokens are kept, but an account above
-- the limit cannot create another one until it revokes some.
INSERT OR IGNORE INTO settings (key, value, updated_at, updated_by)
VALUES ('max_api_tokens_per_user', '20', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);

-- Seed the default group and move every existing account into it. Both quotas
-- stay NULL so the site-wide settings keep applying exactly as before.
INSERT INTO user_groups
    (name, description, max_urls, max_api_tokens, allow_custom_slug, allow_password,
     allow_expiry, allow_max_clicks, allow_tags, allow_api_tokens,
     allow_domain_selection, is_default, created_at, updated_at)
VALUES
    ('Standard', NULL, NULL, NULL, 1, 1, 1, 1, 1, 1, 1, 1,
     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

UPDATE users
   SET group_id = (SELECT id FROM user_groups WHERE is_default = 1)
 WHERE group_id IS NULL;
