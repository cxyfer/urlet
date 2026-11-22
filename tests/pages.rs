//! Integration tests for the server rendered pages.

mod common;

use axum::http::StatusCode;
use common::{post_form, post_form_as, TestApp};
use serde_json::json;

async fn login(app: &TestApp, email: &str, password: &str) -> String {
    let response = app
        .send(post_form(
            "/app/login",
            &format!("email={email}&password={password}"),
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_eq!(
        response.status,
        StatusCode::SEE_OTHER,
        "login failed: {}",
        response.body
    );
    response.session_cookie().expect("session cookie")
}

#[tokio::test]
async fn landing_page_serves_anonymous_visitors() {
    let app = TestApp::spawn().await;
    let response = app.get("/").await;
    assert_eq!(response.status, StatusCode::OK);
    // English is the default when neither a cookie nor Accept-Language is sent.
    assert_eq!(response.header("content-language").as_deref(), Some("en"));
    assert!(response.body.contains("<html lang=\"en\">"));
    assert!(response.body.contains("Short links for teams"));
    // The public page carries a description for search engines and previews.
    assert!(response
        .body
        .contains("<meta name=\"description\" content=\"A lightweight, self-hosted"));
    // Both entry points into the application are offered.
    assert!(response.body.contains("href=\"/app/register\""));
    assert!(response.body.contains("href=\"/app/login\""));
    // Anonymous visitors keep the brand on the landing page.
    assert!(response.body.contains("class=\"brand\" href=\"/\""));
    assert!(!response.body.contains("Open dashboard"));
    // One `h1`, six feature cards and a single language switcher.
    assert_eq!(response.body.matches("<h1>").count(), 1);
    assert_eq!(response.body.matches("landing-feature").count(), 6);
    assert_eq!(response.body.matches("data-lang-switcher").count(), 1);
    // The footer attributes the instance to the project it was built from. The
    // mark is inline SVG, because the CSP allows no remote image or icon font.
    assert!(response
        .body
        .contains(&format!("href=\"{}\"", urlet::REPOSITORY)));
    assert!(response.body.contains("class=\"footer-repo-icon\""));
    assert!(response.body.contains("rel=\"noopener\""));
}

#[tokio::test]
async fn landing_page_serves_signed_in_visitors() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let response = app.get_as("/", &cookie).await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.body.contains("Open dashboard"));
    // The top bar offers the dashboard and the sign-out form instead.
    assert!(response.body.contains("class=\"brand\" href=\"/app\""));
    assert!(response.body.contains("href=\"/app\">Dashboard</a>"));
    assert!(response.body.contains("user@example.com"));
    assert!(response.body.contains("action=\"/app/logout\""));
    // Signed-in visitors get the dashboard instead of the sign-up funnel.
    assert!(!response.body.contains("Create account"));
}

#[tokio::test]
async fn landing_page_is_no_store_for_anonymous_and_signed_in_visitors() {
    let app = TestApp::spawn().await;
    let anonymous = app.get("/").await;
    assert_eq!(anonymous.status, StatusCode::OK);
    assert_eq!(
        anonymous.header("cache-control").as_deref(),
        Some("no-store")
    );

    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let signed_in = app.get_as("/", &cookie).await;
    assert_eq!(signed_in.status, StatusCode::OK);
    assert_eq!(
        signed_in.header("cache-control").as_deref(),
        Some("no-store")
    );
}

#[tokio::test]
async fn landing_page_treats_a_stale_session_as_anonymous() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    // A syntactically plausible but unknown cookie is simply ignored.
    let response = app.get_as("/", "urlet_session=not-a-real-token").await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.body.contains("Short links for teams"));
    assert!(!response.body.contains("Open dashboard"));
    assert!(!response.body.contains("user@example.com"));
    assert!(!response.body.contains("action=\"/app/logout\""));

    // So is a real session that has expired.
    sqlx::query("UPDATE sessions SET expires_at = $1")
        .bind(urlet::services::time::iso_in_hours(-1))
        .execute(&app.state.db)
        .await
        .expect("expire the session");

    let response = app.get_as("/", &cookie).await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.body.contains("Short links for teams"));
    assert!(!response.body.contains("Open dashboard"));
    assert!(!response.body.contains("user@example.com"));
    assert_eq!(response.body.matches("data-lang-switcher").count(), 1);
}

