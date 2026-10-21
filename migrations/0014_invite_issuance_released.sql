-- ---------------------------------------------------------------------------
-- urlet schema, version 14
--
-- Release an invite-quota slot when an unused code is revoked.
--
--   * invite_issuance.released_at marks a ledger row that no longer counts
--     towards the group's quota. The row itself stays, so the issuance history
--     is preserved.
--   * Only a code with used_count = 0 is released. A revoked code that already
--     admitted someone keeps consuming its slot; otherwise a member could
--     create, redeem and revoke codes to exceed the per-period limit.
--   * Already-revoked, never-redeemed codes from before this migration are
--     released too, so the counter matches the new rule immediately. The
--     release timestamp is the upgrade time, not the original revocation.
-- ---------------------------------------------------------------------------

ALTER TABLE invite_issuance ADD COLUMN released_at TEXT;

UPDATE invite_issuance
   SET released_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
 WHERE released_at IS NULL
   AND invite_id IN (
           SELECT id FROM invite_codes
            WHERE status = 'revoked' AND used_count = 0
       );
