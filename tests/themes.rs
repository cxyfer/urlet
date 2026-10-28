//! Theme catalogue, switching and permission integration tests.

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use common::{get_as, post_form_as, put_json_as, TestApp};
use serde_json::json;

async fn admin(app: &TestApp) -> String {
    let (cookie, user) = app.register("admin@example.com", "supersecret").await;
    assert_eq!(user["user"]["role"], "admin");
    cookie
}

async fn member(app: &TestApp, email: &str) -> String {
    let (cookie, _) = app.register(email, "supersecret").await;
    cookie
}

#[tokio::test]
async fn the_theme_catalogue_is_seeded_and_listed_by_the_admin_api() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    let response = app.get_as("/api/v1/admin/themes", &cookie).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    let body = response.json();
    assert_eq!(body["switching"], "all");
    let themes = body["themes"].as_array().expect("themes");
    assert_eq!(themes.len(), 6);
    let names: Vec<&str> = themes
        .iter()
        .map(|theme| theme["key"].as_str().expect("key"))
        .collect();
    assert_eq!(
        names,
        vec!["verdant", "aurora", "paper", "signal", "gilded", "swiss"]
    );
    assert_eq!(themes[0]["default"], true);
    assert_eq!(themes[0]["enabled"], true);
    assert_eq!(themes[0]["css_url"], "/assets/app.css");
    assert!(themes
        .iter()
        .all(|theme| theme["css_url"].as_str().is_some()));

    // The catalogue marking comes from one source of truth, so a row can never
    // report `auto` while a badge calls it light-only or dark-only.
    for theme in themes {
        assert_eq!(theme["dark_only"], theme["appearance"] == "dark");
        assert_eq!(theme["light_only"], theme["appearance"] == "light");
    }
    let aurora = themes
        .iter()
        .find(|theme| theme["key"] == "aurora")
        .expect("aurora");
    assert_eq!(aurora["appearance"], "dark");
    assert_eq!(aurora["dark_only"], true);
}

#[tokio::test]
async fn only_administrators_can_manage_themes() {
    let app = TestApp::spawn().await;
    admin(&app).await;
    let cookie = member(&app, "member@example.com").await;

    let response = app.get_as("/api/v1/admin/themes", &cookie).await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert_eq!(response.json()["error"]["code"], "forbidden");

    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "default": "paper" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    let response = app.get_as("/app/admin/themes", &cookie).await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_admin_can_choose_the_enabled_set_and_default_theme() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({
                "enabled": ["paper", "swiss"],
                "default": "paper",
                "switching": "admin"
            }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    let body = response.json();
    assert_eq!(body["switching"], "admin");
    let themes = body["themes"].as_array().expect("themes");
    let paper = themes
        .iter()
        .find(|theme| theme["key"] == "paper")
        .expect("paper");
    assert_eq!(paper["enabled"], true);
    assert_eq!(paper["default"], true);
    let verdant = themes
        .iter()
        .find(|theme| theme["key"] == "verdant")
        .expect("verdant");
    assert_eq!(verdant["enabled"], false);
    assert_eq!(verdant["default"], false);

    // The landing page switches to the new default immediately.
    let response = app.get("/").await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.body.contains("/assets/themes/paper"));
    assert!(response.body.contains("data-theme-key=\"paper\""));

    // The site policy is persisted and reflected in the admin page.
    let page = app.get_as("/app/admin/themes", &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Paper Ledger"));
    assert!(page.body.contains("value=\"admin\" selected"));

    // A partial update that only changes the policy must keep the catalogue
    // that the previous request committed.
    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "switching": "all" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    let body = response.json();
    assert_eq!(body["switching"], "all");
    let themes = body["themes"].as_array().expect("themes");
    let paper = themes
        .iter()
        .find(|theme| theme["key"] == "paper")
        .expect("paper");
    assert_eq!(paper["enabled"], true);
    assert_eq!(paper["default"], true);
    let verdant = themes
        .iter()
        .find(|theme| theme["key"] == "verdant")
        .expect("verdant");
    assert_eq!(verdant["enabled"], false);
}

#[tokio::test]
async fn a_member_may_switch_only_when_the_policy_allows_it() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let member_cookie = member(&app, "member@example.com").await;

    // `all` is the seeded policy, so a signed-in member can switch.
    let response = app
        .send(get_as("/app/theme/aurora?next=/app", &member_cookie))
        .await;
    assert_eq!(response.status, StatusCode::FOUND);
    assert_eq!(response.header("location").as_deref(), Some("/app"));
    let theme_cookie = response.cookie_named("urlet_theme").expect("theme cookie");
    assert!(theme_cookie.contains("urlet_theme=aurora"));

    let combined = format!("{member_cookie}; {theme_cookie}");
    let page = app.send(get_as("/app", &combined)).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("/assets/themes/aurora"));
    assert!(page.body.contains("data-theme-key=\"aurora\""));
    // Aurora is dark-only, so the light/dark toggle is not rendered.
    assert!(!page.body.contains("data-theme-toggle"));

    // Administrators switch the policy to admin-only.
    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "switching": "admin" }),
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK);

    let response = app
        .send(get_as("/app/theme/paper?next=/app", &member_cookie))
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn every_design_marks_its_own_palette_policy() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    // The five designs that ship both palettes offer the light/dark switch and
    // load the script that resolves the stored preference. None of them pins
    // the document, so the operating system still decides until the visitor
    // chooses.
    for key in ["verdant", "paper", "signal", "gilded", "swiss"] {
        let page = app
            .get_as("/app", &format!("{cookie}; urlet_theme={key}"))
            .await;
        assert_eq!(page.status, StatusCode::OK, "{key}");
        assert!(
            page.body.contains("data-theme-toggle"),
            "{key} must offer the light/dark switch"
        );
        assert!(
            page.body.contains("/assets/theme.js"),
            "{key} must load the preference script"
        );
        assert!(
            !page.body.contains("data-theme-lock"),
            "{key} must not lock"
        );
        assert!(!page.body.contains("data-theme=\"light\""), "{key}");
        assert!(!page.body.contains("data-theme=\"dark\""), "{key}");
    }

    // Aurora is dark-only: the page carries the server-side marker and offers
    // no control, because a switch could not change anything.
    let aurora = app
        .get_as("/app", &format!("{cookie}; urlet_theme=aurora"))
        .await;
    assert_eq!(aurora.status, StatusCode::OK);
    assert!(
        aurora.body.contains("data-theme=\"dark\""),
        "{}",
        aurora.body
    );
    assert!(aurora.body.contains("data-theme-lock=\"dark\""));
    assert!(!aurora.body.contains("data-theme-toggle"));
}

#[tokio::test]
async fn disabled_and_unknown_themes_are_rejected_or_fall_back() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "enabled": ["verdant"], "default": "verdant" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);

    // A stale cookie for a now-disabled theme renders the default instead.
    let page = app
        .send(get_as("/app", &format!("{cookie}; urlet_theme=aurora")))
        .await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("/assets/app.css"));
    assert!(page.body.contains("data-theme-key=\"verdant\""));

    // The picker cannot set a disabled or unknown theme.
    let response = app
        .send(get_as("/app/theme/aurora?next=/app", &cookie))
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    let response = app
        .send(get_as("/app/theme/not-a-theme?next=/app", &cookie))
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);

    // At least one theme and an enabled default are mandatory.
    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "enabled": [], "default": "verdant" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "enabled": ["paper"], "default": "aurora" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn theme_assets_are_embedded_and_unknown_keys_are_404() {
    let app = TestApp::spawn().await;
    for theme in urlet::services::theme::ALL {
        let key = theme.key();
        let response = app.get(&format!("/assets/themes/{key}")).await;
        assert_eq!(response.status, StatusCode::OK, "{key}");
        assert_eq!(
            response.header("content-type").as_deref(),
            Some("text/css; charset=utf-8")
        );
        assert!(response.body.contains("body") || response.body.contains(":root"));
    }
    let response = app.get("/assets/themes/unknown").await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    let response = app.get("/assets/theme-ui.css").await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.body.contains(".theme-switcher"));
    // The light/dark switch is shared chrome like the picker, so a design that
    // only loads its own stylesheet still gets a painted control.
    assert!(response.body.contains(".theme-toggle"));
}

