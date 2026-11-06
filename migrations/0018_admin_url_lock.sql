-- 管理員停用連結時的鎖定時間（UTC ISO 字串）：非 NULL 表示擁有者不能自行重新啟用。
ALTER TABLE urls ADD COLUMN admin_disabled_at TEXT;

-- 回填：升級前由管理員停用的連結。
-- 判斷依據是 audit_logs 中該連結最後一筆「帶有 status 的管理員事件」
-- （action = 'admin.url_updated'，target_id 是文字形式的連結 id，metadata.status 為
-- 'active' 或 'disabled'）。最後一筆若是 'disabled'，且連結目前仍是 disabled，
-- 就以該事件時間上鎖；最後一筆若是 'active'，或根本沒有管理員事件，則不上鎖。
-- 擁有者的 'url.updated' 事件只記錄「有改 status」而不記錄值，而舊版編輯表單每次都會
-- 送出 status，所以擁有者之後的事件無法證明管理員的處置已被撤銷；這裡刻意忽略它們，
-- 寧可多鎖（管理員啟用即可解鎖），也不讓管理員的停用失效。
-- audit_logs 只保留 180 天（每日維護寫死，與 click_retention_days 無關），更早之前的
-- 管理員停用無從回填，這類連結維持未鎖定；管理員對它再停用一次即可補上鎖。
UPDATE urls
   SET admin_disabled_at = (
        SELECT a.created_at
          FROM audit_logs a
         WHERE a.action = 'admin.url_updated'
           AND a.target_type = 'url'
           AND a.target_id = CAST(urls.id AS TEXT)
           AND CASE WHEN json_valid(a.metadata)
                    THEN json_extract(a.metadata, '$.status') END = 'disabled'
           AND a.id = (
                SELECT MAX(b.id)
                  FROM audit_logs b
                 WHERE b.action = 'admin.url_updated'
                   AND b.target_type = 'url'
                   AND b.target_id = a.target_id
                   AND CASE WHEN json_valid(b.metadata)
                            THEN json_extract(b.metadata, '$.status') END IN ('active', 'disabled')
           )
   )
 WHERE status = 'disabled' AND deleted_at IS NULL;
