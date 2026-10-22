-- Explicitly public SNS metadata; uploaded raster images stay in SQLite.
CREATE TABLE url_previews (
    url_id INTEGER PRIMARY KEY REFERENCES urls(id) ON DELETE CASCADE,
    title TEXT,
    description TEXT,
    image_url TEXT,
    image_key TEXT UNIQUE,
    image_mime TEXT,
    image_data BLOB,
    CHECK (image_url IS NULL OR image_key IS NULL),
    CHECK ((image_key IS NULL AND image_mime IS NULL AND image_data IS NULL)
        OR (image_key IS NOT NULL AND image_mime IS NOT NULL AND image_data IS NOT NULL))
);

-- 刪除連結時釋放上傳圖片，也涵蓋刪除帳號時的批次軟刪除。
CREATE TRIGGER url_previews_delete_with_link
AFTER UPDATE OF deleted_at ON urls
WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL
BEGIN
    DELETE FROM url_previews WHERE url_id = NEW.id;
END;
