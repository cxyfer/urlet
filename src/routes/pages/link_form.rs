//! 短網址新增與編輯共用的表單。
//!
//! 儀表板的新增表單、首頁的公開表單，以及編輯懸浮窗（含無 JavaScript 時的
//! 編輯頁）都由 [`LinkForm`] 渲染 `partials/link_form.html`，欄位順序與兩個
//! 展開區塊（詳細設定、SNS 預覽）因此只有一份實作。

use axum::extract::{FromRequest, Multipart, Request};
use axum::http::header;
use axum::Form;
use serde::de::DeserializeOwned;

use crate::db::url_previews::Preview;
use crate::error::AppError;
use crate::i18n::{self, Lang};
use crate::routes::pages::UrlRow;
use crate::routes::previews::{FormView, PreviewFields};
use crate::services::domains::DomainOption;
use crate::services::limits::Limits;
use crate::services::redirect_mode::{RedirectMode, RedirectModePolicy};
use crate::services::settings::PublicFields;
use crate::state::AppState;

/// One rendered link form; see the module documentation.
#[derive(Debug, Clone, Default)]
pub struct LinkForm {
    pub is_edit: bool,
    pub is_public: bool,
    pub action: String,
    /// 同一頁可能同時有新增表單與編輯懸浮窗，所以每份表單的 id 都加上前綴。
    pub prefix: &'static str,
    /// Page to return to after a successful edit (validated, may be empty).
    pub next: String,
    /// Localised error shown above the form after a rejected submission.
    pub error: String,
    /// Catalogue key of the "some options are restricted" note, or empty.
    pub notice: &'static str,
    pub show_domain: bool,
    pub show_slug: bool,
    pub show_title: bool,
    pub show_tags: bool,
    pub show_password: bool,
    pub show_expiry: bool,
    pub show_max_clicks: bool,
    pub show_redirect_type: bool,
    pub show_redirect_mode: bool,
    pub show_status: bool,
    pub domains: Vec<DomainOption>,
    /// Hostname selected in the domain select.
    pub domain: String,
    pub target_url: String,
    pub slug: String,
    pub title: String,
    pub tags: String,
    /// The link currently has a password (edit only).
    pub has_password: bool,
    pub expires_at: String,
    pub max_clicks: String,
    pub redirect_type: i64,
    /// Instance default, labelled as such in the select.
    pub default_redirect_type: i64,
    pub redirect_mode: String,
    /// False when the stored mode is not `direct`/`splash`: the select then
    /// keeps it through a disabled placeholder instead of rewriting it.
    pub mode_known: bool,
    pub allow_direct_mode: bool,
    pub allow_splash_mode: bool,
    /// The group fixes the mode, so the hint explains the lock.
    pub mode_locked: bool,
    pub splash_delay_seconds: String,
    pub splash_title: String,
    pub splash_message: String,
    /// `active` or `disabled` (edit only).
    pub status: String,
    pub clear_password: bool,
    pub clear_expiry: bool,
    pub clear_max_clicks: bool,
    pub clear_tags: bool,
    /// SNS 預覽欄位；公開表單沒有這一區。
    pub preview: Option<FormView>,
    pub advanced_open: bool,
    pub preview_open: bool,
    /// Short summary of the configured advanced options (edit only).
    pub advanced_badges: Vec<String>,
    /// Short summary of the configured preview (edit only).
    pub preview_badges: Vec<String>,
    pub submit_label: &'static str,
}

impl LinkForm {
    /// Whether the "advanced" disclosure has anything to show.
    pub fn has_advanced(&self) -> bool {
        self.show_password
            || self.show_expiry
            || self.show_max_clicks
            || self.show_redirect_type
            || self.show_redirect_mode
            || self.has_password
            || !self.expires_at.is_empty()
            || !self.max_clicks.is_empty()
    }

    /// The form carries a file input, so it has to post multipart.
    pub fn multipart(&self) -> bool {
        self.preview.is_some()
    }

    /// The dashboard's create form.
    pub fn create(
        state: &AppState,
        limits: &Limits,
        allow_domain_selection: bool,
        domains: Vec<DomainOption>,
    ) -> Self {
        let policy = limits.redirect_mode_policy;
        let default_type = state.settings.default_redirect_type();
        let restricted = !limits.allow_custom_slug
            || !limits.allow_password
            || !limits.allow_expiry
            || !limits.allow_max_clicks
            || !limits.allow_tags
            || !allow_domain_selection
            || policy != RedirectModePolicy::Choice;
        Self {
            action: "/app/urls".into(),
            notice: if restricted {
                "dashboard.group_restricted"
            } else {
                ""
            },
            show_domain: allow_domain_selection,
            show_slug: limits.allow_custom_slug,
            show_title: true,
            show_tags: limits.allow_tags,
            show_password: limits.allow_password,
            show_expiry: limits.allow_expiry,
            show_max_clicks: limits.allow_max_clicks,
            show_redirect_type: true,
            show_redirect_mode: true,
            domain: domains
                .iter()
                .find(|domain| domain.is_default)
                .map(|domain| domain.hostname.clone())
                .unwrap_or_default(),
            domains,
            redirect_type: default_type,
            default_redirect_type: default_type,
            redirect_mode: policy.default_mode().as_str().into(),
            mode_known: true,
            allow_direct_mode: policy.allows(RedirectMode::Direct),
            allow_splash_mode: policy.allows(RedirectMode::Splash),
            mode_locked: policy != RedirectModePolicy::Choice,
            splash_delay_seconds: "5".into(),
            status: "active".into(),
            preview: Some(FormView::new(0, None, state)),
            submit_label: "dashboard.create",
            ..Self::default()
        }
    }

