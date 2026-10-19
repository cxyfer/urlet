-- ---------------------------------------------------------------------------
-- urlet schema, version 7
--
-- Member self-service invite codes.
--
--   * user_groups.allow_invites: when 1, the group's members may issue invite
--     codes themselves. The code is always pinned to their own group, so a
--     redeemer joins that group. Default 0 keeps every existing instance
--     admin-only until an administrator opts in.
--   * The per-group `max_invites` / `invite_period` quota (migration 6) applies
--     to member-issued codes too, and both paths share one ledger: an invite an
--     administrator issued to the group consumes the same budget.
-- ---------------------------------------------------------------------------

ALTER TABLE user_groups
    ADD COLUMN allow_invites INTEGER NOT NULL DEFAULT 0
        CHECK (allow_invites IN (0, 1));