#[tokio::test]
async fn dashboard_requires_a_session() {
    let app = TestApp::spawn().await;
    let response = app.get("/app").await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert_eq!(response.header("location").as_deref(), Some("/app/login"));

    let response = app.get("/app/urls/1").await;
    assert_eq!(
        response.header("location").as_deref(),
        Some("/app/login?next=%2Fapp%2Furls%2F1")
    );
}

#[tokio::test]
async fn login_page_renders_and_redirects_authenticated_visitors() {
    let app = TestApp::spawn().await;
    let page = app.get("/app/login").await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Sign in"));
    assert!(page.body.contains("/assets/app.css"));
    assert!(page.body.contains("name=\"password\""));

    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let redirected = app.get_as("/app/login", &cookie).await;
    assert_eq!(redirected.status, StatusCode::SEE_OTHER);
    assert_eq!(redirected.header("location").as_deref(), Some("/app"));
}

#[tokio::test]
async fn registration_page_honours_the_registration_mode() {
    let app = TestApp::spawn().await;
    let open = app.get("/app/register").await;
    assert_eq!(open.status, StatusCode::OK);
    assert_eq!(open.header("cache-control").as_deref(), Some("no-store"));
    assert!(open.body.contains("Create account"));
    // Open registration still offers the invite field, but only as optional:
    // a code can pin the new account to a group.
    assert!(open.body.contains("name=\"invite_code\""));
    assert!(open
        .body
        .contains("Optional: if you were given a group invite code"));

    app.state
        .settings
        .update(&app.state.db, "registration_mode", "invite", None)
        .await
        .expect("switch to invite");
    let invite = app.get("/app/register?invite=K7M2QPX4RT").await;
    assert!(invite.body.contains("Invite code"));
    assert!(
        invite.body.contains("value=\"K7M2QPX4RT\""),
        "the link must prefill the code: {}",
        invite.body
    );

    app.state
        .settings
        .update(&app.state.db, "registration_mode", "closed", None)
        .await
        .expect("close registration");
    let closed = app.get("/app/register").await;
    assert!(closed.body.contains("Registration is currently closed"));
    assert!(!closed.body.contains("name=\"password\""));
}

