//! `tags` and `url_tags` tables.

use std::collections::HashMap;

use sqlx::{query_as, FromRow, SqlitePool};

use crate::services::time::now_iso;

#[derive(Debug, Clone, FromRow)]
pub struct Tag {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub color: Option<String>,
    pub created_at: String,
}

/// Tag with the number of live links carrying it.
#[derive(Debug, Clone, FromRow)]
pub struct TagWithCount {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub url_count: i64,
}

const SELECT: &str = "SELECT id, user_id, name, color, created_at FROM tags";

/// Create the tag when missing, then return its id.
pub async fn ensure(pool: &SqlitePool, user_id: i64, name: &str) -> Result<i64, sqlx::Error> {
    sqlx::query("INSERT OR IGNORE INTO tags (user_id, name, created_at) VALUES (?, ?, ?)")
        .bind(user_id)
        .bind(name)
        .bind(now_iso())
        .execute(pool)
        .await?;

    sqlx::query_scalar("SELECT id FROM tags WHERE user_id = ? AND name = ?")
        .bind(user_id)
        .bind(name)
        .fetch_one(pool)
        .await
}

/// Find one tag by name.
pub async fn find_by_name(
    pool: &SqlitePool,
    user_id: i64,
    name: &str,
) -> Result<Option<Tag>, sqlx::Error> {
    sqlx::query_as::<_, Tag>(&format!("{SELECT} WHERE user_id = ? AND name = ?"))
        .bind(user_id)
        .bind(name)
        .fetch_optional(pool)
        .await
}

/// Tags of one user, with usage counts.
pub async fn list_for_user(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<Vec<TagWithCount>, sqlx::Error> {
    sqlx::query_as::<_, TagWithCount>(
        "SELECT t.id, t.name, t.color,
                (SELECT COUNT(*) FROM url_tags ut JOIN urls u ON u.id = ut.url_id
                 WHERE ut.tag_id = t.id AND u.deleted_at IS NULL) AS url_count
         FROM tags t
         WHERE t.user_id = ?
         ORDER BY t.name COLLATE NOCASE ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Tags attached to a link.
pub async fn for_url(pool: &SqlitePool, url_id: i64) -> Result<Vec<Tag>, sqlx::Error> {
    sqlx::query_as::<_, Tag>(&format!(
        "{SELECT} WHERE id IN (SELECT tag_id FROM url_tags WHERE url_id = ?)
         ORDER BY name COLLATE NOCASE ASC"
    ))
    .bind(url_id)
    .fetch_all(pool)
    .await
}

/// Tags for many links at once, keyed by link id.
pub async fn for_urls(
    pool: &SqlitePool,
    url_ids: &[i64],
) -> Result<HashMap<i64, Vec<Tag>>, sqlx::Error> {
    if url_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = std::iter::repeat("?").take(url_ids.len()).collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT ut.url_id AS url_id, t.id, t.user_id, t.name, t.color, t.created_at
         FROM url_tags ut JOIN tags t ON t.id = ut.tag_id
         WHERE ut.url_id IN ({placeholders})
         ORDER BY t.name COLLATE NOCASE ASC"
    );
    let mut query = sqlx::query_as::<_, Row>(&sql);
    for id in url_ids {
        query = query.bind(id);
    }

    #[derive(FromRow)]
    struct Row {
        url_id: i64,
        id: i64,
        user_id: i64,
        name: String,
        color: Option<String>,
        created_at: String,
    }

    let rows: Vec<Row> = query_as::<_, Row>(&sql).fetch_all(pool).await?;
    let mut map: HashMap<i64, Vec<Tag>> = HashMap::new();
    for row in rows {
        map.entry(row.url_id).or_default().push(Tag {
            id: row.id,
            user_id: row.user_id,
            name: row.name,
            color: row.color,
            created_at: row.created_at,
        });
    }
    Ok(map)
}

/// Replace the tag set of a link.
pub async fn set_for_url(
    pool: &SqlitePool,
    url_id: i64,
    tag_ids: &[i64],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM url_tags WHERE url_id = ?").bind(url_id).execute(&mut *tx).await?;
    for tag_id in tag_ids {
        sqlx::query("INSERT OR IGNORE INTO url_tags (url_id, tag_id) VALUES (?, ?)")
            .bind(url_id)
            .bind(tag_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Delete a tag owned by a user (the link association cascades).
pub async fn delete(pool: &SqlitePool, tag_id: i64, user_id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM tags WHERE id = ? AND user_id = ?")
        .bind(tag_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Rename a tag.
pub async fn rename(
    pool: &SqlitePool,
    tag_id: i64,
    user_id: i64,
    name: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("UPDATE tags SET name = ? WHERE id = ? AND user_id = ?")
        .bind(name)
        .bind(tag_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