#[tokio::test]
async fn every_dual_palette_design_serves_the_system_and_the_explicit_palette() {
    let app = TestApp::spawn().await;

    // Following the operating system is not enough once a design offers the
    // light/dark switch: an explicit choice has to be able to override it in
    // both directions.
    for key in ["paper", "signal", "gilded", "swiss"] {
        let response = app.get(&format!("/assets/themes/{key}")).await;
        assert_eq!(response.status, StatusCode::OK, "{key}");
        assert!(
            response.body.contains(":root:not([data-theme=\"light\"])"),
            "{key} must let an explicit light choice beat the system palette"
        );
        assert!(
            response.body.contains(":root[data-theme=\"dark\"]"),
            "{key} must define the explicit dark palette"
        );
    }

    // The dark-only design has no second palette to serve.
    let aurora = app.get("/assets/themes/aurora").await;
    assert!(!aurora.body.contains("prefers-color-scheme"));
    assert!(!aurora.body.contains("data-theme=\"dark\""));
}

#[tokio::test]
async fn a_design_that_paints_the_browser_chrome_follows_the_resolved_mode() {
    let app = TestApp::spawn().await;

    // `theme-color` can only be selected by a media query, never by
    // `data-theme`, so Signal marks its two tags with the mode they belong to
    // and the resolved mode turns the matching one on.
    let landing = app.get_as("/", "urlet_theme=signal").await;
    assert_eq!(landing.status, StatusCode::OK);
    for mode in ["light", "dark"] {
        // The marker has to agree with the media query it sits next to, or the
        // no-JavaScript path and the script would disagree about the mode.
        let tag = format!("media=\"(prefers-color-scheme: {mode})\" data-theme-mode=\"{mode}\"");
        assert!(landing.body.contains(&tag), "missing {tag}");
    }

    // The shared script is what flips them, so it must know the marker.
    let script = app.get("/assets/theme.js").await;
    assert_eq!(script.status, StatusCode::OK);
    assert!(
        script
            .body
            .contains("meta[name=\"theme-color\"][data-theme-mode]"),
        "theme.js must select the marked theme-color tags"
    );
}

#[tokio::test]
async fn embedded_asset_urls_are_versioned() {
    let app = TestApp::spawn().await;
    let version = urlet::VERSION;

    // The default theme carries the build version on every CSS/JS URL, so a
    // browser cannot keep a previous release's stylesheet for an hour.
    let page = app.get("/").await;
    for asset in [
        "/assets/theme.js",
        "/assets/app.css",
        "/assets/app.js",
        "/assets/theme-ui.css",
    ] {
        let expected = format!("{asset}?v={version}");
        assert!(page.body.contains(&expected), "missing {expected}");
    }

    // Non-default themes version their own stylesheet URL too.
    let cookie = admin(&app).await;
    let themed = app
        .get_as("/app", &format!("{cookie}; urlet_theme=aurora"))
        .await;
    let expected = format!("/assets/themes/aurora?v={version}");
    assert!(themed.body.contains(&expected), "missing {expected}");
}

#[tokio::test]
async fn the_theme_picker_menu_paints_its_own_text_and_background() {
    let app = TestApp::spawn().await;
    let response = app.get("/assets/theme-ui.css").await;
    assert_eq!(response.status, StatusCode::OK);
    let rule = response
        .body
        .split(".theme-menu {")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .expect("the shared picker CSS must define .theme-menu");
    // The menu floats over content and must not inherit the colour of the bar
    // that opened it: on inverted bars (Swiss, Signal, Gilded) that colour is
    // the same as the menu surface, so both sides of the pair are explicit.
    assert!(rule.contains("background:"), "{rule}");
    assert!(rule.contains("color:"), "{rule}");
    assert!(!rule.contains("color: inherit"), "{rule}");

    // The picker also sits in the page footer, where a downward menu would open
    // past the end of the document.
    assert!(
        response.body.contains("footer.site .theme-menu"),
        "the footer menu must be told to open upward"
    );
    assert!(
        response.body.contains("footer.colophon .theme-menu"),
        "Paper calls its footer `colophon`"
    );
}

#[tokio::test]
async fn the_theme_picker_is_hidden_without_permission() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let member_cookie = member(&app, "member@example.com").await;

    // The admin sees the picker and can switch.
    let page = app.get_as("/app", &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("data-theme-switcher"));
    assert!(page.body.contains("href=\"/app/theme/paper\""));
    assert_eq!(
        page.body.matches("aria-current=\"true\"").count(),
        1,
        "the picker must mark exactly one current entry"
    );

    // The signed-in landing page carries the same picker.
    let landing = app.get_as("/", &cookie).await;
    assert!(landing.body.contains("data-theme-switcher"));

    // A member sees it too while the policy is `all`.
    let page = app.get_as("/app", &member_cookie).await;
    assert!(page.body.contains("data-theme-switcher"));

    // Turn it off: the member's picker disappears, and an existing cookie is
    // ignored at render time rather than only being hidden in the UI.
    let member_theme = format!("{member_cookie}; urlet_theme=aurora");
    let before = app.get_as("/app", &member_theme).await;
    assert!(before.body.contains("/assets/themes/aurora"));
    app.send(put_json_as(
        "/api/v1/admin/themes",
        json!({ "switching": "admin" }),
        &cookie,
    ))
    .await;
    let page = app.get_as("/app", &member_theme).await;
    assert!(!page.body.contains("data-theme-switcher"));
    assert!(page.body.contains("/assets/app.css"));
    assert!(page.body.contains("data-theme-key=\"verdant\""));
    let landing = app.get_as("/", &member_theme).await;
    assert!(!landing.body.contains("data-theme-switcher"));
    assert!(landing.body.contains("/assets/app.css"));
    // Anonymous cookies are suppressed too under `admin`.
    let anonymous = app.get_as("/", "urlet_theme=paper").await;
    assert!(anonymous.body.contains("/assets/app.css"));
    let page = app.get_as("/app", &cookie).await;
    assert!(page.body.contains("data-theme-switcher"));
}

#[tokio::test]
async fn a_visitor_can_pick_a_theme_while_the_policy_is_all() {
    let app = TestApp::spawn().await;

    // The landing page offers the picker to a visitor without a session, and the
    // sign-in page repeats it in the shared footer.
    let landing = app.get("/").await;
    assert_eq!(landing.status, StatusCode::OK);
    // The shared chrome imports the picker macro; the document must still start
    // with the doctype rather than a stray newline.
    assert!(landing.body.starts_with("<!DOCTYPE html>"));
    assert!(landing.body.contains("data-theme-switcher"));
    assert!(landing.body.contains("href=\"/app/theme/paper\""));
    assert!(landing.body.contains("/assets/app.css"));
    let login = app.get("/app/login").await;
    assert!(login.body.contains("data-theme-switcher"));

    // Picking a theme writes the cookie and returns to the landing page. With no
    // `next` and no `Referer`, `/app` would only bounce the visitor to the
    // sign-in screen, so the visitor fallback is `/`.
    let switched = app.get("/app/theme/paper").await;
    assert_eq!(switched.status, StatusCode::FOUND);
    assert_eq!(switched.header("location").as_deref(), Some("/"));
    let theme_cookie = switched.cookie_named("urlet_theme").expect("theme cookie");
    assert!(theme_cookie.contains("urlet_theme=paper"));

    // The next request renders the visitor's choice, not the site default.
    let themed = app.get_as("/", &theme_cookie).await;
    assert!(themed.body.contains("/assets/themes/paper"));
    assert!(themed.body.contains("data-theme-key=\"paper\""));
    assert!(themed.body.contains("data-theme-switcher"));

    // The site-default entry clears the explicit choice again.
    let cleared = app.get_as("/app/theme/default", &theme_cookie).await;
    assert_eq!(cleared.status, StatusCode::FOUND);
    let cleared_cookie = cleared.cookie_named("urlet_theme").expect("clear cookie");
    assert_eq!(cleared_cookie, "urlet_theme=");

    // The redirect surface stays minimal: a link page never offers the picker,
    // even to a visitor who may switch on the landing page.
    let missing = app.get("/definitely-not-a-slug").await;
    assert!(!missing.body.contains("data-theme-switcher"));
}

