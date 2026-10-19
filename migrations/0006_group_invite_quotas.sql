-- ---------------------------------------------------------------------------
-- urlet schema, version 6
--
-- Per-group invite issuance quotas.
--
--   * user_groups.max_invites: how many invite codes the group may issue per
--     period. NULL inherits the site-wide `max_invites_per_period` setting and
--     0 means unlimited, matching the existing quota convention. The default is
--     NULL and the site default is 0, so an upgrade changes no behaviour.
--   * user_groups.invite_period: calendar period used for the counter, one of
--     day / week (Monday start) / month, evaluated in UTC.
--   * invite_issuance: one row per issued invite code. Counting these rows makes
--     the quota atomic and means revoking a code does not hand the slot back.
--     `invite_id` is nullable so the ledger survives (unlikely) hard deletes;
--     rows are removed with their group.
--
-- Invite codes that are not pinned to a group consume the site-wide quota, so
-- the same period settings are stored in `settings` for them.
-- ---------------------------------------------------------------------------

ALTER TABLE user_groups
    ADD COLUMN max_invites INTEGER CHECK (max_invites IS NULL OR max_invites >= 0);

ALTER TABLE user_groups
    ADD COLUMN invite_period TEXT NOT NULL DEFAULT 'month'
        CHECK (invite_period IN ('day', 'week', 'month'));

CREATE TABLE IF NOT EXISTS invite_issuance (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id   INTEGER REFERENCES user_groups (id) ON DELETE CASCADE,
    actor_id   INTEGER REFERENCES users (id) ON DELETE SET NULL,
    invite_id  INTEGER REFERENCES invite_codes (id) ON DELETE SET NULL,
    created_at TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_invite_issuance_group_time
    ON invite_issuance (group_id, created_at);

INSERT OR IGNORE INTO settings (key, value, updated_at, updated_by)
VALUES ('max_invites_per_period', '0', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);

INSERT OR IGNORE INTO settings (key, value, updated_at, updated_by)
VALUES ('invite_period', 'month', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);
