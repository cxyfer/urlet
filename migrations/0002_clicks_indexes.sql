-- ---------------------------------------------------------------------------
-- urlet schema, version 2
--
-- Supports the click recording path: unique-visitor detection and referrer
-- breakdowns both filter on columns that were not indexed in v1.
-- ---------------------------------------------------------------------------

CREATE INDEX IF NOT EXISTS idx_clicks_visitor ON clicks (url_id, visitor_hash);
CREATE INDEX IF NOT EXISTS idx_clicks_referrer ON clicks (url_id, referrer_host);
CREATE INDEX IF NOT EXISTS idx_clicks_bot ON clicks (url_id, is_bot);