#[tokio::test]
async fn a_slug_beginning_with_app_stays_on_the_redirect_surface() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    // `app` is reserved, but a slug that merely begins with it is an ordinary
    // link. It must not be mistaken for the `/app` shell: that would give a
    // visitor the picker on a link page under `all`, and cost the redirect hot
    // path the session lookup it is meant to avoid.
    let created = app
        .post_json_as(
            "/api/v1/urls",
            json!({
                "target_url": "https://example.com/apple",
                "slug": "apple",
                "password": "secret123"
            }),
            &cookie,
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);

    // The cookie is still honoured; only the picker is withheld.
    let page = app.get_as("/apple", "urlet_theme=paper").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.body.contains("/assets/themes/paper"));
    assert!(!page.body.contains("data-theme-switcher"));

    // The same visitor is offered the picker on the landing page.
    let landing = app.get_as("/", "urlet_theme=paper").await;
    assert!(landing.body.contains("data-theme-switcher"));
}

#[tokio::test]
async fn every_theme_offers_the_picker_to_a_visitor() {
    let app = TestApp::spawn().await;

    // Every design must behave the same for a visitor without a session: the
    // landing page and the sign-in page carry the shared tools, the redirect
    // surface stays minimal, and the picker is never rendered twice.
    for theme in urlet::services::theme::ALL {
        let key = theme.key();
        let cookie = format!("urlet_theme={key}");
        // Aurora ships one palette, so it must not offer a switch; every other
        // design offers exactly one, and never a second copy in the footer.
        let toggles = if theme.is_dark_only() { 0 } else { 1 };

        let landing = app.get_as("/", &cookie).await;
        assert_eq!(landing.status, StatusCode::OK, "{key} landing");
        assert_eq!(
            landing.body.matches("data-theme-switcher").count(),
            1,
            "{key} must offer exactly one picker on the landing page"
        );
        assert_eq!(
            landing.body.matches("data-theme-toggle").count(),
            toggles,
            "{key} light/dark switch count on the landing page"
        );

        let login = app.get_as("/app/login", &cookie).await;
        assert_eq!(login.status, StatusCode::OK, "{key} sign-in");
        assert_eq!(
            login.body.matches("data-theme-switcher").count(),
            1,
            "{key} must offer exactly one picker on the sign-in page"
        );
        assert_eq!(
            login.body.matches("data-theme-toggle").count(),
            toggles,
            "{key} light/dark switch count on the sign-in page"
        );
        assert!(
            login.body.contains("/assets/theme-ui.css"),
            "{key} must load the picker stylesheet where it renders the picker"
        );

        let register = app.get_as("/app/register", &cookie).await;
        assert_eq!(register.status, StatusCode::OK, "{key} registration");
        assert_eq!(
            register.body.matches("data-theme-switcher").count(),
            1,
            "{key} must offer exactly one picker on the registration page"
        );
        assert_eq!(
            register.body.matches("data-theme-toggle").count(),
            toggles,
            "{key} light/dark switch count on the registration page"
        );

        let link = app.get_as("/definitely-not-a-slug", &cookie).await;
        assert_eq!(link.status, StatusCode::NOT_FOUND, "{key} link page");
        assert!(
            !link.body.contains("data-theme-switcher"),
            "{key} must keep the link page free of the picker"
        );
        // The switch is not part of the theme catalogue policy: it is a
        // cosmetic preference, so the minimal link page keeps it in the footer.
        assert_eq!(
            link.body.matches("data-theme-toggle").count(),
            toggles,
            "{key} light/dark switch count on the link page"
        );
    }
}

#[tokio::test]
async fn a_visitor_keeps_the_default_while_the_policy_is_admin() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    app.send(put_json_as(
        "/api/v1/admin/themes",
        json!({ "switching": "admin" }),
        &cookie,
    ))
    .await;

    // No picker, and a cookie from a previous `all` policy is ignored at render
    // time rather than only hidden in the UI.
    let landing = app.get_as("/", "urlet_theme=paper").await;
    assert_eq!(landing.status, StatusCode::OK);
    assert!(!landing.body.contains("data-theme-switcher"));
    assert!(landing.body.contains("/assets/app.css"));
    let login = app.get_as("/app/login", "urlet_theme=paper").await;
    assert!(!login.body.contains("data-theme-switcher"));

    // A direct switch is refused instead of being silently accepted: the visitor
    // is sent to sign in with a way back to the page they were reading.
    let refused = app.get_as("/app/theme/paper", "urlet_theme=paper").await;
    assert_eq!(refused.status, StatusCode::SEE_OTHER);
    let location = refused.header("location").expect("location");
    assert!(location.starts_with("/app/login?next="), "{location}");
    assert!(refused.cookie_named("urlet_theme").is_none());

    // Administrators keep the picker and can still switch.
    let admin_page = app.get_as("/app", &cookie).await;
    assert!(admin_page.body.contains("data-theme-switcher"));
    let admin_switch = app.get_as("/app/theme/paper", &cookie).await;
    assert_eq!(admin_switch.status, StatusCode::FOUND);
    assert!(admin_switch
        .cookie_named("urlet_theme")
        .expect("theme cookie")
        .contains("urlet_theme=paper"));
}

#[tokio::test]
async fn a_visitor_cannot_reach_the_administrator_preview() {
    let app = TestApp::spawn().await;

    // Preview may name a disabled theme, so it stays administrator-only even
    // though `all` lets a visitor switch. A refused request must leave neither
    // cookie behind.
    let refused = app.get("/app/theme/aurora?preview=1").await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert!(refused.cookie_named("urlet_theme_preview").is_none());
    assert!(refused.cookie_named("urlet_theme").is_none());

    // Under `admin` the plain switch and the site-default reset are closed too.
    let cookie = admin(&app).await;
    app.send(put_json_as(
        "/api/v1/admin/themes",
        json!({ "switching": "admin" }),
        &cookie,
    ))
    .await;

    for uri in ["/app/theme/paper", "/app/theme/default"] {
        let response = app.get(uri).await;
        assert_eq!(response.status, StatusCode::SEE_OTHER, "{uri}");
        let location = response.header("location").expect("location");
        assert!(
            location.starts_with("/app/login?next="),
            "{uri}: {location}"
        );
        assert!(response.cookie_named("urlet_theme").is_none(), "{uri}");
    }
}

#[tokio::test]
async fn a_visitor_switch_never_leaves_the_site() {
    let app = TestApp::spawn().await;

    // The anonymous path is new, so re-check that a hostile `next` cannot turn
    // the switch into an open redirect: each rejected value falls back to `/`.
    for next in [
        "https%3A%2F%2Fevil.example%2Fx",
        "%2F%2Fevil.example",
        "%2F%5Cevil.example",
        "%0A%2F%2Fevil.example",
    ] {
        let response = app.get(&format!("/app/theme/paper?next={next}")).await;
        assert_eq!(response.status, StatusCode::FOUND, "next={next}");
        assert_eq!(
            response.header("location").as_deref(),
            Some("/"),
            "next={next} must not become an open redirect"
        );
    }

    // A same-site `next` is still honoured.
    let kept = app.get("/app/theme/paper?next=%2Fapp%2Faccount").await;
    assert_eq!(kept.header("location").as_deref(), Some("/app/account"));

    // An off-site referer is ignored the same way.
    let request = Request::builder()
        .method("GET")
        .uri("/app/theme/paper")
        .header(header::HOST, "localhost:8080")
        .header(header::REFERER, "https://evil.example/app")
        .body(Body::empty())
        .expect("request");
    let response = app.send(request).await;
    assert_eq!(response.status, StatusCode::FOUND);
    assert_eq!(response.header("location").as_deref(), Some("/"));
}