#[tokio::test]
async fn an_invite_link_prefills_and_completes_registration() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("owner@example.com", "supersecret").await;
    app.state
        .settings
        .update(&app.state.db, "registration_mode", "invite", None)
        .await
        .expect("invite mode");

    let created = app
        .post_json_as(
            "/api/v1/admin/invites",
            json!({ "note": "linked", "max_uses": 1 }),
            &cookie,
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let body = created.json();
    let code = body["code"].as_str().expect("code").to_string();
    let link = body["link"].as_str().expect("link").to_string();
    let path = link
        .strip_prefix("http://localhost:8080")
        .expect("test base URL");

    let page = app.get(path).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(
        page.body.contains(&format!("value=\"{code}\"")),
        "the link must prefill the code: {}",
        page.body
    );

    let registered = app
        .send(post_form(
            "/app/register",
            &format!("email=invited@example.com&password=supersecret&invite_code={code}"),
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_eq!(
        registered.status,
        StatusCode::SEE_OTHER,
        "{}",
        registered.body
    );

    let listing = app.get_as("/api/v1/admin/invites", &cookie).await.json();
    assert_eq!(listing["items"][0]["used_count"], 1);
}

#[tokio::test]
async fn form_login_and_logout_round_trip() {
    let app = TestApp::spawn().await;
    app.register("user@example.com", "supersecret").await;

    let cookie = login(&app, "user@example.com", "supersecret").await;
    let dashboard = app.get_as("/app", &cookie).await;
    assert_eq!(dashboard.status, StatusCode::OK);
    assert!(dashboard.body.contains("Your links"));
    assert!(dashboard.body.contains("New short link"));

    let logout = app.send(post_form_as("/app/logout", "", &cookie)).await;
    assert_eq!(logout.status, StatusCode::SEE_OTHER);
    assert_eq!(
        logout.header("location").as_deref(),
        Some("/app/login?ok=logout")
    );
    assert_eq!(
        app.get_as("/app", &cookie).await.status,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn wrong_credentials_render_an_error() {
    let app = TestApp::spawn().await;
    app.register("user@example.com", "supersecret").await;

    let response = app
        .send(post_form(
            "/app/login",
            "email=user@example.com&password=wrong",
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.body.contains("Invalid email or password"));
}

#[tokio::test]
async fn registration_form_creates_and_signs_in() {
    let app = TestApp::spawn().await;
    let response = app
        .send(post_form(
            "/app/register",
            "email=new@example.com&password=supersecret&display_name=New+User",
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    let cookie = response.session_cookie().expect("session cookie");

    let dashboard = app.get_as("/app", &cookie).await;
    assert_eq!(dashboard.status, StatusCode::OK);
    assert!(dashboard.body.contains("New User"));
}

#[tokio::test]
async fn short_link_forms_create_update_and_delete() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let created = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fexample.com%2Flanding&slug=promo\
             &title=Launch&tags=campaign%2C+docs&expires_at=2030-01-01+12%3A00&max_clicks=25",
            &cookie,
        ))
        .await;
    assert_eq!(created.status, StatusCode::SEE_OTHER, "{}", created.body);
    let location = created.header("location").expect("location");
    assert!(location.starts_with("/app/urls/"), "{location}");
    assert!(location.ends_with("?ok=created"));

    let detail = app.get_as(&location, &cookie).await;
    assert_eq!(detail.status, StatusCode::OK);
    assert!(detail.body.contains("promo"));
    assert!(detail.body.contains("example.com/landing"));
    assert!(detail.body.contains("Launch"));
    assert!(detail.body.contains("campaign"));
    assert!(detail.body.contains("0 / 25"), "click limit is summarised");
    let editor_path = location.replace("?ok=created", "/edit");
    assert!(
        detail.body.contains(&format!("href=\"{editor_path}\"")),
        "the statistics page links to the editor"
    );
    let editor = app.get_as(&editor_path, &cookie).await;
    assert_eq!(editor.status, StatusCode::OK, "{}", editor.body);
    assert!(editor.body.contains("2030-01-01"), "expiry is editable");
    assert!(
        editor.body.contains("value=\"25\""),
        "click limit is editable"
    );

    let id: i64 = location
        .trim_start_matches("/app/urls/")
        .split('?')
        .next()
        .expect("id")
        .parse()
        .expect("numeric id");

    // Update: rename, add a password and clear the click limit.
    let updated = app
        .send(post_form_as(
            &format!("/app/urls/{id}"),
            "target_url=https%3A%2F%2Fexample.com%2Fupdated&slug=promo&title=Renamed\
             &tags=campaign&password=linkpass&clear_max_clicks=on&status=active&redirect_type=301",
            &cookie,
        ))
        .await;
    assert_eq!(updated.status, StatusCode::SEE_OTHER, "{}", updated.body);
    assert_eq!(
        updated.header("location").as_deref(),
        Some(format!("/app/urls/{id}?ok=updated").as_str())
    );

    let detail = app.get_as(&format!("/app/urls/{id}"), &cookie).await;
    assert!(detail.body.contains("Renamed"));
    assert!(detail.body.contains("example.com/updated"));
    assert!(detail.body.contains("Password protected"));
    let editor = app.get_as(&format!("/app/urls/{id}/edit"), &cookie).await;
    assert!(editor.body.contains("currently password protected"));

    // A cross-site form submission must be refused.
    let blocked = app
        .send(post_form(
            &format!("/app/urls/{id}/delete"),
            "",
            Some(&cookie),
            Some("https://evil.example"),
        ))
        .await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);

    let deleted = app
        .send(post_form_as(&format!("/app/urls/{id}/delete"), "", &cookie))
        .await;
    assert_eq!(deleted.status, StatusCode::SEE_OTHER);
    assert_eq!(
        deleted.header("location").as_deref(),
        Some("/app?ok=deleted")
    );
    let list = app.get_as("/app", &cookie).await;
    assert!(list.body.contains("No links match this filter") || !list.body.contains("promo"));
}

#[tokio::test]
async fn create_form_reports_errors_with_inline_messages_and_retains_values() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let lang = urlet::i18n::Lang::En;

    // Validation failures re-render the form in-place (422) with specific
    // error messages, expanded sections, and preserved submitted values.
    let invalid_target = app
        .send(post_form_as(
            "/app/urls",
            "target_url=javascript%3Aalert%281%29&slug=kept-slug",
            &cookie,
        ))
        .await;
    assert_eq!(invalid_target.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(invalid_target.body.contains(lang.t("flash.err.target")));
    assert!(invalid_target.body.contains("value=\"kept-slug\""));

    let reserved = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fa.example&slug=api",
            &cookie,
        ))
        .await;
    assert_eq!(reserved.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(reserved.body.contains(lang.t("flash.err.slug")));
    assert!(reserved.body.contains("value=\"https://a.example\""));
    assert!(reserved.body.contains("value=\"api\""));

    let past_expiry = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fa.example&expires_at=2000-01-01",
            &cookie,
        ))
        .await;
    assert_eq!(past_expiry.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(past_expiry.body.contains(lang.t("flash.err.expiry")));
    assert!(past_expiry.body.contains("value=\"2000-01-01\""));
    assert!(past_expiry
        .body
        .contains("<details class=\"advanced link-section\" open>"));

    let short_password = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fa.example&password=ab",
            &cookie,
        ))
        .await;
    assert_eq!(short_password.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(short_password
        .body
        .contains(lang.t("flash.err.password_rules")));
    assert!(short_password
        .body
        .contains("<details class=\"advanced link-section\" open>"));

    let first = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fa.example&slug=taken",
            &cookie,
        ))
        .await;
    assert_eq!(first.status, StatusCode::SEE_OTHER);
    let duplicate = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fb.example&slug=taken",
            &cookie,
        ))
        .await;
    assert_eq!(duplicate.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(duplicate.body.contains("already taken"));
    assert!(duplicate.body.contains("value=\"https://b.example\""));
    assert!(duplicate.body.contains("value=\"taken\""));

    // Edit also maps advanced field errors to specific messages and opens Advanced settings.
    let id: i64 = first
        .header("location")
        .unwrap()
        .trim_start_matches("/app/urls/")
        .split('?')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let bad_clicks = app
        .send(post_form_as(
            &format!("/app/urls/{id}"),
            "target_url=https%3A%2F%2Fa.example&max_clicks=0",
            &cookie,
        ))
        .await;
    assert_eq!(bad_clicks.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(bad_clicks.body.contains(lang.t("flash.err.max_clicks")));
    assert!(bad_clicks
        .body
        .contains("<details class=\"advanced link-section\" open>"));
}

#[tokio::test]
async fn dashboard_filters_and_flash_banners() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    app.send(post_form_as(
        "/app/urls",
        "target_url=https%3A%2F%2Falpha.example&slug=alpha&tags=work",
        &cookie,
    ))
    .await;

    let filtered = app
        .get_as("/app?q=alpha&tag=work&sort=clicks", &cookie)
        .await;
    assert_eq!(filtered.status, StatusCode::OK);
    assert!(filtered.body.contains("value=\"alpha\""));

    let empty = app.get_as("/app?q=nomatch", &cookie).await;
    assert!(empty.body.contains("No links match this filter"));

    // Unknown flash codes are ignored rather than rendered.
    let unknown = app.get_as("/app?ok=%3Cscript%3E", &cookie).await;
    assert!(!unknown.body.contains("<script>alert"));
}

