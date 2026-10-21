-- ---------------------------------------------------------------------------
-- urlet schema, version 9
--
-- Show invite codes again after creation.
--
--   * `code_hash` stays the lookup key, so redemption is still an indexed
--     hash comparison and never needs the plaintext.
--   * `code_sealed` is an authenticated, encrypted copy of the code. It lets
--     the issuer (and administrators) display the code and its registration
--     link later, without storing the plaintext in the database. The key is
--     derived from URLET_SECRET, so changing the secret makes old copies
--     unreadable; those rows fall back to `code_prefix`.
--   * This does not make a database dump confidential by itself: the lookup
--     hash and four-character prefix still allow offline guessing of a
--     ten-character code, and the auto-generated secret.key sits beside the
--     database. Protect database and data-directory backups.
--   * Existing rows keep `code_sealed = NULL`: hashed codes cannot be
--     recovered, so they stay visible by prefix only.
-- ---------------------------------------------------------------------------

ALTER TABLE invite_codes
    ADD COLUMN code_sealed TEXT;
