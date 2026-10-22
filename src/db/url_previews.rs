//! SNS 預覽的資料存取；一般 metadata 查詢不載入圖片 BLOB。
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, Default, FromRow)]
pub struct Preview {
    pub url_id: i64,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub image_key: Option<String>,
}

#[derive(Debug, Default)]
pub struct Image {
    pub url: Option<String>,
    pub key: Option<String>,
    pub mime: Option<String>,
    pub data: Option<Vec<u8>>,
}

#[derive(Debug, Default)]
pub struct Changes {
    pub title: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub image: Option<Image>,
}

pub async fn find(pool: &SqlitePool, url_id: i64) -> Result<Option<Preview>, sqlx::Error> {
    sqlx::query_as("SELECT url_id, title, description, image_url, image_key FROM url_previews WHERE url_id = ?")
        .bind(url_id).fetch_optional(pool).await
}

/// 寫入時重新檢查擁有者；文字、圖片替換與停用在同一交易完成。
pub async fn save(
    pool: &SqlitePool,
    url_id: i64,
    user_id: i64,
    changes: &Changes,
) -> Result<bool, sqlx::Error> {
    let empty_image = Image::default();
    let image = changes.image.as_ref().unwrap_or(&empty_image);
    let mut tx = pool.begin().await?;
    let result = sqlx::query(
        "INSERT INTO url_previews (url_id, title, description, image_url, image_key, image_mime, image_data)
         SELECT id, ?, ?, ?, ?, ?, ? FROM urls WHERE id = ? AND user_id = ? AND deleted_at IS NULL
         ON CONFLICT(url_id) DO UPDATE SET
         title = CASE WHEN ? THEN excluded.title ELSE url_previews.title END,
         description = CASE WHEN ? THEN excluded.description ELSE url_previews.description END,
         image_url = CASE WHEN ? THEN excluded.image_url ELSE url_previews.image_url END,
         image_key = CASE WHEN ? THEN excluded.image_key ELSE url_previews.image_key END,
         image_mime = CASE WHEN ? THEN excluded.image_mime ELSE url_previews.image_mime END,
         image_data = CASE WHEN ? THEN excluded.image_data ELSE url_previews.image_data END"
    )
    .bind(changes.title.as_ref().and_then(|v| v.as_deref()))
    .bind(changes.description.as_ref().and_then(|v| v.as_deref()))
    .bind(image.url.as_deref()).bind(image.key.as_deref()).bind(image.mime.as_deref()).bind(image.data.as_deref())
    .bind(url_id).bind(user_id).bind(changes.title.is_some()).bind(changes.description.is_some())
    .bind(changes.image.is_some()).bind(changes.image.is_some()).bind(changes.image.is_some()).bind(changes.image.is_some())
    .execute(&mut *tx).await?;
    sqlx::query("DELETE FROM url_previews WHERE url_id = ? AND title IS NULL AND description IS NULL AND image_url IS NULL AND image_key IS NULL")
        .bind(url_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result.rows_affected() != 0)
}

#[derive(FromRow)]
pub struct StoredImage {
    pub url_id: i64,
    pub image_mime: String,
    pub image_data: Vec<u8>,
}

pub async fn image(pool: &SqlitePool, key: &str) -> Result<Option<StoredImage>, sqlx::Error> {
    sqlx::query_as("SELECT url_id, image_mime, image_data FROM url_previews WHERE image_key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
}