#[tokio::test]
async fn account_page_lists_sessions_and_issues_tokens() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let page = app.get_as("/app/account", &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("API tokens"));
    assert!(page.body.contains("this device"));

    let created = app
        .send(post_form_as(
            "/app/account/tokens",
            "name=ci+pipeline&expires_at=2030-01-01",
            &cookie,
        ))
        .await;
    assert_eq!(created.status, StatusCode::OK);
    assert!(created.body.contains("Copy this token now"));
    assert!(created.body.contains("ur_"));

    // The plain token only exists in that response; the list shows a prefix.
    let page = app.get_as("/app/account", &cookie).await;
    assert!(page.body.contains("ci pipeline"));
    assert!(!page.body.contains("Copy this token now"));
}

#[tokio::test]
async fn profile_and_password_forms_work() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let profile = app
        .send(post_form_as(
            "/app/account",
            "display_name=Ada+Lovelace",
            &cookie,
        ))
        .await;
    assert_eq!(
        profile.header("location").as_deref(),
        Some("/app/account?ok=profile")
    );
    let page = app.get_as("/app/account", &cookie).await;
    assert!(page.body.contains("Ada Lovelace"));

    let wrong = app
        .send(post_form_as(
            "/app/account/password",
            "current_password=nope&new_password=brandnewpass",
            &cookie,
        ))
        .await;
    assert_eq!(
        wrong.header("location").as_deref(),
        Some("/app/account?err=password")
    );

    let changed = app
        .send(post_form_as(
            "/app/account/password",
            "current_password=supersecret&new_password=brandnewpass",
            &cookie,
        ))
        .await;
    assert_eq!(
        changed.header("location").as_deref(),
        Some("/app/account?ok=password")
    );
    assert!(app
        .send(post_form(
            "/app/login",
            "email=user@example.com&password=brandnewpass",
            None,
            Some("http://localhost:8080"),
        ))
        .await
        .session_cookie()
        .is_some());
}

