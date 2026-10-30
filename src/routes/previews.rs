//! SNS 預覽管理及公開圖片。使用者只能編輯自己短網址的預覽。
use axum::{
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use serde::{Deserialize, Serialize};

use crate::{
    db::url_previews::{self, Changes, Image, Preview},
    error::{ApiError, AppError},
    middleware::auth::{CurrentUser, MaybeUser},
    routes::extract::Json,
    routes::{
        dto::double_option,
        pages::{redirect_ok, redirect_to_login},
    },
    services::{
        audit::{action, Audit},
        ratelimit::limits,
        settings::PreviewImagePolicy,
        url_preview as service,
    },
    state::AppState,
};

#[derive(Debug, Default, Deserialize)]
pub struct Patch {
    #[serde(default, deserialize_with = "double_option")]
    pub title: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub description: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub image_url: Option<Option<String>>,
}

impl Patch {
    fn changes(self) -> Result<Changes, AppError> {
        Ok(Changes {
            title: self
                .title
                .map(|s| {
                    service::text(
                        s.as_deref().unwrap_or_default(),
                        "title",
                        service::MAX_TITLE,
                    )
                })
                .transpose()?,
            description: self
                .description
                .map(|s| {
                    service::text(
                        s.as_deref().unwrap_or_default(),
                        "description",
                        service::MAX_DESCRIPTION,
                    )
                })
                .transpose()?,
            image: self
                .image_url
                .map(|s| {
                    service::image_url(s.as_deref().unwrap_or_default()).map(|url| Image {
                        url,
                        ..Default::default()
                    })
                })
                .transpose()?,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct PreviewDto {
    pub title: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub image_uploaded: bool,
    pub enabled: bool,
}

fn dto(row: Option<Preview>, state: &AppState) -> PreviewDto {
    let enabled = row.is_some();
    let row = row.unwrap_or_default();
    PreviewDto {
        image_url: service::public_image_url(
            &row,
            &state.config.base_url,
            state.settings.preview_image_policy(),
        ),
        image_uploaded: row.image_key.is_some(),
        title: row.title,
        description: row.description,
        enabled,
    }
}

pub fn api_router() -> Router<AppState> {
    Router::new()
        .route(
            "/urls/{id}/preview",
            axum::routing::get(get).patch(patch).delete(clear),
        )
        .route(
            "/urls/{id}/preview/image",
            axum::routing::post(upload_api).layer(DefaultBodyLimit::max(service::MAX_BODY_BYTES)),
        )
}

pub async fn get(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<PreviewDto>, ApiError> {
    service::owned(&state.db, user.user.id, id).await?;
    Ok(Json(dto(url_previews::find(&state.db, id).await?, &state)))
}

pub async fn patch(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Json(input): Json<Patch>,
) -> Result<Json<PreviewDto>, ApiError> {
    service::owned(&state.db, user.user.id, id).await?;
    let row = save(&state, &user, id, input.changes()?).await?;
    Ok(Json(dto(row, &state)))
}

pub async fn clear(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    service::owned(&state.db, user.user.id, id).await?;
    save(
        &state,
        &user,
        id,
        Changes {
            title: Some(None),
            description: Some(None),
            image: Some(Image::default()),
        },
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn save(
    state: &AppState,
    user: &CurrentUser,
    id: i64,
    changes: Changes,
) -> Result<Option<Preview>, AppError> {
    if let Some(image) = changes.image.as_ref() {
        let policy = state.settings.preview_image_policy();
        if image.url.is_some() {
            service::require_image_source(policy, false)?;
        }
        if image.key.is_some() {
            service::require_image_source(policy, true)?;
        }
    }
    let row = service::save(&state.db, user.user.id, id, changes).await?;
    Audit::new(action::URL_UPDATED)
        .actor(user.user.id, &user.user.role)
        .target("url", id)
        .metadata(serde_json::json!({"fields": ["preview"]}))
        .record(&state.db)
        .await;
    Ok(row)
}

pub(crate) fn upload_limit(state: &AppState, user_id: i64) -> Result<(), AppError> {
    state
        .limiter
        .check(
            &format!("preview-upload:{user_id}"),
            limits::CREATE_URL_CAPACITY,
            limits::CREATE_URL_REFILL_PER_SEC,
        )
        .map_err(AppError::rate_limited)
}

/// SNS 預覽表單的欄位。合併在連結表單時欄位名稱帶 `preview_` 前綴。
#[derive(Debug, Default)]
pub struct PreviewFields {
    pub title: String,
    pub description: String,
    pub image_url: String,
    pub image_mode: String,
    pub image: Option<Vec<u8>>,
}

async fn read_form(mut multipart: Multipart) -> Result<PreviewFields, AppError> {
    let mut fields = PreviewFields::default();
    let mut seen = std::collections::HashSet::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| AppError::bad_request("invalid or oversized preview form"))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if !seen.insert(name.clone()) {
            return Err(AppError::bad_request("duplicate preview field"));
        }
        if name == "image" {
            let bytes = field
                .bytes()
                .await
                .map_err(|_| AppError::bad_request("invalid or oversized image upload"))?;
            if !bytes.is_empty() {
                fields.image = Some(bytes.to_vec());
            }
        } else {
            let value = field
                .text()
                .await
                .map_err(|_| AppError::bad_request("invalid preview field"))?;
            match name.as_str() {
                "title" => fields.title = value,
                "description" => fields.description = value,
                "image_url" => fields.image_url = value,
                "image_mode" => fields.image_mode = value,
                _ => return Err(AppError::bad_request("unknown preview field")),
            }
        }
    }
    Ok(fields)
}

pub async fn upload_api(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    multipart: Result<Multipart, axum::extract::multipart::MultipartRejection>,
) -> Result<Json<PreviewDto>, ApiError> {
    service::owned(&state.db, user.user.id, id).await?;
    service::require_image_source(state.settings.preview_image_policy(), true)?;
    upload_limit(&state, user.user.id)?;
    let fields =
        read_form(multipart.map_err(|_| AppError::bad_request("invalid image upload"))?).await?;
    let image = service::upload(
        fields
            .image
            .ok_or_else(|| AppError::validation("image", "choose an image"))?,
    )
    .await?;
    let row = save(
        &state,
        &user,
        id,
        Changes {
            image: Some(image),
            ..Default::default()
        },
    )
    .await?;
    Ok(Json(dto(row, &state)))
}

/// 共用欄位同時用於詳情頁及驗證失敗後的重試頁。
#[derive(Debug, Clone, Default)]
pub struct FormView {
    pub url_id: i64,
    pub title: String,
    pub description: String,
    pub image_url: String,
    pub image_mode: String,
    pub current_image: String,
    pub error: String,
    pub allow_url: bool,
    pub allow_upload: bool,
    pub policy_key: String,
    pub image_suppressed: bool,
    /// Largest accepted upload, so the browser can refuse a bigger file before
    /// sending it.
    pub max_image_bytes: usize,
}

impl FormView {
    pub fn new(id: i64, row: Option<Preview>, state: &AppState) -> Self {
        let policy = state.settings.preview_image_policy();
        let has_image = row
            .as_ref()
            .is_some_and(|p| p.image_key.is_some() || p.image_url.is_some());
        let current_image = row
            .as_ref()
            .and_then(|p| service::public_image_url(p, &state.config.base_url, policy))
            .unwrap_or_default();
        let row = row.unwrap_or_default();
        Self {
            url_id: id,
            title: row.title.unwrap_or_default(),
            description: row.description.unwrap_or_default(),
            image_url: row.image_url.unwrap_or_default(),
            image_mode: if !has_image { "none" } else { "keep" }.into(),
            image_suppressed: has_image && current_image.is_empty(),
            current_image,
            error: String::new(),
            allow_url: policy.allows_url(),
            allow_upload: policy.allows_upload(),
            policy_key: format!("preview.policy.{}", policy.as_str()),
            max_image_bytes: service::MAX_IMAGE_BYTES,
        }
    }
}

impl FormView {
    /// 把送出的值放回表單，驗證失敗時使用者不必重填。
    pub fn retain(&mut self, fields: &PreviewFields) {
        self.title = fields.title.clone();
        self.description = fields.description.clone();
        if fields.image_mode == "url" {
            self.image_url = fields.image_url.clone();
        }
        self.image_mode = fields.image_mode.clone();
    }

    /// Record why the preview was rejected. A source the policy no longer
    /// offers falls back to keeping the stored image, so the select still has
    /// a valid choice.
    pub fn reject(&mut self, key: &'static str) {
        self.error = key.into();
        if (self.image_mode == "url" && !self.allow_url)
            || (self.image_mode == "upload" && !self.allow_upload)
        {
            self.image_mode = "keep".into();
        }
    }
}

/// Whether `err` is a problem with the submitted preview, reported next to the
/// fields, rather than a server failure.
pub(crate) fn is_form_error(err: &AppError) -> bool {
    matches!(
        err,
        AppError::Validation { .. } | AppError::BadRequest(_) | AppError::Forbidden(_)
    )
}

/// Catalogue key of the message shown for a rejected preview.
pub(crate) fn error_key(err: &AppError) -> &'static str {
    match err {
        AppError::Forbidden(_) => "preview.error_policy",
        AppError::Validation { field: "title", .. } => "preview.error_title",
        AppError::Validation {
            field: "description",
            ..
        } => "preview.error_description",
        AppError::Validation {
            field: "image_url", ..
        } => "preview.error_url",
        _ => "preview.error_image",
    }
}

/// 連結表單每次儲存都會送出預覽欄位；內容沒變就不寫入，避免多出空白預覽列與
/// 不必要的稽核紀錄。
pub(crate) fn is_noop(changes: &Changes, existing: Option<&Preview>) -> bool {
    let empty = Preview::default();
    let existing = existing.unwrap_or(&empty);
    let same_text = |change: &Option<Option<String>>, current: &Option<String>| match change {
        None => true,
        Some(value) => value == current,
    };
    let same_image = match &changes.image {
        None => true,
        Some(image) => {
            image.url.is_none()
                && image.key.is_none()
                && existing.image_url.is_none()
                && existing.image_key.is_none()
        }
    };
    same_text(&changes.title, &existing.title)
        && same_text(&changes.description, &existing.description)
        && same_image
}

/// `POST /app/urls/{id}/preview` — the standalone preview form of earlier
/// releases. The link editor now saves the preview together with the link;
/// this route stays for pages and scripts that still post here, and renders
/// the link editor when the preview is rejected.
pub async fn save_form(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    Path(id): Path<i64>,
    multipart: Result<Multipart, axum::extract::multipart::MultipartRejection>,
) -> Result<Response, AppError> {
    let Some(user) = user else {
        return Ok(redirect_to_login(Some(&format!("/app/urls/{id}"))).into_response());
    };
    service::owned(&state.db, user.user.id, id).await?;
    upload_limit(&state, user.user.id)?;
    let mut submitted = None;
    let result = match multipart {
        Err(_) => Err(AppError::bad_request("invalid or oversized preview form")),
        Ok(multipart) => match read_form(multipart).await {
            Err(err) => Err(err),
            Ok(fields) => {
                let changes = form_changes(&fields, state.settings.preview_image_policy()).await;
                submitted = Some(fields);
                changes
            }
        },
    };
    let result = match result {
        Ok(changes) => save(&state, &user, id, changes).await.map(|_| ()),
        Err(err) => Err(err),
    };
    match result {
        Ok(()) => Ok(redirect_ok(&format!("/app/urls/{id}"), "saved").into_response()),
        Err(err) if is_form_error(&err) => {
            crate::routes::pages::urls::render_edit(&state, &user.user, id, |form| {
                if let Some(view) = form.preview.as_mut() {
                    if let Some(fields) = &submitted {
                        view.retain(fields);
                    }
                    view.reject(error_key(&err));
                }
                form.preview_open = true;
            })
            .await
        }
        Err(err) => Err(err),
    }
}

pub(crate) async fn form_changes(
    fields: &PreviewFields,
    policy: PreviewImagePolicy,
) -> Result<Changes, AppError> {
    let title = service::text(&fields.title, "title", service::MAX_TITLE)?;
    let description = service::text(&fields.description, "description", service::MAX_DESCRIPTION)?;
    let image = match fields.image_mode.as_str() {
        "keep" => None,
        "none" => Some(Image::default()),
        "url" => {
            service::require_image_source(policy, false)?;
            let url = service::image_url(&fields.image_url)?;
            if url.is_none() {
                return Err(AppError::validation(
                    "image_url",
                    "enter an HTTPS image URL",
                ));
            }
            Some(Image {
                url,
                ..Default::default()
            })
        }
        "upload" => {
            service::require_image_source(policy, true)?;
            Some(
                service::upload(
                    fields
                        .image
                        .clone()
                        .ok_or_else(|| AppError::validation("image", "choose an image"))?,
                )
                .await?,
            )
        }
        _ => return Err(AppError::validation("image", "choose an image source")),
    };
    Ok(Changes {
        title: Some(title),
        description: Some(description),
        image,
    })
}

pub async fn public_image(State(state): State<AppState>, Path(key): Path<String>) -> Response {
    crate::routes::no_store(public_image_inner(&state, &key).await.into_response())
}

async fn public_image_inner(state: &AppState, key: &str) -> Result<Response, AppError> {
    if !state.settings.preview_image_policy().allows_upload() {
        return Err(AppError::not_found());
    }
    if key.len() != 32
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(AppError::not_found());
    }
    let image = url_previews::image(&state.db, key)
        .await?
        .ok_or_else(AppError::not_found)?;
    let url = crate::db::urls::find_by_id(&state.db, image.url_id)
        .await?
        .ok_or_else(AppError::not_found)?;
    if !url.state().is_reachable()
        || !state
            .domains
            .lookup(&state.domain_hostname(url.domain_id))
            .is_some_and(|d| d.is_active())
    {
        return Err(AppError::not_found());
    }
    Ok((
        [
            (header::CONTENT_TYPE, image.image_mime),
            (header::CACHE_CONTROL, "no-store".into()),
        ],
        image.image_data,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored() -> Preview {
        Preview {
            url_id: 1,
            title: Some("Title".into()),
            description: Some("Description".into()),
            image_url: None,
            image_key: Some("key".into()),
        }
    }

    fn changes(title: Option<&str>, description: Option<&str>, image: Option<Image>) -> Changes {
        Changes {
            title: Some(title.map(str::to_string)),
            description: Some(description.map(str::to_string)),
            image,
        }
    }

    #[test]
    fn resubmitting_the_stored_preview_is_a_noop() {
        let existing = stored();
        assert!(is_noop(
            &changes(Some("Title"), Some("Description"), None),
            Some(&existing)
        ));
    }

    #[test]
    fn an_empty_section_without_a_stored_preview_is_a_noop() {
        assert!(is_noop(&changes(None, None, Some(Image::default())), None));
    }

    #[test]
    fn changed_text_or_a_removed_image_is_a_change() {
        let existing = stored();
        assert!(!is_noop(
            &changes(None, Some("Description"), None),
            Some(&existing)
        ));
        assert!(!is_noop(
            &changes(Some("Title"), Some("Other"), None),
            Some(&existing)
        ));
        // "No image" while an image is stored removes it.
        assert!(!is_noop(
            &changes(Some("Title"), Some("Description"), Some(Image::default())),
            Some(&existing)
        ));
        // A new image is always a change.
        let image = Image {
            url: Some("https://images.example/a.png".into()),
            ..Default::default()
        };
        assert!(!is_noop(&changes(None, None, Some(image)), None));
    }
}
