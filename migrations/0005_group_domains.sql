-- ---------------------------------------------------------------------------
-- urlet schema, version 5
--
-- Per-group domain policy.
--
--   * user_groups.default_domain_id: the domain a new link uses when the client
--     omits `domain`. NULL means the site-wide default; deletion falls back to
--     NULL.
--   * user_groups.restrict_domains: when 1, only rows in group_domains (plus the
--     group default) may be selected. 0 keeps the old behaviour: every active
--     domain is selectable.
--   * group_domains: the explicit allowlist. Existing groups get no rows and
--     restrict_domains = 0, so an upgrade changes no behaviour.
-- ---------------------------------------------------------------------------

ALTER TABLE user_groups
    ADD COLUMN default_domain_id INTEGER REFERENCES domains (id) ON DELETE SET NULL;

ALTER TABLE user_groups
    ADD COLUMN restrict_domains INTEGER NOT NULL DEFAULT 0
        CHECK (restrict_domains IN (0, 1));

CREATE TABLE IF NOT EXISTS group_domains (
    group_id  INTEGER NOT NULL REFERENCES user_groups (id) ON DELETE CASCADE,
    domain_id INTEGER NOT NULL REFERENCES domains (id) ON DELETE CASCADE,
    PRIMARY KEY (group_id, domain_id)
);

CREATE INDEX IF NOT EXISTS idx_group_domains_domain ON group_domains (domain_id);
