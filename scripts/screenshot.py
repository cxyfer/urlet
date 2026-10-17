#!/usr/bin/env python3
"""Screenshot the urlet UI for the README.

    SHOTS=docs/screenshots ./scripts/demo.sh

`demo.sh` seeds a throwaway instance and calls this script with the base URL and
an output directory. The file names below are the ones the README references, so
pointing `SHOTS` at `docs/screenshots` regenerates the documentation images.
"""

import sys

from playwright.sync_api import sync_playwright

BASE = sys.argv[1]
OUT = sys.argv[2]

VIEWPORT = {"width": 1280, "height": 900}


def shoot(page, path, name, full=True):
    page.goto(f"{BASE}{path}", wait_until="load")
    page.wait_for_timeout(300)
    page.screenshot(path=f"{OUT}/{name}.png", full_page=full)
    print(f"  {name}: {page.title()}")


def login(page):
    page.goto(f"{BASE}/app/login")
    page.fill("#email", "owner@example.com")
    page.fill("#password", "password123")
    page.click("button[type=submit]")
    page.wait_for_url(f"{BASE}/app")
    page.wait_for_timeout(300)


with sync_playwright() as playwright:
    browser = playwright.chromium.launch()

    # ---------------------------------------------------------------- English
    context = browser.new_context(viewport=VIEWPORT, device_scale_factor=2)
    page = context.new_page()

    # The public landing page at `/` is the first thing a visitor sees.
    shoot(page, "/", "landing")
    shoot(page, "/app/login", "login", full=False)
    shoot(page, "/app/register", "register", full=False)

    login(page)
    shoot(page, "/app", "dashboard")
    shoot(page, "/app/urls/1", "url-detail")
    shoot(page, "/app/account", "account")

    for path, name in [
        ("/app/admin", "admin-overview"),
        ("/app/admin/users", "admin-users"),
        ("/app/admin/urls", "admin-urls"),
        ("/app/admin/invites", "admin-invites"),
        ("/app/admin/audit", "admin-audit"),
    ]:
        shoot(page, path, name)

    # Bot protection configured through the admin panel, so the section shows a
    # provider, both keys and an active state. Cloudflare's public test keys.
    page.goto(f"{BASE}/app/admin/settings")
    page.select_option("#captcha_provider", "turnstile")
    page.fill("#captcha_site_key", "1x00000000000000000000AA")
    page.fill("#captcha_secret_key", "1x0000000000000000000000000000000AA")
    page.check("input[name=captcha_on_register]")
    # Scope the click to the settings form: the top bar has its own submit
    # button (sign out), and a bare `button[type=submit]` hits that one first.
    page.click("form[action='/app/admin/settings'] button[type=submit]")
    page.wait_for_load_state("load")
    page.wait_for_timeout(300)
    page.screenshot(path=f"{OUT}/admin-captcha.png", full_page=True)
    print(f"  admin-captcha: {page.title()}")

    # Public link pages.
    shoot(page, "/deck", "password-page", full=False)
    shoot(page, "/archived", "expired-link", full=False)
    shoot(page, "/does-not-exist", "not-found", full=False)

    # ------------------------------------------------- Traditional Chinese UI
    # A `zh-TW` locale makes the browser send `Accept-Language: zh-TW`, which is
    # exactly how a first-time visitor is negotiated onto a language.
    zh = browser.new_context(
        viewport=VIEWPORT, device_scale_factor=2, locale="zh-TW"
    )
    zh_page = zh.new_page()
    shoot(zh_page, "/", "landing-zh-hant")
    login(zh_page)
    shoot(zh_page, "/app", "dashboard-zh-hant")

    # ------------------------------------------------------------------ dark
    dark = browser.new_context(
        viewport=VIEWPORT, device_scale_factor=2, color_scheme="dark"
    )
    dark_page = dark.new_page()
    login(dark_page)
    dark_page.wait_for_timeout(300)
    dark_page.screenshot(path=f"{OUT}/dark.png", full_page=True)
    print(f"  dark: {dark_page.title()}")

    # ---------------------------------------------------------------- mobile
    mobile = browser.new_context(
        viewport={"width": 390, "height": 844}, device_scale_factor=2
    )
    mobile_page = mobile.new_page()
    shoot(mobile_page, "/", "landing-mobile")

    browser.close()
