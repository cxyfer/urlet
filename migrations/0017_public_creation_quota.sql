-- 遊客自然週期配額：來源只存雜湊，重啟不會清空尚未到期的計數。
CREATE TABLE public_creation_quota (
    scope TEXT NOT NULL,
    period TEXT NOT NULL CHECK (period IN ('minute', 'hour', 'day', 'week', 'month')),
    window_start INTEGER NOT NULL,
    window_end INTEGER NOT NULL,
    used INTEGER NOT NULL CHECK (used > 0),
    PRIMARY KEY (scope, period, window_start)
);
CREATE INDEX idx_public_creation_quota_end ON public_creation_quota(window_end);

INSERT OR IGNORE INTO settings (key, value, updated_at)
VALUES ('public_rate_limit_per_ip_period', 'hour', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
       ('public_rate_limit_global_period', 'hour', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
