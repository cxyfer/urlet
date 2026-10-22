-- ---------------------------------------------------------------------------
-- urlet schema, version 15
--
-- The public shortener: an anonymous visitor may create a link from the landing
-- page, and those links need an owner because `urls.user_id` is a NOT NULL
-- foreign key.
--
--   * users.is_system marks the reserved accounts the application manages
--     itself. Exactly one such account exists (the partial unique index below
--     enforces it): the Guest account that owns every link created by an
--     anonymous visitor. System accounts are hidden from the user list, from
--     every user count and from the bootstrap-admin logic, and an administrator
--     cannot edit or delete them.
--   * The Guest account cannot be signed into: it is `suspended` (login is
--     refused and no session can be issued) and its password hash is the
--     unparsable sentinel `!`, which never verifies. Its `group_id` stays NULL
--     so the public-tool settings, not a group, decide what a visitor may do.
--   * The account is created here rather than lazily so the reserved address can
--     never be registered by a person first. `guest@urlet.invalid` uses the
--     reserved `.invalid` TLD (RFC 2606) and is only ever a label in the admin
--     UI. The guard clauses keep the upgrade healthy even in the pathological
--     case where somebody already holds that address.
-- ---------------------------------------------------------------------------

ALTER TABLE users
    ADD COLUMN is_system INTEGER NOT NULL DEFAULT 0 CHECK (is_system IN (0, 1));

CREATE UNIQUE INDEX IF NOT EXISTS idx_users_single_system
    ON users (is_system) WHERE is_system = 1;

INSERT INTO users
    (email, email_normalized, password_hash, display_name, role, status, group_id,
     is_system, created_at, updated_at)
SELECT 'guest@urlet.invalid', 'guest@urlet.invalid', '!', 'Guest', 'user', 'suspended', NULL,
       1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
 WHERE NOT EXISTS (SELECT 1 FROM users WHERE is_system = 1)
   AND NOT EXISTS (SELECT 1 FROM users WHERE email_normalized = 'guest@urlet.invalid');

-- Settings of the public shortener. The feature is off by default: an upgrade
-- keeps the previous behaviour (only signed-in accounts create links) until an
-- administrator turns the public form on.
INSERT OR IGNORE INTO settings (key, value, updated_at, updated_by) VALUES
    ('public_create_access', 'user', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('public_captcha', 'true', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('public_rate_limit_per_ip', '20', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('public_rate_limit_global', '500', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('public_allow_slug', 'true', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('public_allow_title', 'true', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('public_allow_tags', 'false', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('public_allow_password', 'false', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('public_allow_expiry', 'false', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('public_allow_max_clicks', 'false', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('public_allow_redirect', 'false', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('public_allow_domain', 'false', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);
