-- ---------------------------------------------------------------------------
-- urlet schema, version 11
--
-- Splash pages (the interstitial shown before the destination).
--
--   * urls.redirect_mode: `direct` keeps the historical behaviour; `splash`
--     renders a short promotional page with a countdown instead. The default is
--     `direct`, so an upgrade changes no behaviour and existing links keep
--     redirecting immediately.
--   * urls.splash_delay_seconds: how long the splash page waits before the
--     automatic redirect, 1-60 seconds (the form and the API validate the same
--     range; the default is 5).
--   * urls.splash_title / urls.splash_message: optional per-link copy. NULL
--     means "use the localised default copy", so the templates stay translated.
--   * user_groups.redirect_mode_policy: which modes a group may use for new
--     links. `choice` (the default) lets members pick either mode, `direct`
--     only allows immediate redirects and `splash` only allows the interstitial.
--     Existing links are never modified; like the other feature toggles the
--     policy only gates creation and switching modes on update.
-- ---------------------------------------------------------------------------

ALTER TABLE urls
    ADD COLUMN redirect_mode TEXT NOT NULL DEFAULT 'direct'
        CHECK (redirect_mode IN ('direct', 'splash'));

ALTER TABLE urls
    ADD COLUMN splash_delay_seconds INTEGER NOT NULL DEFAULT 5
        CHECK (splash_delay_seconds BETWEEN 1 AND 60);

ALTER TABLE urls
    ADD COLUMN splash_title TEXT;

ALTER TABLE urls
    ADD COLUMN splash_message TEXT;

ALTER TABLE user_groups
    ADD COLUMN redirect_mode_policy TEXT NOT NULL DEFAULT 'choice'
        CHECK (redirect_mode_policy IN ('direct', 'splash', 'choice'));