#[tokio::test]
async fn the_theme_form_updates_the_catalogue() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    let body = "theme_switching=admin\
                &enabled_verdant=on\
                &enabled_gilded=on\
                &default_theme=gilded";
    let response = app
        .send(post_form_as("/app/admin/themes", body, &cookie))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    assert_eq!(
        response.header("location").as_deref(),
        Some("/app/admin/themes?ok=saved")
    );

    let page = app.get_as("/app/admin/themes", &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("/assets/themes/gilded"));
    assert!(page.body.contains("selected"));

    // A form with everything unchecked is rejected without changing anything.
    let response = app
        .send(post_form_as(
            "/app/admin/themes",
            "theme_switching=all&default_theme=verdant",
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert!(response
        .header("location")
        .as_deref()
        .unwrap_or_default()
        .contains("err=theme"));

    let api = app.get_as("/api/v1/admin/themes", &cookie).await;
    let body = api.json();
    assert_eq!(body["switching"], "admin");
    let gilded = body["themes"]
        .as_array()
        .expect("themes")
        .iter()
        .find(|theme| theme["key"] == "gilded")
        .expect("gilded");
    assert_eq!(gilded["default"], true);
}

#[tokio::test]
async fn every_theme_renders_every_page() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    // One ordinary link for the detail page and one password-protected link
    // for the public password page.
    let created = app
        .post_json_as(
            "/api/v1/urls",
            json!({ "target_url": "https://example.com/ordinary" }),
            &cookie,
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let ordinary_id = created.json()["id"].as_i64().expect("id");

    let protected = app
        .post_json_as(
            "/api/v1/urls",
            json!({
                "target_url": "https://example.com/protected",
                "password": "secret123"
            }),
            &cookie,
        )
        .await;
    assert_eq!(protected.status, StatusCode::CREATED, "{}", protected.body);
    let protected_slug = protected.json()["slug"].as_str().expect("slug").to_string();

    let groups = app.get_as("/api/v1/admin/groups", &cookie).await.json();
    let default_group_id = groups["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|group| group["is_default"] == true)
        .expect("default group")["id"]
        .as_i64()
        .expect("id");

    let theme_group = app
        .post_json_as(
            "/api/v1/admin/groups",
            json!({ "name": "Theme Edit Group" }),
            &cookie,
        )
        .await;
    assert_eq!(
        theme_group.status,
        StatusCode::CREATED,
        "{}",
        theme_group.body
    );
    let theme_group_id = theme_group.json()["id"].as_i64().expect("id");

    let pages = [
        "/",
        "/app",
        "/app/account",
        "/app/invites",
        "/app/admin",
        "/app/admin/users",
        "/app/admin/groups",
        "/app/admin/groups/new",
        "/app/admin/groups/{group}/edit",
        "/app/admin/groups/{other}/edit",
        "/app/admin/urls",
        "/app/admin/invites",
        "/app/admin/domains",
        "/app/admin/themes",
        "/app/admin/themes/new",
        "/app/admin/settings",
        "/app/admin/audit",
        "/app/urls/{id}",
    ];

    for theme in urlet::services::theme::ALL {
        let key = theme.key();
        let css = if key == "verdant" {
            "/assets/app.css".to_string()
        } else {
            format!("/assets/themes/{key}")
        };
        let cookie_with_theme = format!("{cookie}; urlet_theme={key}");
        for template in pages {
            let uri = template
                .replace("{id}", &ordinary_id.to_string())
                .replace("{group}", &default_group_id.to_string())
                .replace("{other}", &theme_group_id.to_string());
            let response = app.send(get_as(&uri, &cookie_with_theme)).await;
            assert_eq!(
                response.status,
                StatusCode::OK,
                "{key} {uri} failed: {}",
                response.body
            );
            assert!(
                response.body.contains(&css),
                "{key} {uri} did not load {css}"
            );
            assert!(
                response.body.contains(&format!("data-theme-key=\"{key}\"")),
                "{key} {uri} did not mark the theme"
            );
            if uri == "/app" {
                assert!(
                    response.body.contains("Your links"),
                    "{key} dashboard content is empty"
                );
            }
            if template == "/app/admin/groups/{other}/edit" {
                assert!(
                    response.body.contains("name=\"reassign_to\""),
                    "{key} did not render the delete form"
                );
            }
        }

        // The public password page and the generic 404 page also honour the
        // cookie, because both are rendered outside the signed-in shell.
        let password = app
            .send(get_as(&format!("/{protected_slug}"), &cookie_with_theme))
            .await;
        assert_eq!(password.status, StatusCode::OK, "{key} password page");
        assert!(password.body.contains(&css));
        assert!(password.body.contains(&format!("data-theme-key=\"{key}\"")));

        let missing = app
            .send(get_as("/definitely-not-a-slug", &cookie_with_theme))
            .await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND, "{key} 404");
        assert!(missing.body.contains(&css));
    }
}

#[tokio::test]
async fn the_admin_tab_marker_follows_the_active_section() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    // Verdant and Signal print the "current / total" marker in their admin
    // rail, so check every destination plus the two form pages that reuse the
    // groups and themes sections.
    let sections = [
        ("/app/admin", "01"),
        ("/app/admin/users", "02"),
        ("/app/admin/groups", "03"),
        ("/app/admin/urls", "04"),
        ("/app/admin/invites", "05"),
        ("/app/admin/domains", "06"),
        ("/app/admin/themes", "07"),
        ("/app/admin/settings", "08"),
        ("/app/admin/audit", "09"),
        ("/app/admin/groups/new", "03"),
        ("/app/admin/themes/new", "07"),
    ];

    for (uri, number) in sections {
        let verdant = app.get_as(uri, &cookie).await;
        assert_eq!(verdant.status, StatusCode::OK, "{uri}");
        assert!(
            verdant.body.contains(&format!("SYS · {number} / 09")),
            "{uri} must show tab {number} / 09: {}",
            verdant.body
        );
        assert_eq!(
            verdant.body.matches("SYS · ").count(),
            1,
            "{uri} must print exactly one Verdant tab marker"
        );

        let signal = app
            .get_as(uri, &format!("{cookie}; urlet_theme=signal"))
            .await;
        assert_eq!(signal.status, StatusCode::OK, "{uri}");
        assert!(
            signal.body.contains(&format!(
                "<span class=\"console-code\">{number} / 09</span>"
            )),
            "{uri} must show tab {number} / 09 in the Signal rail"
        );
        assert_eq!(
            signal.body.matches("<span class=\"console-code\">").count(),
            1,
            "{uri} must print exactly one Signal tab marker"
        );
    }

    // Swiss prints a number on every link, not just the active one.
    let swiss = app
        .get_as("/app/admin", &format!("{cookie}; urlet_theme=swiss"))
        .await;
    for number in ["01", "02", "03", "04", "05", "06", "07", "08", "09"] {
        assert!(
            swiss.body.contains(&format!(
                "<span class=\"tab-n\" aria-hidden=\"true\">{number}</span>"
            )),
            "the Swiss rail is missing tab {number}"
        );
    }

    // The three pages with their own header also print the page number there;
    // those copies used to keep the old six-tab positions.
    for (uri, number) in [
        ("/app/admin", "01"),
        ("/app/admin/urls", "04"),
        ("/app/admin/audit", "09"),
    ] {
        let signal = app
            .get_as(uri, &format!("{cookie}; urlet_theme=signal"))
            .await;
        assert!(
            signal
                .body
                .contains(&format!("<span class=\"k-index\">{number}</span>")),
            "{uri} has a stale Signal page header"
        );
        assert_eq!(
            signal.body.matches("class=\"k-index\"").count(),
            1,
            "{uri} must print exactly one Signal page header"
        );

        let gilded = app
            .get_as(uri, &format!("{cookie}; urlet_theme=gilded"))
            .await;
        assert!(
            gilded.body.contains(&format!("№ {number} · MMXXVI")),
            "{uri} has a stale Gilded page header"
        );
        assert_eq!(
            gilded.body.matches("№ ").count(),
            1,
            "{uri} must print exactly one Gilded page number"
        );

        let paper = app
            .get_as(uri, &format!("{cookie}; urlet_theme=paper"))
            .await;
        assert!(
            paper
                .body
                .contains(&format!("<span aria-hidden=\"true\">№ {number}</span>")),
            "{uri} has a stale Paper page header"
        );
        assert_eq!(
            paper.body.matches("№ ").count(),
            1,
            "{uri} must print exactly one Paper page number"
        );

        let swiss = app
            .get_as(uri, &format!("{cookie}; urlet_theme=swiss"))
            .await;
        assert!(
            swiss.body.contains(&format!(
                "<span class=\"sec-num\" aria-hidden=\"true\">{number}</span>"
            )),
            "{uri} has a stale Swiss page header"
        );
        assert!(
            swiss.body.contains("/// — 09"),
            "{uri} must show the current nine-section total"
        );
        assert_eq!(
            swiss.body.matches("class=\"sec-num\"").count(),
            1,
            "{uri} must print exactly one Swiss page number"
        );
    }

    // Aurora has no page number in its admin header by design; the existing
    // `every_theme_renders_every_page` test still renders it for every URI.
}