    /// The landing page's public form. Its switches come from the site
    /// settings instead of a group.
    pub fn public(fields: &PublicFields, domains: Vec<DomainOption>) -> Self {
        Self {
            is_public: true,
            action: "/".into(),
            prefix: "public_",
            show_domain: fields.domain,
            show_slug: fields.slug,
            show_title: fields.title,
            show_tags: fields.tags,
            show_password: fields.password,
            show_expiry: fields.expiry,
            show_max_clicks: fields.max_clicks,
            show_redirect_mode: fields.redirect,
            domain: domains
                .iter()
                .find(|domain| domain.is_default)
                .map(|domain| domain.hostname.clone())
                .unwrap_or_default(),
            domains,
            redirect_mode: "direct".into(),
            mode_known: true,
            allow_direct_mode: true,
            allow_splash_mode: true,
            splash_delay_seconds: "5".into(),
            submit_label: "public.submit",
            ..Self::default()
        }
    }

    /// The edit form of an existing link.
    #[allow(clippy::too_many_arguments)]
    pub fn edit(
        state: &AppState,
        url: &UrlRow,
        current_domain: String,
        domains: Vec<DomainOption>,
        limits: &Limits,
        allow_domain_selection: bool,
        preview: Option<Preview>,
        lang: Lang,
    ) -> Self {
        let policy = limits.redirect_mode_policy;
        let current_mode = RedirectMode::from_db(&url.redirect_mode);
        let mode_known = matches!(url.redirect_mode.as_str(), "direct" | "splash");
        let restricted = !limits.allow_custom_slug
            || !limits.allow_password
            || !limits.allow_expiry
            || !limits.allow_max_clicks
            || !limits.allow_tags
            || !allow_domain_selection
            || policy != RedirectModePolicy::Choice;
        let default_type = state.settings.default_redirect_type();

        let mut advanced_badges = Vec::new();
        if url.has_password {
            advanced_badges.push(lang.t("link_form.badge.password").to_string());
        }
        if let Some(when) = &url.expires_human {
            advanced_badges.push(i18n::format(
                lang,
                "link_form.badge.expires",
                &[("when", when.clone())],
            ));
        }
        if let Some(limit) = url.max_clicks {
            advanced_badges.push(i18n::format(
                lang,
                "link_form.badge.max_clicks",
                &[("count", limit.to_string())],
            ));
        }
        if url.is_splash {
            advanced_badges.push(lang.t("url.mode.splash").to_string());
        }
        if url.redirect_type != default_type {
            advanced_badges.push(format!("HTTP {}", url.redirect_type));
        }
        let preview_badges = preview
            .as_ref()
            .map(|row| {
                let mut badges = Vec::new();
                if row.title.is_some() || row.description.is_some() {
                    badges.push(lang.t("link_form.badge.preview_text").to_string());
                }
                if row.image_key.is_some() || row.image_url.is_some() {
                    badges.push(lang.t("link_form.badge.preview_image").to_string());
                }
                badges
            })
            .unwrap_or_default();

        Self {
            is_edit: true,
            action: format!("/app/urls/{}", url.id),
            prefix: "edit_",
            notice: if restricted {
                "url_detail.settings.restricted"
            } else {
                ""
            },
            show_domain: allow_domain_selection,
            show_slug: limits.allow_custom_slug,
            show_title: true,
            show_tags: limits.allow_tags,
            show_password: limits.allow_password,
            show_expiry: limits.allow_expiry,
            show_max_clicks: limits.allow_max_clicks,
            show_redirect_type: true,
            show_redirect_mode: true,
            show_status: true,
            domains,
            domain: current_domain,
            target_url: url.target_url.clone(),
            slug: url.slug.clone(),
            title: url.title.clone().unwrap_or_default(),
            tags: url.tags.join(", "),
            has_password: url.has_password,
            expires_at: url.expires_at.clone().unwrap_or_default(),
            max_clicks: url.max_clicks.map(|n| n.to_string()).unwrap_or_default(),
            redirect_type: url.redirect_type,
            default_redirect_type: default_type,
            redirect_mode: url.redirect_mode.clone(),
            mode_known,
            // The current mode is always offered, so an existing link can
            // keep it after the group policy changed.
            allow_direct_mode: policy.allows(RedirectMode::Direct)
                || current_mode == RedirectMode::Direct,
            allow_splash_mode: policy.allows(RedirectMode::Splash)
                || current_mode == RedirectMode::Splash,
            mode_locked: policy != RedirectModePolicy::Choice,
            splash_delay_seconds: url.splash_delay_seconds.to_string(),
            splash_title: url.splash_title.clone(),
            splash_message: url.splash_message.clone(),
            status: if url.state == "disabled" {
                "disabled".into()
            } else {
                "active".into()
            },
            preview: Some(FormView::new(url.id, preview, state)),
            advanced_badges,
            preview_badges,
            submit_label: "url_detail.save",
            ..Self::default()
        }
    }
}

