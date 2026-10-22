//! 明確公開的 SNS 預覽。外部圖片只儲存網址，伺服器不代抓內容。
mod webp;

use std::io::Cursor;

use image::{ImageFormat, Limits};
use sqlx::SqlitePool;

use crate::db::{
    url_previews::{self, Changes, Image, Preview},
    urls,
};
use crate::error::{AppError, Result};
use crate::services::{secret, target};

pub const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_BODY_BYTES: usize = MAX_IMAGE_BYTES + 64 * 1024;
// 限制同時解碼數量；單次 max_alloc 是解碼器 buffer 預算，非整個程序的 RSS 上限。
static DECODERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
pub const MAX_TITLE: usize = 200;
pub const MAX_DESCRIPTION: usize = 1000;

pub fn text(raw: &str, field: &'static str, limit: usize) -> Result<Option<String>> {
    let value = raw.trim();
    if value.chars().count() > limit
        || value
            .chars()
            .any(|c| c.is_control() && !(field == "description" && matches!(c, '\n' | '\r' | '\t')))
    {
        return Err(AppError::validation(
            field,
            format!("use at most {limit} characters without control characters"),
        ));
    }
    Ok((!value.is_empty()).then(|| value.to_string()))
}

pub fn image_url(raw: &str) -> Result<Option<String>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let invalid = || {
        AppError::validation(
            "image_url",
            "use an absolute HTTPS image URL without credentials or whitespace",
        )
    };
    if !raw
        .get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
        || raw.chars().any(char::is_whitespace)
    {
        return Err(invalid());
    }
    let validated = target::validate(raw).map_err(|_| invalid())?;
    let authority = validated[8..]
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    if authority.contains('@') {
        return Err(invalid());
    }
    Ok(Some(validated))
}

/// 先限制大小，再於 blocking worker 解碼；不相信副檔名與上傳 MIME。
pub async fn upload(bytes: Vec<u8>) -> Result<Image> {
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err(AppError::validation(
            "image",
            "choose a PNG, JPEG, GIF or WebP image up to 2 MiB",
        ));
    }
    let permit = DECODERS.acquire().await.map_err(AppError::internal)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let invalid = || {
            AppError::validation(
                "image",
                "choose a valid PNG, JPEG, GIF or WebP image up to 4096 × 4096 pixels",
            )
        };
        let format = image::guess_format(&bytes).map_err(|_| invalid())?;
        let mime = match format {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Gif => "image/gif",
            ImageFormat::WebP => "image/webp",
            _ => return Err(invalid()),
        };
        if format == ImageFormat::Gif {
            validate_gif(&bytes).map_err(|_| invalid())?;
        }
        if format == ImageFormat::WebP {
            webp::validate(&bytes).map_err(|_| invalid())?;
        }
        let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
        let mut limits = Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits.clone());
        // 在像素解碼前驗證尺寸；新版 WebP constructor 僅解析標頭。
        let (width, height) = reader.into_dimensions().map_err(|_| invalid())?;
        if width == 0 || height == 0 || width > 4096 || height > 4096 {
            return Err(invalid());
        }
        let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
        reader.limits(limits);
        reader.decode().map_err(|_| invalid())?;
        Ok(Image {
            url: None,
            key: Some(secret::random_token(24)),
            mime: Some(mime.into()),
            data: Some(bytes),
        })
    })
    .await
    .map_err(AppError::internal)?
}

// 只掃描 frame descriptor，略過 LZW 解碼；所有影格必須在受限畫布內。
fn validate_gif(bytes: &[u8]) -> std::result::Result<(), ()> {
    let mut options = gif::DecodeOptions::new();
    options.check_frame_consistency(true);
    options.skip_frame_decoding(true);
    let mut reader = options.read_info(Cursor::new(bytes)).map_err(|_| ())?;
    if reader.width() == 0
        || reader.height() == 0
        || reader.width() > 4096
        || reader.height() > 4096
    {
        return Err(());
    }
    let mut found = false;
    while let Some(frame) = reader.next_frame_info().map_err(|_| ())? {
        if frame.width == 0 || frame.height == 0 {
            return Err(());
        }
        found = true;
    }
    if found {
        Ok(())
    } else {
        Err(())
    }
}

pub async fn owned(pool: &SqlitePool, user_id: i64, id: i64) -> Result<()> {
    urls::find_owned(pool, id, user_id)
        .await?
        .ok_or_else(AppError::not_found)?;
    Ok(())
}

pub async fn save(
    pool: &SqlitePool,
    user_id: i64,
    id: i64,
    changes: Changes,
) -> Result<Option<Preview>> {
    if !url_previews::save(pool, id, user_id, &changes).await? {
        return Err(AppError::not_found());
    }
    Ok(url_previews::find(pool, id).await?)
}

pub fn image_path(key: &str) -> String {
    format!("/assets/link-previews/{key}")
}

pub fn public_image_url(preview: &Preview, base_url: &str) -> Option<String> {
    preview
        .image_key
        .as_deref()
        .map(|key| format!("{}{}", base_url.trim_end_matches('/'), image_path(key)))
        .or_else(|| preview.image_url.clone())
}