#[tokio::test]
async fn every_theme_renders_the_public_pages() {
    let app = TestApp::spawn().await;
    for theme in urlet::services::theme::ALL {
        let key = theme.key();
        let css = if key == "verdant" {
            "/assets/app.css".to_string()
        } else {
            format!("/assets/themes/{key}")
        };
        let cookie = format!("urlet_theme={key}");
        for uri in ["/", "/app/login", "/app/register"] {
            let response = app.send(get_as(uri, &cookie)).await;
            assert_eq!(
                response.status,
                StatusCode::OK,
                "{key} {uri} failed: {}",
                response.body
            );
            assert!(response.body.contains(&css), "{key} {uri} missing {css}");
            assert!(response.body.contains(&format!("data-theme-key=\"{key}\"")));
            // Every design links the upstream repository from its footer, and
            // the mark is inline: the CSP allows no remote image or icon font.
            assert!(
                response
                    .body
                    .contains(&format!("href=\"{}\"", urlet::REPOSITORY)),
                "{key} {uri} is missing the repository link"
            );
            assert!(
                response.body.contains("class=\"footer-repo-icon\""),
                "{key} {uri} is missing the inline repository mark"
            );
            // The embedded API reference is linked from the same footer, so a
            // visitor can reach it from whatever design the instance runs.
            assert!(
                response.body.contains("class=\"footer-link footer-api\"")
                    && response.body.contains("href=\"/docs\""),
                "{key} {uri} is missing the API reference link"
            );
        }
        // The public shell still carries exactly one language switcher and the
        // sign-in form even when a non-default theme is active.
        let login = app.send(get_as("/app/login", &cookie)).await;
        assert_eq!(login.body.matches("data-lang-switcher").count(), 1);
        assert!(login.body.contains("name=\"password\""));
    }
}

#[tokio::test]
async fn every_theme_renders_the_extra_domain_landing() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    app.add_domain("extra.example.com", "active").await;

    for theme in urlet::services::theme::ALL {
        let key = theme.key();
        let css = if key == "verdant" {
            "/assets/app.css".to_string()
        } else {
            format!("/assets/themes/{key}")
        };
        let cookie_with_theme = format!("{cookie}; urlet_theme={key}");
        let response = app
            .send(common::get_with_host_as(
                "/",
                "extra.example.com",
                &cookie_with_theme,
            ))
            .await;
        assert_eq!(
            response.status,
            StatusCode::OK,
            "{key} short landing: {}",
            response.body
        );
        assert!(response.body.contains(&css), "{key} missing {css}");
        assert!(response.body.contains(&format!("data-theme-key=\"{key}\"")));
        assert!(response.body.contains("noindex"));
        // The short landing keeps the shared footer, so the repository link is
        // there even though the page drops the sign-in shell.
        assert!(
            response
                .body
                .contains(&format!("href=\"{}\"", urlet::REPOSITORY)),
            "{key} short landing is missing the repository link"
        );
        // The redirect-only landing page is a public page too, so a visitor can
        // switch from it under the default `all` policy.
        assert_eq!(
            response.body.matches("data-theme-switcher").count(),
            1,
            "{key} must offer exactly one picker on the extra-domain landing"
        );
    }
}

#[tokio::test]
async fn catalogue_and_policy_roll_back_together() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    // Remove the destination table of the second write. The theme transaction
    // must fail as a whole: the enabled/default rows written before the settings
    // insert have to roll back, not be left behind under a 500 response.
    sqlx::query("DROP TABLE settings")
        .execute(&app.state.db)
        .await
        .expect("drop settings");

    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({
                "enabled": ["paper"],
                "default": "paper",
                "switching": "admin"
            }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);

    let enabled: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM themes WHERE is_enabled = 1")
        .fetch_one(&app.state.db)
        .await
        .expect("enabled count");
    let default: String = sqlx::query_scalar("SELECT key FROM themes WHERE is_default = 1")
        .fetch_one(&app.state.db)
        .await
        .expect("default key");
    assert_eq!(enabled, 6, "the enabled set must roll back with the policy");
    assert_eq!(
        default, "verdant",
        "the default must roll back with the policy"
    );
}

#[tokio::test]
async fn administrators_can_preview_a_disabled_theme_without_enabling_it() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let member_cookie = member(&app, "member@example.com").await;

    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "enabled": ["verdant"], "default": "verdant" }),
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);

    // A member cannot use the preview cookie.
    let response = app
        .send(get_as(
            "/app/theme/aurora?preview=1&next=/app",
            &member_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    let response = app
        .send(get_as(
            "/app/theme/aurora?preview=1&next=/app",
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::FOUND);
    let preview = response
        .cookie_named("urlet_theme_preview")
        .expect("preview cookie");
    let raw = response
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("urlet_theme_preview="))
        .expect("raw preview cookie");
    assert!(raw.contains("HttpOnly"));
    assert!(raw.contains("SameSite=Lax"));
    assert!(raw.contains("Path=/"));
    assert!(raw.contains("Max-Age=3600"));
    let combined = format!("{admin_cookie}; {preview}");
    let page = app.get_as("/app", &combined).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("/assets/themes/aurora"));
    assert!(page.body.contains("data-theme-key=\"aurora\""));

    let admin_page = app.get_as("/app/admin/themes", &combined).await;
    assert!(admin_page.body.contains("Stop preview"));
    assert!(admin_page.body.contains("preview=1"));

    let stop = app
        .send(get_as("/app/theme/preview/stop?next=/app", &combined))
        .await;
    assert_eq!(stop.status, StatusCode::FOUND);
    let cleared = stop
        .cookie_named("urlet_theme_preview")
        .expect("clear cookie");
    assert!(cleared.starts_with("urlet_theme_preview="));
    let page = app
        .get_as("/app", &format!("{admin_cookie}; {cleared}"))
        .await;
    assert!(page.body.contains("/assets/app.css"));
}

#[tokio::test]
async fn the_site_default_clears_the_explicit_choice() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let switched = app
        .send(get_as("/app/theme/paper?next=/app", &cookie))
        .await;
    let theme_cookie = switched.cookie_named("urlet_theme").expect("theme");
    let combined = format!("{cookie}; {theme_cookie}");
    assert!(app
        .get_as("/app", &combined)
        .await
        .body
        .contains("/assets/themes/paper"));

    let response = app
        .send(get_as("/app/theme/default?next=/app", &combined))
        .await;
    assert_eq!(response.status, StatusCode::FOUND);
    let cleared = response.cookie_named("urlet_theme").expect("clear cookie");
    assert!(cleared.starts_with("urlet_theme="));
    let page = app.get_as("/app", &format!("{cookie}; {cleared}")).await;
    assert!(page.body.contains("/assets/app.css"));
}

#[tokio::test]
async fn logging_out_clears_the_theme_cookies() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let response = app.send(post_form_as("/app/logout", "", &cookie)).await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    let cookies: Vec<String> = response
        .headers
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_string))
        .collect();
    assert!(cookies
        .iter()
        .any(|value| value.starts_with("urlet_session=;")));
    assert!(cookies
        .iter()
        .any(|value| value.starts_with("urlet_theme=;")));
    assert!(cookies
        .iter()
        .any(|value| value.starts_with("urlet_theme_preview=;")));
}

#[tokio::test]
async fn the_theme_admin_form_is_admin_and_csrf_protected() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let member_cookie = member(&app, "member@example.com").await;

    // A member cannot reach or POST the page even with a same-origin form.
    let response = app
        .send(post_form_as(
            "/app/admin/themes",
            "theme_switching=all&default_theme=verdant&enabled_verdant=on",
            &member_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    // A cookie-authenticated POST without an Origin/Referer is rejected by the
    // shared CSRF middleware before the handler runs.
    let response = app
        .send(common::post_form(
            "/app/admin/themes",
            "theme_switching=all&default_theme=verdant&enabled_verdant=on",
            Some(&admin_cookie),
            None,
        ))
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_theme_policy_can_be_changed_through_the_settings_api() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let member_cookie = member(&app, "member@example.com").await;

    let response = app
        .send(put_json_as(
            "/api/v1/admin/settings",
            json!({ "theme_switching": "admin" }),
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);

    let catalogue = app.get_as("/api/v1/admin/themes", &admin_cookie).await;
    assert_eq!(catalogue.json()["switching"], "admin");
    let page = app.get_as("/app", &member_cookie).await;
    assert!(!page.body.contains("data-theme-switcher"));
}

#[tokio::test]
async fn theme_updates_are_audited() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "enabled": ["verdant"], "default": "verdant" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);

    let audit = app.get_as("/api/v1/admin/audit", &cookie).await;
    assert_eq!(audit.status, StatusCode::OK);
    assert!(
        audit.body.contains("admin.themes_updated"),
        "theme update missing from audit: {}",
        audit.body
    );
}

