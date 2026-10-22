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
