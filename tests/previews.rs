//! SNS 預覽的實際 router、權限、上傳與生命週期驗證。
mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::{delete_as, patch_json_as, post_form_as, put_json_as, TestApp, TestResponse};
use serde_json::{json, Value};
use std::io::Cursor;

async fn setup() -> (TestApp, String, i64) {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("owner@example.com", "supersecret").await;
    let response = app.post_json_as("/api/v1/urls", json!({"target_url":"https://destination.example/private-path", "slug":"share", "title":"private management title", "notes":"private notes"}), &cookie).await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    let id = response.json()["id"].as_i64().unwrap();
    (app, cookie, id)
}

async fn patch(app: &TestApp, cookie: &str, id: i64, value: Value) -> TestResponse {
    app.send(patch_json_as(
        &format!("/api/v1/urls/{id}/preview"),
        value,
        cookie,
    ))
    .await
}

async fn crawl(app: &TestApp, ua: &str) -> TestResponse {
    app.send(
        Request::builder()
            .uri("/share")
            .header("user-agent", ua)
            .body(Body::empty())
            .unwrap(),
    )
    .await
}

fn png() -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

async fn configure_images(app: &TestApp, admin: &str, policy: &str) {
    let response = app
        .send(put_json_as(
            "/api/v1/admin/settings",
            json!({"preview_image_policy":policy}),
            admin,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
}

#[tokio::test]
async fn image_policy_settings_require_admin_validate_atomically_and_persist() {
    let (app, admin, _) = setup().await;
    let (member, _) = app.register("member@example.com", "supersecret").await;
    let uri = "/api/v1/admin/settings";
    assert_eq!(
        app.get_as(uri, &admin).await.json()["preview_image_policy"],
        "both"
    );
    assert_eq!(
        app.send(put_json_as(
            uri,
            json!({"preview_image_policy":"none"}),
            &member
        ))
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(put_json_as(uri, json!({"preview_image_policy":"none"}), ""))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    configure_images(&app, &admin, "UPLOAD").await;
    for invalid in ["", "typo", "all", "true"] {
        let response = app
            .send(put_json_as(
                uri,
                json!({"preview_image_policy":invalid, "site_name":"must not save"}),
                &admin,
            ))
            .await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(app.state.settings.site_name(), "urlet");
        assert_eq!(app.state.settings.preview_image_policy().as_str(), "upload");
    }
    let old_form = "site_name=urlet&registration_mode=open&default_slug_length=7&max_urls_per_user=1000&default_redirect_type=302&click_retention_days=0";
    assert_eq!(
        app.send(post_form_as("/app/admin/settings", old_form, &admin))
            .await
            .status,
        StatusCode::SEE_OTHER
    );
    assert_eq!(app.state.settings.preview_image_policy().as_str(), "upload");
    let form = format!("{old_form}&preview_image_policy=url");
    assert_eq!(
        app.send(post_form_as("/app/admin/settings", &form, &admin))
            .await
            .status,
        StatusCode::SEE_OTHER
    );
    let reloaded = urlet::services::settings::Settings::load(&app.state.db, &app.state.config)
        .await
        .unwrap();
    assert_eq!(reloaded.preview_image_policy().as_str(), "url");
    sqlx::query("DELETE FROM settings WHERE key='preview_image_policy'")
        .execute(&app.state.db)
        .await
        .unwrap();
    let reloaded = urlet::services::settings::Settings::load(&app.state.db, &app.state.config)
        .await
        .unwrap();
    assert_eq!(reloaded.preview_image_policy().as_str(), "both");
    assert_eq!(
        urlet::services::settings::PreviewImagePolicy::parse("corrupt"),
        urlet::services::settings::PreviewImagePolicy::None
    );
}

#[tokio::test]
async fn all_four_image_policies_gate_api_and_form_writes_without_blocking_text_or_clear() {
    let (app, admin, id) = setup().await;
    let api = format!("/api/v1/urls/{id}/preview/image");
    let form = format!("/app/urls/{id}/preview");
    for (policy, allow_url, allow_upload) in [
        ("none", false, false),
        ("url", true, false),
        ("upload", false, true),
        ("both", true, true),
    ] {
        configure_images(&app, &admin, policy).await;
        assert_eq!(
            patch(
                &app,
                &admin,
                id,
                json!({"title":"Before", "image_url":null})
            )
            .await
            .status,
            StatusCode::OK
        );
        let response = patch(&app, &admin, id, json!({"title":"External", "image_url":"https://images.example/signed.png?signature=keep"})).await;
        assert_eq!(
            response.status,
            if allow_url {
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            },
            "{policy}: {}",
            response.body
        );
        if !allow_url {
            assert_eq!(
                urlet::db::url_previews::find(&app.state.db, id)
                    .await
                    .unwrap()
                    .unwrap()
                    .title
                    .as_deref(),
                Some("Before")
            );
        }
        // 禁止上傳時，連無效圖片都先回政策錯誤，不進解碼器。
        let bytes = if allow_upload {
            png()
        } else {
            b"invalid image".to_vec()
        };
        let response = app.send(multipart(&api, &admin, &[], Some(&bytes))).await;
        assert_eq!(
            response.status,
            if allow_upload {
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            },
            "{policy}: {}",
            response.body
        );
        for (mode, permitted) in [("url", allow_url), ("upload", allow_upload)] {
            patch(&app, &admin, id, json!({"title":"Before"})).await;
            let response = app
                .send(multipart(
                    &form,
                    &admin,
                    &[
                        ("title", "Retain this text"),
                        ("image_mode", mode),
                        ("image_url", "https://images.example/new.png"),
                    ],
                    Some(&png()),
                ))
                .await;
            assert_eq!(
                response.status,
                if permitted {
                    StatusCode::SEE_OTHER
                } else {
                    StatusCode::UNPROCESSABLE_ENTITY
                },
                "{policy} {mode}: {}",
                response.body
            );
            if !permitted {
                assert!(response.body.contains("Retain this text"));
                assert!(response
                    .body
                    .contains("This image source is no longer allowed"));
                assert_eq!(
                    urlet::db::url_previews::find(&app.state.db, id)
                        .await
                        .unwrap()
                        .unwrap()
                        .title
                        .as_deref(),
                    Some("Before")
                );
            }
        }
        for mode in ["keep", "none"] {
            let response = app
                .send(multipart(
                    &form,
                    &admin,
                    &[("title", "Text only"), ("image_mode", mode)],
                    None,
                ))
                .await;
            assert_eq!(
                response.status,
                StatusCode::SEE_OTHER,
                "{policy} {mode}: {}",
                response.body
            );
        }
        assert_eq!(
            app.send(delete_as(&format!("/api/v1/urls/{id}/preview"), &admin))
                .await
                .status,
            StatusCode::NO_CONTENT
        );
    }
}

#[tokio::test]
async fn policy_changes_suspend_existing_images_and_restore_them_without_data_loss() {
    let (app, admin, id) = setup().await;
    for uploaded in [false, true] {
        configure_images(&app, &admin, "both").await;
        patch(
            &app,
            &admin,
            id,
            json!({"title":"Public title", "image_url":"https://images.example/a.png"}),
        )
        .await;
        if uploaded {
            assert_eq!(
                app.send(multipart(
                    &format!("/api/v1/urls/{id}/preview/image"),
                    &admin,
                    &[],
                    Some(&png())
                ))
                .await
                .status,
                StatusCode::OK
            );
        }
        let before = urlet::db::url_previews::find(&app.state.db, id)
            .await
            .unwrap()
            .unwrap();
        for (policy, permits) in [
            ("none", false),
            ("url", !uploaded),
            ("upload", uploaded),
            ("both", true),
        ] {
            configure_images(&app, &admin, policy).await;
            let response = crawl(&app, "Discordbot").await;
            assert_eq!(response.status, StatusCode::OK);
            assert!(response.body.contains("Public title"));
            assert_eq!(
                response.body.contains("property=\"og:image\""),
                permits,
                "{policy} uploaded={uploaded}"
            );
            let dto = app
                .get_as(&format!("/api/v1/urls/{id}/preview"), &admin)
                .await
                .json();
            assert_eq!(dto["image_url"].is_null(), !permits);
            assert_eq!(dto["image_uploaded"], uploaded);
            if let Some(key) = before.image_key.as_deref() {
                let media = app
                    .get(&urlet::services::url_preview::image_path(key))
                    .await;
                assert_eq!(
                    media.status,
                    if permits {
                        StatusCode::OK
                    } else {
                        StatusCode::NOT_FOUND
                    }
                );
                assert_eq!(media.header("cache-control").as_deref(), Some("no-store"));
            }
            let stored = urlet::db::url_previews::find(&app.state.db, id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored.image_key, before.image_key);
            assert_eq!(stored.image_url, before.image_url);
        }
        configure_images(&app, &admin, "none").await;
        assert_eq!(
            patch(&app, &admin, id, json!({"image_url":null}))
                .await
                .status,
            StatusCode::OK
        );
        let stored = urlet::db::url_previews::find(&app.state.db, id)
            .await
            .unwrap()
            .unwrap();
        assert!(stored.image_key.is_none() && stored.image_url.is_none());
    }
}

#[tokio::test]
async fn image_policy_controls_render_for_every_theme_locale_and_policy() {
    let (app, admin, id) = setup().await;
    for (policy, allow_url, allow_upload) in [
        ("none", false, false),
        ("url", true, false),
        ("upload", false, true),
        ("both", true, true),
    ] {
        configure_images(&app, &admin, policy).await;
        for theme in urlet::services::theme::ALL {
            for locale in ["en", "zh-Hant", "zh-Hans"] {
                let cookie = format!("{admin}; urlet_theme={}; urlet_lang={locale}", theme.key());
                let editor = app.get_as(&format!("/app/urls/{id}"), &cookie).await;
                assert_eq!(editor.status, StatusCode::OK);
                assert_eq!(editor.body.contains("id=\"preview-image-url\""), allow_url);
                assert_eq!(editor.body.contains("id=\"preview-image\""), allow_upload);
                assert!(!editor.body.contains("preview.policy."));
                let settings = app.get_as("/app/admin/settings", &cookie).await;
                assert_eq!(settings.status, StatusCode::OK);
                assert!(settings.body.contains("name=\"preview_image_policy\""));
                assert!(settings
                    .body
                    .contains(&format!("value=\"{policy}\" selected")));
                assert!(!settings
                    .body
                    .contains("admin.settings.preview_image_policy"));
            }
        }
    }
}

fn multipart(
    uri: &str,
    cookie: &str,
    fields: &[(&str, &str)],
    image: Option<&[u8]>,
) -> Request<Body> {
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(format!("--previewboundary\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    if let Some(bytes) = image {
        body.extend_from_slice(b"--previewboundary\r\nContent-Disposition: form-data; name=\"image\"; filename=\"untrusted.html\"\r\nContent-Type: text/html\r\n\r\n");
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"--previewboundary--\r\n");
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(
            "content-type",
            "multipart/form-data; boundary=previewboundary",
        )
        .header("cookie", cookie)
        .header("origin", "http://localhost:8080")
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn custom_cards_escape_content_and_leave_browser_redirects_intact() {
    let (app, cookie, id) = setup().await;
    let response = patch(&app, &cookie, id, json!({"title":"<b>Public & title</b>", "description":"Description \"quoted\"\nnext line", "image_url":"https://images.example/card.png?a=1&b=2"})).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(response.json()["image_uploaded"], false);
    let response = crawl(&app, "Mozilla/5.0 (compatible; Discordbot/2.0)").await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert!(response.body.contains("og:image"));
    assert!(response.body.contains("og:description"));
    assert!(response.body.contains("summary_large_image"));
    assert!(!response.body.contains("<b>Public"));
    assert!(response.body.contains("&lt;b&gt;Public"));
    for private in [
        "destination.example",
        "private-path",
        "private management title",
        "private notes",
    ] {
        assert!(!response.body.contains(private));
    }
    assert_eq!(
        response.header("cache-control").as_deref(),
        Some("no-store")
    );
    assert!(response
        .headers
        .get_all("vary")
        .iter()
        .any(|s| s.to_str().unwrap().contains("User-Agent")));
    assert_eq!(
        urlet::db::urls::find_by_id(&app.state.db, id)
            .await
            .unwrap()
            .unwrap()
            .click_count,
        0
    );
    let browser = app.get("/share").await;
    assert_eq!(browser.status, StatusCode::FOUND);
    assert_eq!(
        browser.header("location").as_deref(),
        Some("https://destination.example/private-path")
    );
}

#[tokio::test]
async fn previews_are_opt_in_and_narrowly_match_social_crawlers() {
    let (app, cookie, id) = setup().await;
    assert_eq!(
        crawl(&app, "Discordbot/2.0").await.status,
        StatusCode::FOUND
    );
    patch(
        &app,
        &cookie,
        id,
        json!({"description":"public description"}),
    )
    .await;
    for ua in [
        "Discordbot/2.0",
        "Twitterbot/1.0",
        "facebookexternalhit/1.1",
        "TelegramBot",
        "Slackbot-LinkExpanding",
        "Line-Poker/1.0",
        "LinkedInBot",
    ] {
        assert_eq!(crawl(&app, ua).await.status, StatusCode::OK, "{ua}");
    }
    for ua in ["curl/8.0", "Googlebot", "uptime-monitor", "Mozilla/5.0", ""] {
        assert_eq!(crawl(&app, ua).await.status, StatusCode::FOUND, "{ua}");
    }
}

#[tokio::test]
async fn password_links_only_publish_explicit_preview_data() {
    let (app, cookie, id) = setup().await;
    let protected = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{id}"),
            json!({"password":"supersecret"}),
            &cookie,
        ))
        .await;
    assert_eq!(protected.status, StatusCode::OK, "{}", protected.body);
    let response = crawl(&app, "Discordbot").await;
    assert!(!response.body.contains("og:description"));
    patch(
        &app,
        &cookie,
        id,
        json!({"description":"Deliberately public"}),
    )
    .await;
    let response = crawl(&app, "Discordbot").await;
    assert!(response.body.contains("Deliberately public"));
    for private in [
        "private management title",
        "private notes",
        "destination.example",
    ] {
        assert!(!response.body.contains(private));
    }
    let browser = app.get("/share").await;
    assert!(browser.body.contains("password"));
    assert!(!browser.body.contains("Deliberately public"));
}

#[tokio::test]
async fn upload_replacement_revokes_old_keys_and_survives_database_reopen() {
    let (app, cookie, id) = setup().await;
    let uri = format!("/api/v1/urls/{id}/preview/image");
    patch(&app, &cookie, id, json!({"title":"Saved title"})).await;
    let response = app.send(multipart(&uri, &cookie, &[], Some(&png()))).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    let value = response.json();
    assert_eq!(value["image_uploaded"], true);
    assert_eq!(value["title"], "Saved title");
    let old_url = value["image_url"].as_str().unwrap();
    let path = old_url.strip_prefix(&app.state.config.base_url).unwrap();
    let image = app.get(path).await;
    assert_eq!(image.status, StatusCode::OK, "{}", image.body);
    assert_eq!(image.header("content-type").as_deref(), Some("image/png"));
    assert_eq!(
        image.header("x-content-type-options").as_deref(),
        Some("nosniff")
    );
    assert_eq!(image.header("cache-control").as_deref(), Some("no-store"));
    let pool = urlet::db::init(&app.state.config).await.unwrap();
    assert!(urlet::db::url_previews::find(&pool, id)
        .await
        .unwrap()
        .unwrap()
        .image_key
        .is_some());
    pool.close().await;
    let response = app.send(multipart(&uri, &cookie, &[], Some(&png()))).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_ne!(response.json()["image_url"], old_url);
    assert_eq!(app.get(path).await.status, StatusCode::NOT_FOUND);
    let new_value = response.json();
    let new_path = new_value["image_url"]
        .as_str()
        .unwrap()
        .strip_prefix(&app.state.config.base_url)
        .unwrap();
    patch(
        &app,
        &cookie,
        id,
        json!({"image_url":"https://images.example/new.png"}),
    )
    .await;
    assert_eq!(app.get(new_path).await.status, StatusCode::NOT_FOUND);
    assert_eq!(
        app.send(delete_as(&format!("/api/v1/urls/{id}/preview"), &cookie))
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(crawl(&app, "Discordbot").await.status, StatusCode::FOUND);
}

#[tokio::test]
async fn failed_changes_are_atomic_and_images_are_bounded() {
    let (app, cookie, id) = setup().await;
    patch(&app, &cookie, id, json!({"title":"Original"})).await;
    for bad in [
        "javascript:alert(1)",
        "http://images.example/a.png",
        "https://name:secret@images.example/a.png",
        "https://images.example/a b.png",
    ] {
        let response = patch(
            &app,
            &cookie,
            id,
            json!({"title":"Changed", "image_url":bad}),
        )
        .await;
        assert_eq!(
            response.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{bad}: {}",
            response.body
        );
    }
    assert_eq!(
        patch(&app, &cookie, id, json!({"title":"a".repeat(201)}))
            .await
            .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        patch(&app, &cookie, id, json!({"description":"a".repeat(1001)}))
            .await
            .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let uri = format!("/api/v1/urls/{id}/preview/image");
    for invalid in [
        include_bytes!("fixtures/previews/oversized-frame.gif").to_vec(),
        include_bytes!("fixtures/previews/outside-second-frame.gif").to_vec(),
        include_bytes!("fixtures/previews/oversized-second-frame.gif").to_vec(),
        b"<svg><script>bad</script></svg>".to_vec(),
        vec![0; 2 * 1024 * 1024 + 1],
        b"\x89PNG\r\n\x1a\ntruncated".to_vec(),
    ] {
        assert_eq!(
            app.send(multipart(&uri, &cookie, &[], Some(&invalid)))
                .await
                .status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(
        urlet::db::url_previews::find(&app.state.db, id)
            .await
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("Original")
    );
    let too_wide = image::DynamicImage::new_rgb8(4097, 1);
    let mut bytes = Cursor::new(Vec::new());
    too_wide
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    assert!(urlet::services::url_preview::upload(bytes.into_inner())
        .await
        .is_err());
}

#[tokio::test]
async fn ownership_authentication_csrf_and_body_limits_are_enforced() {
    let (app, cookie, id) = setup().await;
    let (other, _) = app.register("other@example.com", "supersecret").await;
    let uri = format!("/api/v1/urls/{id}/preview");
    assert_eq!(app.get(&uri).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(app.get_as(&uri, &other).await.status, StatusCode::NOT_FOUND);
    assert_eq!(
        patch(&app, &other, id, json!({"title":"Other"}))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(delete_as(&uri, &other)).await.status,
        StatusCode::NOT_FOUND
    );
    let upload = format!("{uri}/image");
    assert_eq!(
        app.send(multipart(&upload, &other, &[], Some(&png())))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let mut request = multipart(&upload, &cookie, &[], Some(&png()));
    request
        .headers_mut()
        .insert("origin", "https://evil.example".parse().unwrap());
    assert_eq!(app.send(request).await.status, StatusCode::FORBIDDEN);
    let oversized = vec![0; urlet::services::url_preview::MAX_BODY_BYTES + 1];
    assert!(app
        .send(multipart(&upload, &cookie, &[], Some(&oversized)))
        .await
        .status
        .is_client_error());
    assert!(urlet::db::url_previews::find(&app.state.db, id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn lifecycle_checks_precede_metadata_and_media_delivery() {
    let (app, cookie, id) = setup().await;
    patch(&app, &cookie, id, json!({"title":"Public"})).await;
    let value = app
        .send(multipart(
            &format!("/api/v1/urls/{id}/preview/image"),
            &cookie,
            &[],
            Some(&png()),
        ))
        .await
        .json();
    let path = value["image_url"]
        .as_str()
        .unwrap()
        .strip_prefix(&app.state.config.base_url)
        .unwrap();
    for (update, expected) in [
        (json!({"status":"disabled"}), StatusCode::NOT_FOUND),
        (json!({"status":"active", "max_clicks":1}), StatusCode::GONE),
    ] {
        app.send(patch_json_as(
            &format!("/api/v1/urls/{id}"),
            update,
            &cookie,
        ))
        .await;
        if expected == StatusCode::GONE {
            sqlx::query("UPDATE urls SET click_count=1 WHERE id=?")
                .bind(id)
                .execute(&app.state.db)
                .await
                .unwrap();
        }
        let response = crawl(&app, "Discordbot").await;
        assert_eq!(response.status, expected, "{}", response.body);
        assert!(!response.body.contains("og:title"));
        let blocked = app.get(path).await;
        assert_eq!(blocked.status, StatusCode::NOT_FOUND);
        assert_eq!(blocked.header("cache-control").as_deref(), Some("no-store"));
    }
    sqlx::query("UPDATE urls SET click_count=0,max_clicks=NULL,expires_at='2000-01-01T00:00:00Z' WHERE id=?").bind(id).execute(&app.state.db).await.unwrap();
    assert_eq!(crawl(&app, "Discordbot").await.status, StatusCode::GONE);
    sqlx::query("UPDATE urls SET expires_at=NULL WHERE id=?")
        .bind(id)
        .execute(&app.state.db)
        .await
        .unwrap();
    assert!(urlet::db::url_previews::find(&app.state.db, id)
        .await
        .unwrap()
        .is_some());
    let deleted = app
        .send(delete_as(&format!("/api/v1/urls/{id}"), &cookie))
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert_eq!(
        crawl(&app, "Discordbot").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(app.get(path).await.status, StatusCode::NOT_FOUND);
    assert!(urlet::db::url_previews::find(&app.state.db, id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn signed_out_form_returns_to_login_with_link_detail_destination() {
    let (app, _, id) = setup().await;
    let response = app
        .send(multipart(
            &format!("/app/urls/{id}/preview"),
            "",
            &[("title", "Unsaved")],
            None,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    assert_eq!(
        response.header("location").as_deref(),
        Some(format!("/app/login?next=%2Fapp%2Furls%2F{id}").as_str())
    );
    assert!(urlet::db::url_previews::find(&app.state.db, id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn account_deletion_reclaims_previews_and_preserves_other_owners() {
    let (app, admin_cookie, admin_url_id) = setup().await;
    patch(
        &app,
        &admin_cookie,
        admin_url_id,
        json!({"title":"Keep me"}),
    )
    .await;
    let (member_cookie, member) = app.register("member@example.com", "supersecret").await;
    let member_id = member["user"]["id"].as_i64().unwrap();
    let created = app
        .post_json_as(
            "/api/v1/urls",
            json!({"target_url":"https://example.com", "slug":"member-share"}),
            &member_cookie,
        )
        .await;
    let id = created.json()["id"].as_i64().unwrap();
    let uploaded = app
        .send(multipart(
            &format!("/api/v1/urls/{id}/preview/image"),
            &member_cookie,
            &[],
            Some(&png()),
        ))
        .await;
    assert_eq!(uploaded.status, StatusCode::OK, "{}", uploaded.body);
    let value = uploaded.json();
    let path = value["image_url"]
        .as_str()
        .unwrap()
        .strip_prefix(&app.state.config.base_url)
        .unwrap();
    assert_eq!(app.get(path).await.status, StatusCode::OK);
    let deleted = app
        .send(delete_as(
            &format!("/api/v1/admin/users/{member_id}"),
            &admin_cookie,
        ))
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);
    assert!(urlet::db::url_previews::find(&app.state.db, id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(app.get(path).await.status, StatusCode::NOT_FOUND);
    assert_eq!(
        urlet::db::url_previews::find(&app.state.db, admin_url_id)
            .await
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("Keep me")
    );
}

#[tokio::test]
async fn form_saves_both_image_sources_preserves_upload_and_retains_text_after_errors() {
    let (app, cookie, id) = setup().await;
    let uri = format!("/app/urls/{id}/preview");
    let fields = [
        ("title", "Public title"),
        ("description", "Public content"),
        ("image_mode", "url"),
        ("image_url", "https://images.example/a.png"),
    ];
    assert_eq!(
        app.send(multipart(&uri, &cookie, &fields, None))
            .await
            .status,
        StatusCode::SEE_OTHER
    );
    let fields = [
        ("title", "Uploaded title"),
        ("description", "Uploaded content"),
        ("image_mode", "upload"),
    ];
    assert_eq!(
        app.send(multipart(&uri, &cookie, &fields, Some(&png())))
            .await
            .status,
        StatusCode::SEE_OTHER
    );
    let old = urlet::db::url_previews::find(&app.state.db, id)
        .await
        .unwrap()
        .unwrap();
    let fields = [
        ("title", "Edited title"),
        ("description", ""),
        ("image_mode", "keep"),
    ];
    assert_eq!(
        app.send(multipart(&uri, &cookie, &fields, None))
            .await
            .status,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        urlet::db::url_previews::find(&app.state.db, id)
            .await
            .unwrap()
            .unwrap()
            .image_key,
        old.image_key
    );
    let fields = [
        ("title", "Preserve this input"),
        ("description", "Retry me"),
        ("image_mode", "upload"),
    ];
    let error = app
        .send(multipart(&uri, &cookie, &fields, Some(b"invalid")))
        .await;
    assert_eq!(error.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(error.body.contains("Preserve this input"));
    assert!(error.body.contains("Retry me"));
    assert!(error.body.contains("role=\"alert\""));
    let fields = [("title", ""), ("description", ""), ("image_mode", "none")];
    assert_eq!(
        app.send(multipart(&uri, &cookie, &fields, None))
            .await
            .status,
        StatusCode::SEE_OTHER
    );
    assert!(urlet::db::url_previews::find(&app.state.db, id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn link_editor_and_retry_form_render_in_all_themes_and_locales() {
    let (app, cookie, id) = setup().await;
    for theme in ["verdant", "aurora", "paper", "signal", "gilded", "swiss"] {
        for lang in ["en", "zh-Hant", "zh-Hans"] {
            let cookies = format!("{cookie}; urlet_theme={theme}; urlet_lang={lang}");
            let response = app.get_as(&format!("/app/urls/{id}"), &cookies).await;
            assert_eq!(
                response.status,
                StatusCode::OK,
                "{theme} {lang}: {}",
                response.body
            );
            assert!(response.body.contains("id=\"preview-image\""));
            assert!(!response.body.contains("preview.heading"));
            let response = app
                .send(multipart(
                    &format!("/app/urls/{id}/preview"),
                    &cookies,
                    &[("title", "Retained"), ("image_mode", "upload")],
                    None,
                ))
                .await;
            assert_eq!(
                response.status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{theme} {lang}: {}",
                response.body
            );
            assert!(response.body.contains("Retained"));
        }
    }
}

#[tokio::test]
async fn supported_rasters_are_validated_from_content_and_audited_without_content() {
    let (app, cookie, id) = setup().await;
    let uri = format!("/api/v1/urls/{id}/preview/image");
    let mut images = vec![
        (png(), "image/png"),
        (
            include_bytes!("fixtures/previews/animated.gif").to_vec(),
            "image/gif",
        ),
    ];
    for (format, mime) in [
        (image::ImageFormat::Jpeg, "image/jpeg"),
        (image::ImageFormat::Gif, "image/gif"),
    ] {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut bytes, format)
            .unwrap();
        images.push((bytes.into_inner(), mime));
    }
    for bytes in [
        include_bytes!("fixtures/previews/animated-lossy.webp").as_slice(),
        include_bytes!("fixtures/previews/animated-lossless.webp").as_slice(),
        include_bytes!("fixtures/previews/extended.webp").as_slice(),
    ] {
        images.push((bytes.to_vec(), "image/webp"));
    }
    for (bytes, mime) in images {
        let response = app.send(multipart(&uri, &cookie, &[], Some(&bytes))).await;
        assert_eq!(response.status, StatusCode::OK, "{mime}: {}", response.body);
        let value = response.json();
        let path = value["image_url"]
            .as_str()
            .unwrap()
            .strip_prefix(&app.state.config.base_url)
            .unwrap();
        let media = app.get(path).await;
        assert_eq!(media.status, StatusCode::OK);
        assert_eq!(media.header("content-type").as_deref(), Some(mime));
    }
    let before: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(id),0) FROM audit_logs")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    let changed = patch(
        &app,
        &cookie,
        id,
        json!({"title":"Do not audit the content"}),
    )
    .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    assert_eq!(changed.json()["title"], "Do not audit the content");
    let metadata: String = sqlx::query_scalar(
        "SELECT metadata FROM audit_logs WHERE action='url.updated' AND target_type='url' AND target_id=? AND id>?",
    )
    .bind(id.to_string())
    .bind(before)
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&metadata).unwrap(),
        json!({"fields":["preview"]})
    );
}

#[tokio::test]
async fn custom_domains_splash_and_password_unlock_keep_existing_browser_behavior() {
    let (app, cookie, id) = setup().await;
    let domain_id = app.add_domain("go.example.com", "active").await;
    let response = app.send(patch_json_as(&format!("/api/v1/urls/{id}"), json!({"domain":"go.example.com", "redirect_mode":"splash", "splash_title":"Browser splash"}), &cookie)).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    patch(&app, &cookie, id, json!({"title":"Public card"})).await;
    let value = app
        .send(multipart(
            &format!("/api/v1/urls/{id}/preview/image"),
            &cookie,
            &[],
            Some(&png()),
        ))
        .await
        .json();
    let path = value["image_url"]
        .as_str()
        .unwrap()
        .strip_prefix(&app.state.config.base_url)
        .unwrap();
    let request = || {
        Request::builder()
            .uri("/share")
            .header("host", "go.example.com")
            .header("user-agent", "Discordbot")
            .body(Body::empty())
            .unwrap()
    };
    let card = app.send(request()).await;
    assert_eq!(card.status, StatusCode::OK);
    assert!(card.body.contains("http://go.example.com/share"));
    assert!(card.body.contains("Public card"));
    assert!(!card.body.contains("Browser splash"));
    let browser = app.get_with_host("/share", "go.example.com").await;
    assert!(browser.body.contains("Browser splash"));
    assert!(!browser.body.contains("Public card"));
    assert_eq!(app.get(path).await.status, StatusCode::OK);
    sqlx::query("UPDATE domains SET status='disabled' WHERE id=?")
        .bind(domain_id)
        .execute(&app.state.db)
        .await
        .unwrap();
    app.state.domains.reload(&app.state.db).await.unwrap();
    assert_eq!(app.send(request()).await.status, StatusCode::NOT_FOUND);
    let blocked = app.get(path).await;
    assert_eq!(blocked.status, StatusCode::NOT_FOUND);
    assert_eq!(blocked.header("cache-control").as_deref(), Some("no-store"));
    sqlx::query("UPDATE domains SET status='active' WHERE id=?")
        .bind(domain_id)
        .execute(&app.state.db)
        .await
        .unwrap();
    app.state.domains.reload(&app.state.db).await.unwrap();
    assert_eq!(app.get(path).await.status, StatusCode::OK);
    let response = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{id}"),
            json!({"domain":"localhost:8080", "password":"supersecret"}),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    let unlocked = app
        .send(common::post_json(
            "/share/unlock",
            json!({"password":"supersecret"}),
        ))
        .await;
    assert_eq!(unlocked.status, StatusCode::OK, "{}", unlocked.body);
    let cookie_name = urlet::services::unlock::cookie_name("localhost:8080", "share");
    let unlocked_cookie = unlocked.cookie_named(&cookie_name).unwrap();
    let followed = app.get_as("/share", &unlocked_cookie).await;
    assert_eq!(followed.status, StatusCode::FOUND);
    assert_eq!(
        followed.header("location").as_deref(),
        Some("https://destination.example/private-path")
    );
    assert_eq!(
        followed.header("cache-control").as_deref(),
        Some("no-store")
    );
}
