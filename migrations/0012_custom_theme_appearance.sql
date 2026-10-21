-- ---------------------------------------------------------------------------
-- urlet schema, version 12
--
-- Custom themes now choose an appearance instead of a single dark-only flag:
--   auto  follow the operating system / saved preference
--   light force the light palette and hide the light/dark switch
--   dark  force the dark palette and hide the light/dark switch
--
-- The original is_dark_only column stays in place for databases created from
-- version 11; it is no longer written by the application.
-- ---------------------------------------------------------------------------

ALTER TABLE custom_themes
    ADD COLUMN appearance TEXT NOT NULL DEFAULT 'auto'
    CHECK (appearance IN ('auto', 'light', 'dark'));

UPDATE custom_themes SET appearance = 'dark' WHERE is_dark_only = 1;