#[tokio::test]
async fn revoking_the_current_session_signs_out() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let sessions = app.get_as("/api/v1/auth/sessions", &cookie).await.json();
    let current = sessions["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|session| session["current"] == true)
        .expect("current session")["id"]
        .as_i64()
        .expect("id");

    let response = app
        .send(post_form_as(
            &format!("/app/account/sessions/{current}/delete"),
            "",
            &cookie,
        ))
        .await;
    assert_eq!(
        response.header("location").as_deref(),
        Some("/app/login?ok=logout")
    );
    assert_eq!(
        app.get_as("/app", &cookie).await.status,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn tags_are_rendered_in_list_views() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    for (slug, tags) in [("first", "campaign,docs"), ("second", "campaign")] {
        let response = app
            .send(post_form_as(
                "/app/urls",
                &format!(
                    "target_url=https%3A%2F%2F{slug}.example&slug={slug}&tags={}",
                    tags.replace(',', "%2C")
                ),
                &cookie,
            ))
            .await;
        assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    }

    // The dashboard and the JSON list both resolve tags for many links at once.
    let dashboard = app.get_as("/app", &cookie).await;
    assert!(
        dashboard.body.contains("campaign"),
        "dashboard shows the tag"
    );
    assert!(dashboard.body.contains("docs"), "dashboard shows every tag");

    let list = app.get_as("/api/v1/urls", &cookie).await.json();
    let first = list["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["slug"] == "first")
        .expect("first link");
    assert_eq!(first["tags"], serde_json::json!(["campaign", "docs"]));
    let second = list["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["slug"] == "second")
        .expect("second link");
    assert_eq!(second["tags"], serde_json::json!(["campaign"]));

    // Tag filtering still works and reports usage counts. (The "top links"
    // card is intentionally not filtered, so assert through the JSON API.)
    let filtered = app.get_as("/api/v1/urls?tag=docs", &cookie).await.json();
    assert_eq!(filtered["total"], 1);
    assert_eq!(filtered["items"][0]["slug"], "first");

    let tags = app.get_as("/api/v1/tags", &cookie).await.json();
    let campaign = tags["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|tag| tag["name"] == "campaign")
        .expect("campaign tag");
    assert_eq!(campaign["url_count"], 2);
}

#[tokio::test]
async fn json_api_still_works_alongside_the_pages() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let created = app
        .send(common::post_json_as(
            "/api/v1/urls",
            json!({ "target_url": "https://example.com" }),
            &cookie,
        ))
        .await;
    assert_eq!(created.status, StatusCode::CREATED);

    let tokens = app.get_as("/api/v1/tokens", &cookie).await;
    assert_eq!(tokens.status, StatusCode::OK);
    assert_eq!(tokens.json()["items"].as_array().expect("items").len(), 0);
}

#[tokio::test]
async fn landing_page_reports_the_servers_own_readouts() {
    let app = TestApp::spawn().await;
    let response = app.get("/").await;
    assert_eq!(response.status, StatusCode::OK);

    // The strip at the end of the page is the only place that reports the
    // instance, and it only reports what this request could read: the clock,
    // the zone it was read in, the build and the process uptime.
    assert!(response.body.contains("data-server-clock"));
    assert!(response.body.contains("data-epoch-ms=\""));
    // The test configuration pins the server zone to UTC.
    assert!(response.body.contains("data-zone=\"UTC\""));
    assert!(response.body.contains(">+00:00<"));
    assert!(response
        .body
        .contains(&format!(">v{}<", env!("CARGO_PKG_VERSION"))));
    assert!(response.body.contains("This server"));

    // The decorative specimen panel says so instead of pretending to measure.
    assert!(response.body.contains("Illustrative only"));

    // Values the process cannot produce must not come back: they were the
    // reason a visitor could not tell the drawing from the telemetry.
    for invented in [
        "0.982",
        "0.942",
        "99.9",
        ">SIG<",
        ">AMB<",
        ">SLO<",
        "01:06",
        "SYS · 01 / 06",
        "0.94 · 06",
    ] {
        assert!(
            !response.body.contains(invented),
            "landing page still prints {invented}"
        );
    }
}

#[tokio::test]
async fn quick_extend_offers_and_moves_only_the_limit_that_stopped_the_link() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let created = app
        .send(common::post_json_as(
            "/api/v1/urls",
            json!({ "target_url": "https://example.com", "slug": "spent", "max_clicks": 1 }),
            &cookie,
        ))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let id = created.json()["id"].as_i64().expect("id");
    sqlx::query("UPDATE urls SET click_count = 1 WHERE id = $1")
        .bind(id)
        .execute(&app.state.db)
        .await
        .expect("spend");

    // 只因點擊上限失效：只提供增加點擊數，不提供延長天數。
    let page = app.get_as(&format!("/app/urls/{id}"), &cookie).await;
    assert!(page.body.contains("name=\"clicks\" value=\"100\""));
    assert!(!page.body.contains("name=\"days\""));

    // 直接送出也不會替沒有到期日的連結加上一個。
    let refused = app
        .send(post_form_as(
            &format!("/app/urls/{id}/extend"),
            "days=30",
            &cookie,
        ))
        .await;
    assert_eq!(refused.status, StatusCode::SEE_OTHER);
    assert!(refused
        .header("location")
        .expect("location")
        .contains("err=invalid"));
    let expires: Option<String> = sqlx::query_scalar("SELECT expires_at FROM urls WHERE id = $1")
        .bind(id)
        .fetch_one(&app.state.db)
        .await
        .expect("expiry");
    assert_eq!(expires, None);

    let raised = app
        .send(post_form_as(
            &format!("/app/urls/{id}/extend"),
            "clicks=100",
            &cookie,
        ))
        .await;
    assert!(raised
        .header("location")
        .expect("location")
        .contains("ok=updated"));
    let cap: Option<i64> = sqlx::query_scalar("SELECT max_clicks FROM urls WHERE id = $1")
        .bind(id)
        .fetch_one(&app.state.db)
        .await
        .expect("cap");
    assert_eq!(cap, Some(101));
    let live = app.get_as(&format!("/app/urls/{id}"), &cookie).await;
    assert!(
        !live.body.contains("/extend\""),
        "a live link needs no extension"
    );
}

#[tokio::test]
async fn quick_extend_lifts_both_limits_when_both_were_reached() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let created = app
        .send(common::post_json_as(
            "/api/v1/urls",
            json!({ "target_url": "https://example.com", "slug": "both", "max_clicks": 1, "expires_at": "2030-01-01" }),
            &cookie,
        ))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let id = created.json()["id"].as_i64().expect("id");
    sqlx::query(
        "UPDATE urls SET click_count = 1, expires_at = '2000-01-01T00:00:00Z' WHERE id = $1",
    )
    .bind(id)
    .execute(&app.state.db)
    .await
    .expect("stop");
    assert_eq!(app.get("/both").await.status, StatusCode::GONE);

    // 每個按鈕都同時延長兩項限制；單獨延長其中一項無法讓連結恢復。
    let page = app.get_as(&format!("/app/urls/{id}"), &cookie).await;
    assert!(
        page.body.contains("+30 days · +100 clicks"),
        "{}",
        page.body
    );
    assert!(!page.body.contains("type=\"submit\" name=\"days\""));
    assert!(!page.body.contains("type=\"submit\" name=\"clicks\""));

    let revived = app
        .send(post_form_as(
            &format!("/app/urls/{id}/extend"),
            "days=30&clicks=100",
            &cookie,
        ))
        .await;
    assert!(revived
        .header("location")
        .expect("location")
        .contains("ok=updated"));
    assert_eq!(app.get("/both").await.status, StatusCode::FOUND);
}

#[tokio::test]
async fn sign_in_and_registration_keep_the_page_to_return_to() {
    let app = TestApp::spawn().await;
    let (admin, _) = app.register("admin@example.com", "supersecret").await;
    for theme in urlet::services::theme::ALL {
        let key = theme.key();
        let switched = app
            .send(common::put_json_as(
                "/api/v1/admin/themes",
                json!({ "enabled": [key], "default": key }),
                &admin,
            ))
            .await;
        assert_eq!(switched.status, StatusCode::OK, "{key}");
        // 互相切換登入與註冊時保留 next。
        let login = app.get("/app/login?next=%2Fapp%2Finvites").await;
        assert!(
            login
                .body
                .contains("href=\"/app/register?next=%2Fapp%2Finvites\""),
            "{key} login page drops next"
        );
        let register = app.get("/app/register?next=%2Fapp%2Finvites").await;
        assert!(
            register
                .body
                .contains("href=\"/app/login?next=%2Fapp%2Finvites\""),
            "{key} register page drops next"
        );
    }

    // 被拒絕的登入導回表單時也保留 next。
    app.register("held@example.com", "supersecret").await;
    sqlx::query("UPDATE users SET status = 'suspended' WHERE email_normalized = $1")
        .bind("held@example.com")
        .execute(&app.state.db)
        .await
        .expect("suspend");
    let denied = app
        .send(post_form(
            "/app/login",
            "email=held@example.com&password=supersecret&next=%2Fapp%2Finvites",
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_eq!(denied.status, StatusCode::SEE_OTHER);
    assert_eq!(
        denied.header("location").as_deref(),
        Some("/app/login?err=denied&next=%2Fapp%2Finvites")
    );
}

/// The figure printed under one dashboard card label.
fn stat_value(body: &str, label: &str) -> String {
    let after = body
        .split(&format!("<span class=\"stat-label\">{label}</span>"))
        .nth(1)
        .unwrap_or_else(|| panic!("no {label} card"));
    let value = after
        .split("<div class=\"stat-value\">")
        .nth(1)
        .expect("stat value");
    value.split('<').next().expect("value").trim().to_string()
}

#[tokio::test]
async fn dashboard_cards_and_quota_meter_follow_each_link_state() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let mut ids = Vec::new();
    for slug in ["live", "off", "capped", "lapsed"] {
        let created = app
            .send(common::post_json_as(
                "/api/v1/urls",
                json!({ "target_url": "https://example.com", "slug": slug }),
                &cookie,
            ))
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        ids.push(created.json()["id"].as_i64().expect("id"));
    }
    // 停用、達點擊上限、已過期各一，另有一條仍可轉址。
    let disabled = app
        .send(common::patch_json_as(
            &format!("/api/v1/urls/{}", ids[1]),
            json!({ "status": "disabled" }),
            &cookie,
        ))
        .await;
    assert_eq!(disabled.status, StatusCode::OK, "{}", disabled.body);
    sqlx::query("UPDATE urls SET max_clicks = 1, click_count = 1 WHERE id = $1")
        .bind(ids[2])
        .execute(&app.state.db)
        .await
        .expect("cap");
    sqlx::query("UPDATE urls SET expires_at = '2000-01-01T00:00:00Z' WHERE id = $1")
        .bind(ids[3])
        .execute(&app.state.db)
        .await
        .expect("expire");

    let page = app.get_as("/app", &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(stat_value(&page.body, "Links"), "4");
    assert_eq!(
        stat_value(&page.body, "Active"),
        "1",
        "a disabled link is not active"
    );
    assert_eq!(stat_value(&page.body, "Expired"), "2");
    // 預設配額 1000：4 條連結無條件進位為 1%，而不是固定的寬度。
    assert!(
        page.body
            .contains("class=\"meter-fill\" style=\"width: 1%\""),
        "the meter must show the share of the quota in use"
    );
}

#[tokio::test]
async fn account_form_errors_name_the_rule_that_failed() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let long_name = "a".repeat(61);
    // 每個帳號表單錯誤都要對應自己的規則，不能借用連結表單的訊息。
    for (path, body, code, message) in [
        (
            "/app/account/password",
            "current_password=supersecret&new_password=short".to_string(),
            "account_password_rules",
            "The password must be between 8 and 128 characters",
        ),
        (
            "/app/account/password",
            "current_password=supersecret&new_password=supersecret".to_string(),
            "password_same",
            "The new password must differ from the current one.",
        ),
        (
            "/app/account",
            format!("display_name={long_name}"),
            "display_name",
            "The display name must be a single line of at most 60 characters.",
        ),
        (
            "/app/account/tokens",
            format!("name={long_name}"),
            "token_name",
            "The token name is required",
        ),
    ] {
        let response = app.send(post_form_as(path, &body, &cookie)).await;
        let location = response.header("location").expect("location");
        assert_eq!(
            location,
            format!("/app/account?err={code}"),
            "{path} {body}"
        );
        let page = app.get_as(&location, &cookie).await;
        assert!(page.body.contains(message), "{code}: {}", page.body);
        assert!(!page.body.contains("The link password"), "{code}");
    }
}
