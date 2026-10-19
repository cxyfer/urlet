-- ---------------------------------------------------------------------------
-- urlet schema, version 8
--
-- Member self-service lists and counts invite codes by creator, so index the
-- column the paginated queries filter on. Also covers the ownership check in
-- `revoke_owned`.
-- ---------------------------------------------------------------------------

CREATE INDEX IF NOT EXISTS idx_invite_codes_created_by
    ON invite_codes (created_by);
