//! SNS 預覽管理及公開圖片。使用者只能編輯自己短網址的預覽。
use askama::Template;
use axum::{
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::{
    db::url_previews::{self, Changes, Image, Preview},
    error::{ApiError, AppError},
    i18n::{self, Lang},
    middleware::auth::CurrentUser,
    routes::{
        dto::double_option,
        pages::{redirect_ok, Chrome},
    },
    services::{
        audit::{action, Audit},
        ratelimit::limits,
        url_preview as service,
    },
    state::AppState,
    themed_page,
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
        image_url: service::public_image_url(&row, &state.config.base_url),
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

async fn save(
    state: &AppState,
    user: &CurrentUser,
    id: i64,
    changes: Changes,
) -> Result<Option<Preview>, AppError> {
    let row = service::save(&state.db, user.user.id, id, changes).await?;
    Audit::new(action::URL_UPDATED)
        .actor(user.user.id, &user.user.role)
        .target("url", id)
        .metadata(serde_json::json!({"fields": ["preview"]}))
        .record(&state.db)
        .await;
    Ok(row)
}

fn upload_limit(state: &AppState, user_id: i64) -> Result<(), AppError> {
    state
        .limiter
        .check(
            &format!("preview-upload:{user_id}"),
            limits::CREATE_URL_CAPACITY,
            limits::CREATE_URL_REFILL_PER_SEC,
        )
        .map_err(AppError::rate_limited)
}

#[derive(Default)]
struct Fields {
    title: String,
    description: String,
    image_url: String,
    image_mode: String,
    image: Option<Vec<u8>>,
}

async fn read_form(mut multipart: Multipart) -> Result<Fields, AppError> {
    let mut fields = Fields::default();
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
#[derive(Debug, Default)]
pub struct FormView {
    pub url_id: i64,
    pub title: String,
    pub description: String,
    pub image_url: String,
    pub image_mode: String,
    pub current_image: String,
    pub error: String,
}

impl FormView {
    pub fn new(id: i64, row: Option<Preview>, state: &AppState) -> Self {
        let current_image = row
            .as_ref()
            .and_then(|p| service::public_image_url(p, &state.config.base_url))
            .unwrap_or_default();
        let row = row.unwrap_or_default();
        Self {
            url_id: id,
            title: row.title.unwrap_or_default(),
            description: row.description.unwrap_or_default(),
            image_url: row.image_url.unwrap_or_default(),
            image_mode: if current_image.is_empty() {
                "none"
            } else {
                "keep"
            }
            .into(),
            current_image,
            error: String::new(),
        }
    }
}

pub struct EditPage {
    pub site_name: String,
    pub chrome: Chrome,
    pub preview: FormView,
    pub lang: Lang,
}

themed_page!(preview_edit_templates, EditView, EditPage,
    Verdant => "preview_edit.html",
    Aurora => "themes/aurora/preview_edit.html",
    Paper => "themes/paper/preview_edit.html",
    Signal => "themes/signal/preview_edit.html",
    Gilded => "themes/gilded/preview_edit.html",
    Swiss => "themes/swiss/preview_edit.html",
);

pub async fn save_form(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    multipart: Result<Multipart, axum::extract::multipart::MultipartRejection>,
) -> Result<Response, AppError> {
    service::owned(&state.db, user.user.id, id).await?;
    upload_limit(&state, user.user.id)?;
    let old = url_previews::find(&state.db, id).await?;
    let mut view = FormView::new(id, old, &state);
    let result = match multipart {
        Err(_) => Err(AppError::bad_request("invalid or oversized preview form")),
        Ok(multipart) => match read_form(multipart).await {
            Err(err) => Err(err),
            Ok(fields) => {
                view.title = fields.title.clone();
                view.description = fields.description.clone();
                if fields.image_mode == "url" {
                    view.image_url = fields.image_url.clone();
                }
                view.image_mode = fields.image_mode.clone();
                form_changes(fields).await
            }
        },
    };
    let result = match result {
        Ok(changes) => save(&state, &user, id, changes).await.map(|_| ()),
        Err(err) => Err(err),
    };
    match result {
        Ok(()) => Ok(redirect_ok(&format!("/app/urls/{id}"), "saved").into_response()),
        Err(err) if matches!(err, AppError::Validation { .. } | AppError::BadRequest(_)) => {
            view.error = match err {
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
            .into();
            let page = EditPage {
                site_name: state.settings.site_name(),
                chrome: Chrome::new(&state, &user.user, "dashboard", None),
                preview: view,
                lang: i18n::current(),
            }
            .themed();
            let html = page.render().map_err(AppError::internal)?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response())
        }
        Err(err) => Err(err),
    }
}

async fn form_changes(fields: Fields) -> Result<Changes, AppError> {
    let title = service::text(&fields.title, "title", service::MAX_TITLE)?;
    let description = service::text(&fields.description, "description", service::MAX_DESCRIPTION)?;
    let image = match fields.image_mode.as_str() {
        "keep" => None,
        "none" => Some(Image::default()),
        "url" => {
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
        "upload" => Some(
            service::upload(
                fields
                    .image
                    .ok_or_else(|| AppError::validation("image", "choose an image"))?,
            )
            .await?,
        ),
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