#[tokio::test]
async fn ported_themes_keep_the_optional_invite_field() {
    let app = TestApp::spawn().await;
    for key in ["aurora", "paper", "signal", "gilded", "swiss"] {
        let page = app
            .get_as("/app/register", &format!("urlet_theme={key}"))
            .await;
        assert_eq!(page.status, StatusCode::OK, "{key}");
        assert!(
            page.body.contains("name=\"invite_code\""),
            "{key} register form lost the invite field"
        );
        assert!(
            page.body
                .contains("Optional: if you were given a group invite code"),
            "{key} register form lost the optional invite hint"
        );
    }
}

#[tokio::test]
async fn a_stale_enabled_row_does_not_break_a_partial_update() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let now = urlet::services::time::now_iso();
    sqlx::query(
        "INSERT INTO themes (key, is_enabled, is_default, sort_order, created_at, updated_at)
         VALUES ('ghost', 1, 0, 999, ?, ?)",
    )
    .bind(&now)
    .bind(&now)
    .execute(&app.state.db)
    .await
    .expect("insert ghost");
    app.state
        .themes
        .reload(&app.state.db)
        .await
        .expect("reload themes");

    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "switching": "admin" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    let body = response.json();
    assert_eq!(body["switching"], "admin");
    assert!(body["themes"]
        .as_array()
        .expect("themes")
        .iter()
        .all(|theme| theme["key"] != "ghost"));
    let enabled: i64 = sqlx::query_scalar("SELECT is_enabled FROM themes WHERE key = 'ghost'")
        .fetch_one(&app.state.db)
        .await
        .expect("ghost row");
    assert_eq!(enabled, 0, "the unknown row must be disabled by the update");
}

#[tokio::test]
async fn the_theme_cookie_is_secure_when_configured() {
    let app = TestApp::spawn_with(|config| config.secure_cookies = true).await;
    let (cookie, _) = app.register("admin@example.com", "supersecret").await;
    let response = app
        .send(get_as("/app/theme/paper?next=/app", &cookie))
        .await;
    assert_eq!(response.status, StatusCode::FOUND);
    let raw = response
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("urlet_theme="))
        .expect("theme cookie");
    assert!(raw.contains("Secure"));
    assert!(raw.contains("HttpOnly"));
    assert!(raw.contains("SameSite=Lax"));
}

#[tokio::test]
async fn the_referer_keeps_theme_switch_on_the_same_site() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("admin@example.com", "supersecret").await;
    let request = Request::builder()
        .method("GET")
        .uri("/app/theme/paper")
        .header(header::HOST, "localhost:8080")
        .header(header::COOKIE, &cookie)
        .header(header::REFERER, "http://localhost:8080/app/account")
        .body(Body::empty())
        .expect("request");
    let response = app.send(request).await;
    assert_eq!(response.status, StatusCode::FOUND);
    assert_eq!(response.header("location").as_deref(), Some("/app/account"));

    // An external referer is ignored rather than becoming an open redirect.
    let request = Request::builder()
        .method("GET")
        .uri("/app/theme/paper")
        .header(header::HOST, "localhost:8080")
        .header(header::COOKIE, &cookie)
        .header(header::REFERER, "https://evil.example/app")
        .body(Body::empty())
        .expect("request");
    let response = app.send(request).await;
    assert_eq!(response.status, StatusCode::FOUND);
    assert_eq!(response.header("location").as_deref(), Some("/app"));
}

#[tokio::test]
async fn concurrent_partial_updates_do_not_lose_the_catalogue() {
    let app = TestApp::spawn().await;
    let enabled = vec!["paper".to_string()];

    let catalogue = urlet::services::theme::update_selection(
        &app.state.db,
        &app.state.themes,
        &app.state.settings,
        Some(&enabled),
        Some("paper"),
        None,
        None,
    );
    let policy = urlet::services::theme::update_selection(
        &app.state.db,
        &app.state.themes,
        &app.state.settings,
        None,
        None,
        Some(urlet::services::settings::ThemeSwitching::AdminOnly),
        None,
    );
    let (catalogue, policy) = tokio::join!(catalogue, policy);
    catalogue.expect("catalogue update");
    policy.expect("policy update");

    let paper: (i64, i64) =
        sqlx::query_as("SELECT is_enabled, is_default FROM themes WHERE key = 'paper'")
            .fetch_one(&app.state.db)
            .await
            .expect("paper row");
    let switching: String =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'theme_switching'")
            .fetch_one(&app.state.db)
            .await
            .expect("policy");
    assert_eq!(
        paper,
        (1, 1),
        "the policy update must keep paper enabled/default"
    );
    assert_eq!(switching, "admin");
    assert_eq!(
        app.state.settings.theme_switching(),
        urlet::services::settings::ThemeSwitching::AdminOnly
    );
    assert!(app.state.themes.is_enabled("paper"));
}

const MULTIPART_BOUNDARY: &str = "----urlet-theme-test-boundary";

/// Build a browser-style multipart upload with the same-origin header the CSRF
/// layer requires.
fn theme_multipart(
    uri: &str,
    cookie: &str,
    fields: &[(&str, &str)],
    file: Option<(&str, &str, &[u8])>,
) -> Request<Body> {
    let mut body: Vec<u8> = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(
            format!(
                "--{MULTIPART_BOUNDARY}\r\n\
                 Content-Disposition: form-data; name=\"{name}\"\r\n\r\n\
                 {value}\r\n"
            )
            .as_bytes(),
        );
    }
    if let Some((field, filename, bytes)) = file {
        body.extend_from_slice(
            format!(
                "--{MULTIPART_BOUNDARY}\r\n\
                 Content-Disposition: form-data; name=\"{field}\"; filename=\"{filename}\"\r\n\
                 Content-Type: text/css\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{MULTIPART_BOUNDARY}--\r\n").as_bytes());

    Request::builder()
        .method("POST")
        .uri(uri)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={MULTIPART_BOUNDARY}"),
        )
        .header(header::COOKIE, cookie)
        .header(header::ORIGIN, "http://localhost:8080")
        .body(Body::from(body))
        .expect("multipart request")
}

fn upload_request(cookie: &str, name: &str, description: &str, css: Option<&str>) -> Request<Body> {
    upload_request_with_appearance(cookie, name, description, None, css)
}

fn upload_request_with_appearance(
    cookie: &str,
    name: &str,
    description: &str,
    appearance: Option<&str>,
    css: Option<&str>,
) -> Request<Body> {
    let mut fields = vec![("name", name), ("description", description)];
    if let Some(appearance) = appearance {
        fields.push(("appearance", appearance));
    }
    theme_multipart(
        "/app/admin/themes/custom",
        cookie,
        &fields,
        css.map(|css| ("css", "theme.css", css.as_bytes())),
    )
}

async fn custom_theme_rows(app: &TestApp) -> Vec<urlet::db::custom_themes::CustomThemeRow> {
    urlet::db::custom_themes::all(&app.state.db)
        .await
        .expect("custom theme rows")
}