/// A link form submission: the link fields plus, when the form carried the
/// SNS preview section, the preview fields.
///
/// The browser forms post multipart because the preview section can upload an
/// image; API clients and older forms keep posting urlencoded bodies, which
/// carry no preview section.
pub struct LinkSubmission<T> {
    pub form: T,
    pub preview: Option<PreviewFields>,
}

/// A submission that could not be read in full.
pub struct Rejected<T> {
    pub error: AppError,
    /// The body was multipart. A browser form only fails to read when it is
    /// over the size limit, which in practice means the preview image.
    pub multipart: bool,
    /// The fields read before the failure, when they still decode. A body
    /// whose declared length is over the limit is refused before any field is
    /// read, so this is often `None`; the browser checks the file size before
    /// sending for exactly that reason.
    pub partial: Option<LinkSubmission<T>>,
}

/// 解析失敗不在 extractor 內回應，交給 handler 決定要導回哪一頁。
pub struct Submission<T>(pub Result<LinkSubmission<T>, Rejected<T>>);

impl<S, T> FromRequest<S> for Submission<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = std::convert::Infallible;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let is_multipart = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("multipart/form-data"));
        let invalid = |multipart| Rejected {
            error: AppError::bad_request("invalid form"),
            multipart,
            partial: None,
        };
        if !is_multipart {
            return Ok(Self(match Form::<T>::from_request(req, state).await {
                Ok(Form(form)) => Ok(LinkSubmission {
                    form,
                    preview: None,
                }),
                Err(_) => Err(invalid(false)),
            }));
        }
        let multipart = match Multipart::from_request(req, state).await {
            Ok(multipart) => multipart,
            Err(_) => return Ok(Self(Err(invalid(true)))),
        };
        Ok(Self(read_multipart(multipart).await))
    }
}

/// Split a multipart body into the link fields (decoded exactly like the
/// urlencoded form) and the `preview_*` fields.
async fn read_multipart<T: DeserializeOwned>(
    mut multipart: Multipart,
) -> Result<LinkSubmission<T>, Rejected<T>> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut preview = PreviewFields::default();
    let mut has_preview = false;
    let read = read_fields(&mut multipart, &mut pairs, &mut preview, &mut has_preview).await;
    let decoded = serde_urlencoded::to_string(&pairs)
        .ok()
        .and_then(|encoded| serde_urlencoded::from_str::<T>(&encoded).ok());
    match (read, decoded) {
        (Ok(()), Some(form)) => Ok(LinkSubmission {
            form,
            preview: has_preview.then_some(preview),
        }),
        (Ok(()), None) => Err(Rejected {
            error: AppError::bad_request("invalid form"),
            multipart: false,
            partial: None,
        }),
        (Err(error), form) => {
            preview.image = None;
            Err(Rejected {
                error,
                multipart: true,
                partial: form.map(|form| LinkSubmission {
                    form,
                    preview: has_preview.then_some(preview),
                }),
            })
        }
    }
}

async fn read_fields(
    multipart: &mut Multipart,
    pairs: &mut Vec<(String, String)>,
    preview: &mut PreviewFields,
    has_preview: &mut bool,
) -> Result<(), AppError> {
    let mut seen = std::collections::HashSet::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| AppError::bad_request("invalid or oversized form"))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if let Some(key) = name.strip_prefix("preview_") {
            if !seen.insert(name.clone()) {
                return Err(AppError::bad_request("duplicate preview field"));
            }
            *has_preview = true;
            if key == "image" {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|_| AppError::bad_request("invalid or oversized image upload"))?;
                if !bytes.is_empty() {
                    preview.image = Some(bytes.to_vec());
                }
                continue;
            }
            let value = field
                .text()
                .await
                .map_err(|_| AppError::bad_request("invalid preview field"))?;
            match key {
                "title" => preview.title = value,
                "description" => preview.description = value,
                "image_url" => preview.image_url = value,
                "image_mode" => preview.image_mode = value,
                _ => return Err(AppError::bad_request("unknown preview field")),
            }
        } else {
            if field.file_name().is_some() {
                return Err(AppError::bad_request("unexpected file field"));
            }
            let value = field
                .text()
                .await
                .map_err(|_| AppError::bad_request("invalid form field"))?;
            pairs.push((name, value));
        }
    }
    Ok(())
}