#[tokio::test]
async fn an_administrator_can_upload_manage_and_delete_a_custom_theme() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    let css = ":root { --canvas: #101820; --ink: #f2f2f2; }\n\
               body { background: var(--canvas); color: var(--ink); }";
    let response = app
        .send(upload_request(
            &cookie,
            "Night Bloom",
            "A dark test theme",
            Some(css),
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    assert_eq!(
        response.header("location").as_deref(),
        Some("/app/admin/themes?ok=theme_created")
    );

    // The list links to the shared form instead of embedding the upload form,
    // and the dialog is the enhanced path to the same pages.
    let page = app.get_as("/app/admin/themes", &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Night Bloom"), "{}", page.body);
    assert!(page.body.contains("/app/admin/themes/custom/"));
    assert!(!page.body.contains("data-theme-drop-input"));
    assert!(page.body.contains("<dialog class=\"form-dialog\""));
    assert!(page.body.contains("data-theme-dialog-root"));
    assert!(page.body.contains("data-theme-dialog"));
    assert!(page.body.contains("/app/admin/themes/new"));

    // The upload page renders the same form the edit page does.
    let new_page = app.get_as("/app/admin/themes/new", &cookie).await;
    assert_eq!(new_page.status, StatusCode::OK, "{}", new_page.body);
    assert!(new_page.body.contains("data-theme-form-panel"));
    assert!(new_page.body.contains("data-theme-form"));
    assert!(new_page
        .body
        .contains("action=\"/app/admin/themes/custom\""));
    assert!(new_page.body.contains("data-theme-drop"));
    assert!(new_page.body.contains("data-theme-drop-input"));
    assert!(new_page
        .body
        .contains("name=\"appearance\" value=\"auto\" checked"));
    // Deleting only exists once the upload has an id.
    assert!(!new_page.body.contains("Deleting removes the uploaded file"));

    let rows = custom_theme_rows(&app).await;
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert!(row.key.starts_with("custom-"));
    let key = row.key.clone();
    let id = row.id;

    // The CSS is on the persistent volume, and a fresh registry (the state a
    // restart rebuilds) finds the upload through the database.
    let path = urlet::services::custom_theme::css_path(
        &app.state.config.themes_dir,
        &row.key,
        &row.css_version,
    );
    let stored = std::fs::read_to_string(&path).expect("stylesheet on disk");
    assert!(stored.contains("--canvas"));
    let reloaded = urlet::services::theme::ThemeRegistry::load(&app.state.db)
        .await
        .expect("reload registry");
    assert!(reloaded.is_enabled(&key));
    assert!(
        reloaded
            .known(&key, urlet::i18n::Lang::En)
            .expect("known custom theme")
            .is_custom
    );

    // The catalogue API exposes the upload with a hashed asset URL.
    let api = app.get_as("/api/v1/admin/themes", &cookie).await;
    assert_eq!(api.status, StatusCode::OK);
    let body = api.json();
    let custom = body["themes"]
        .as_array()
        .expect("themes")
        .iter()
        .find(|theme| theme["key"] == key)
        .expect("custom theme in API");
    assert_eq!(custom["custom"], true);
    assert_eq!(custom["enabled"], true);
    assert_eq!(custom["default"], false);
    assert_eq!(custom["name"], "Night Bloom");
    let css_url = custom["css_url"].as_str().expect("css url").to_string();
    assert!(css_url.starts_with("/assets/custom-themes/custom-"));
    assert!(css_url.contains("?v="));

    let asset = app.get(&css_url).await;
    assert_eq!(asset.status, StatusCode::OK, "{}", asset.body);
    assert!(asset.body.contains("--canvas"));
    // The upload composes with the base Verdant stylesheet, so a small colour
    // override still gets the complete layout.
    assert!(asset.body.contains("--forest"), "base stylesheet missing");
    assert_eq!(
        asset.header("content-type").as_deref(),
        Some("text/css; charset=utf-8")
    );

    let edit = app
        .get_as(&format!("/app/admin/themes/custom/{id}"), &cookie)
        .await;
    assert_eq!(edit.status, StatusCode::OK, "{}", edit.body);
    assert!(edit.body.contains("Night Bloom"));
    assert!(edit.body.contains(&key));
    assert!(edit.body.contains("data-theme-form-panel"));
    assert!(edit.body.contains("data-theme-form"));
    assert!(edit
        .body
        .contains(&format!("action=\"/app/admin/themes/custom/{id}\"")));
    assert!(edit.body.contains("data-theme-drop"));
    assert!(edit.body.contains("data-theme-drop-input"));
    // The edit page keeps the delete flow the upload page has no id for.
    assert!(edit.body.contains("Deleting removes the uploaded file"));

    // Make the upload the only enabled theme and the default.
    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "enabled": ["verdant", key], "default": key }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);

    let landing = app.get("/").await;
    assert_eq!(landing.status, StatusCode::OK);
    assert!(landing.body.contains(&css_url), "{}", landing.body);
    assert!(landing.body.contains(&format!("data-theme-key=\"{key}\"")));
    let admin_landing = app.get_as("/", &cookie).await;
    assert!(admin_landing.body.contains("Night Bloom"));
    assert!(admin_landing
        .body
        .contains(&format!("href=\"/app/theme/{key}\"")));

    // A member can select the uploaded theme through the ordinary picker.
    let member_cookie = member(&app, "member@example.com").await;
    let switched = app
        .send(get_as(
            &format!("/app/theme/{key}?next=/app"),
            &member_cookie,
        ))
        .await;
    assert_eq!(switched.status, StatusCode::FOUND);
    let theme_cookie = switched.cookie_named("urlet_theme").expect("theme cookie");
    assert!(theme_cookie.contains(&key));
    let member_page = app
        .get_as("/app", &format!("{member_cookie}; {theme_cookie}"))
        .await;
    assert_eq!(member_page.status, StatusCode::OK);
    assert!(member_page
        .body
        .contains(&format!("data-theme-key=\"{key}\"")));

    // The success flash is theme-specific, not the link wording.
    let flash = app
        .get_as("/app/admin/themes?ok=theme_created", &cookie)
        .await;
    assert!(flash.body.contains("Custom theme uploaded."));

    // Replace the stylesheet and mark the result dark-only. The content hash
    // changes the public URL, so the browser cache is not pinned to old CSS.
    let new_css = "body { background: #222222; }";
    let response = app
        .send(theme_multipart(
            &format!("/app/admin/themes/custom/{id}"),
            &cookie,
            &[
                ("name", "Night Bloom v2"),
                ("description", "Replaced stylesheet"),
                ("is_dark_only", "on"),
            ],
            Some(("css", "night.css", new_css.as_bytes())),
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    assert_eq!(
        response.header("location").as_deref(),
        Some(format!("/app/admin/themes/custom/{id}?ok=theme_updated").as_str())
    );

    let api = app.get_as("/api/v1/admin/themes", &cookie).await;
    let body = api.json();
    let custom = body["themes"]
        .as_array()
        .expect("themes")
        .iter()
        .find(|theme| theme["key"] == key)
        .expect("updated custom theme");
    let new_url = custom["css_url"].as_str().expect("css url").to_string();
    assert_ne!(new_url, css_url);
    assert_eq!(custom["dark_only"], true);
    assert_eq!(custom["name"], "Night Bloom v2");
    let asset = app.get(&new_url).await;
    assert_eq!(asset.status, StatusCode::OK);
    assert!(asset.body.contains("#222222"));
    assert!(asset.body.contains("--forest"), "base stylesheet missing");
    let landing = app.get("/").await;
    assert!(landing.body.contains(&new_url));
    assert!(!landing.body.contains("data-theme-toggle"));

    // A browser submits an empty file part when the picker was not touched.
    // The edit must keep the current stylesheet instead of rejecting it.
    let response = app
        .send(theme_multipart(
            &format!("/app/admin/themes/custom/{id}"),
            &cookie,
            &[
                ("name", "Night Bloom v3"),
                ("description", "Metadata-only edit"),
                ("appearance", "light"),
            ],
            Some(("css", "", b"")),
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    assert_eq!(
        response.header("location").as_deref(),
        Some(format!("/app/admin/themes/custom/{id}?ok=theme_updated").as_str())
    );

    // A default theme cannot be deleted: the administrator has to move the
    // default first.
    let response = app
        .send(post_form_as(
            &format!("/app/admin/themes/custom/{id}/delete"),
            "",
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert_eq!(
        response.header("location").as_deref(),
        Some("/app/admin/themes?err=theme_default")
    );
    assert_eq!(custom_theme_rows(&app).await.len(), 1);

    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "enabled": ["verdant", key], "default": "verdant" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);

    let response = app
        .send(post_form_as(
            &format!("/app/admin/themes/custom/{id}/delete"),
            "",
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert_eq!(
        response.header("location").as_deref(),
        Some("/app/admin/themes?ok=theme_deleted")
    );
    assert!(custom_theme_rows(&app).await.is_empty());
    // The row is gone, so the asset route can no longer resolve the key.
    let asset = app.get(&new_url).await;
    assert_eq!(asset.status, StatusCode::NOT_FOUND);
    let leftovers: Vec<_> = std::fs::read_dir(&app.state.config.themes_dir)
        .expect("theme directory")
        .collect();
    assert!(leftovers.is_empty(), "stylesheet was not removed");
    let landing = app.get("/").await;
    assert!(landing.body.contains("/assets/app.css"));

    let audit = app.get_as("/api/v1/admin/audit", &cookie).await;
    assert_eq!(audit.status, StatusCode::OK);
    for action in [
        "admin.theme_created",
        "admin.theme_updated",
        "admin.theme_deleted",
    ] {
        assert!(
            audit.body.contains(action),
            "missing {action}: {}",
            audit.body
        );
    }
}

#[tokio::test]
async fn custom_theme_uploads_are_validated_and_never_reach_the_volume() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    for (name, css, message) in [
        (
            "External",
            "body { background: url(https://example.com/x.png); }",
            "The stylesheet is invalid",
        ),
        (
            "Imported",
            "@import url(/assets/app.css);\nbody { color: red; }",
            "The stylesheet is invalid",
        ),
        (
            "Not CSS",
            "this is not a stylesheet",
            "The stylesheet is invalid",
        ),
        ("", "body { color: red; }", "Use a name of 1-60 characters"),
    ] {
        let response = app.send(upload_request(&cookie, name, "", Some(css))).await;
        // A rejected upload re-renders the shared form with 422 and the values
        // the administrator typed, instead of bouncing to the list page that
        // no longer carries the form.
        assert_eq!(
            response.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{name:?}: {}",
            response.body
        );
        assert!(response.body.contains("data-theme-form-panel"), "{name:?}");
        assert!(response.body.contains("data-theme-form"), "{name:?}");
        assert!(
            response.body.contains(message),
            "{name:?} lost its flash: {}",
            response.body
        );
        assert!(
            response.body.contains(&format!("value=\"{name}\"")),
            "{name:?} lost the submitted name"
        );
    }

    // No file at all is rejected before the service is called.
    let response = app.send(upload_request(&cookie, "Empty", "", None)).await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        response.body.contains("The stylesheet is invalid"),
        "{}",
        response.body
    );
    assert!(response.body.contains("value=\"Empty\""));

    // Oversized CSS reaches the service (the multipart limit is larger) and is
    // rejected by the stylesheet validator.
    let huge = format!("body {{ color: red; }}/*{}*/", "x".repeat(260 * 1024));
    let response = app
        .send(upload_request(&cookie, "Huge", "", Some(&huge)))
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        response.body.contains("The stylesheet is invalid"),
        "{}",
        response.body
    );

    assert!(custom_theme_rows(&app).await.is_empty());
    let api = app.get_as("/api/v1/admin/themes", &cookie).await;
    assert!(api.json()["themes"]
        .as_array()
        .expect("themes")
        .iter()
        .all(|theme| theme["custom"] == false));

    // A body the multipart reader cannot parse still belongs on the upload
    // form: the list page no longer carries one.
    let response = app
        .send(post_form_as("/app/admin/themes/custom", "name=x", &cookie))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert_eq!(
        response.header("location").as_deref(),
        Some("/app/admin/themes/new?err=theme_upload")
    );

    // A missing upload answers the same not-found redirect as an unknown id.
    let response = app
        .send(post_form_as(
            "/app/admin/themes/custom/9999/delete",
            "",
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert_eq!(
        response.header("location").as_deref(),
        Some("/app/admin/themes?err=not_found")
    );
}

#[tokio::test]
async fn only_administrators_can_upload_or_edit_custom_themes() {
    let app = TestApp::spawn().await;
    admin(&app).await;
    let member_cookie = member(&app, "member@example.com").await;

    let response = app
        .send(upload_request(
            &member_cookie,
            "Member theme",
            "",
            Some("body { color: red; }"),
        ))
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
    let response = app
        .get_as("/app/admin/themes/custom/1", &member_cookie)
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
    let response = app.get_as("/app/admin/themes/new", &member_cookie).await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
    let response = app
        .send(post_form_as(
            "/app/admin/themes/custom/1/delete",
            "",
            &member_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert!(custom_theme_rows(&app).await.is_empty());
}

#[tokio::test]
async fn a_light_only_custom_theme_forces_the_light_palette() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    let response = app
        .send(upload_request_with_appearance(
            &cookie,
            "Paper White",
            "A light-only reading theme",
            Some("light"),
            Some("body { background: #ffffff; color: #111111; }"),
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);

    let rows = custom_theme_rows(&app).await;
    assert_eq!(rows.len(), 1);
    let key = rows[0].key.clone();
    let id = rows[0].id;

    let api = app.get_as("/api/v1/admin/themes", &cookie).await;
    let body = api.json();
    let custom = body["themes"]
        .as_array()
        .expect("themes")
        .iter()
        .find(|theme| theme["key"] == key)
        .expect("custom theme");
    assert_eq!(custom["appearance"], "light");
    assert_eq!(custom["light_only"], true);
    assert_eq!(custom["dark_only"], false);

    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "enabled": ["verdant", key], "default": key }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);

    // The lock is rendered by the server, so it works before JavaScript runs
    // and for visitors with JavaScript disabled.
    let landing = app.get("/").await;
    assert_eq!(landing.status, StatusCode::OK);
    assert!(
        landing.body.contains("data-theme=\"light\""),
        "{}",
        landing.body
    );
    assert!(landing.body.contains("data-theme-lock=\"light\""));
    assert!(!landing.body.contains("data-theme-toggle"));

    // A visitor preference cannot override a forced theme: the middleware
    // still renders the light lock, and theme.js will keep it.
    let forced = app.get_as("/", "urlet_theme=paper; urlet_lang=en").await;
    assert!(forced.body.contains("data-theme=\"light\""));

    // Editing the upload to dark flips the server lock immediately.
    let response = app
        .send(theme_multipart(
            &format!("/app/admin/themes/custom/{id}"),
            &cookie,
            &[
                ("name", "Paper Night"),
                ("description", "Now dark-only"),
                ("appearance", "dark"),
            ],
            None,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    let landing = app.get("/").await;
    assert!(
        landing.body.contains("data-theme=\"dark\""),
        "{}",
        landing.body
    );
    assert!(landing.body.contains("data-theme-lock=\"dark\""));
    assert!(!landing.body.contains("data-theme-toggle"));

    // Auto restores the ordinary control.
    let response = app
        .send(theme_multipart(
            &format!("/app/admin/themes/custom/{id}"),
            &cookie,
            &[
                ("name", "Paper Auto"),
                ("description", "Follows the system"),
                ("appearance", "auto"),
            ],
            None,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    let landing = app.get("/").await;
    assert!(!landing.body.contains("data-theme=\"light\""));
    assert!(!landing.body.contains("data-theme=\"dark\""));
    assert!(!landing.body.contains("data-theme-lock"));
    assert!(landing.body.contains("data-theme-toggle"));
}

#[tokio::test]
async fn every_theme_renders_the_splash_page() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let created = app
        .send(common::post_json_as(
            "/api/v1/urls",
            json!({
                "target_url": "https://example.com/themed",
                "slug": "themed-splash",
                "redirect_mode": "splash",
                "splash_title": "Theme <b>splash</b>"
            }),
            &cookie,
        ))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);

    for key in ["verdant", "aurora", "paper", "signal", "gilded", "swiss"] {
        let switched = app
            .send(get_as(&format!("/app/theme/{key}?next=/app"), &cookie))
            .await;
        assert_eq!(
            switched.status,
            StatusCode::FOUND,
            "{key}: {}",
            switched.body
        );
        let theme_cookie = switched.cookie_named("urlet_theme").expect("theme cookie");
        let both = format!("{cookie}; {theme_cookie}");
        let page = app.get_as("/themed-splash", &both).await;
        assert_eq!(page.status, StatusCode::OK, "{key}: {}", page.body);
        assert!(
            page.body.contains("Theme &lt;b&gt;splash&lt;/b&gt;"),
            "{key} lost or failed to escape the copy"
        );
        assert!(
            !page.body.contains("<b>splash</b>"),
            "{key} rendered the custom heading as HTML"
        );
        assert!(
            page.body.contains(&format!("data-theme-key=\"{key}\"")),
            "{key} did not render its own shell"
        );
        assert!(
            page.body.contains("Continue now"),
            "{key} did not render the splash body"
        );
    }
}
