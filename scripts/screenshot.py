#!/usr/bin/env python3
"""Screenshot the uwurls UI for visual verification."""

import sys

from playwright.sync_api import sync_playwright

BASE = sys.argv[1]
OUT = sys.argv[2]


def shoot(page, path, name, full=True):
    page.goto(f"{BASE}{path}", wait_until="load")
    page.wait_for_timeout(250)
    page.screenshot(path=f"{OUT}/{name}.png", full_page=full)
    print(f"  {name}: {page.title()}")


with sync_playwright() as playwright:
    browser = playwright.chromium.launch()
    context = browser.new_context(viewport={"width": 1280, "height": 900}, device_scale_factor=2)
    page = context.new_page()

    shoot(page, "/app/login", "01-login", full=False)
    shoot(page, "/app/register", "02-register", full=False)

    page.goto(f"{BASE}/app/login")
    page.fill("#email", "owner@example.com")
    page.fill("#password", "password123")
    page.click("button[type=submit]")
    page.wait_for_url(f"{BASE}/app")
    page.wait_for_timeout(300)
    page.screenshot(path=f"{OUT}/03-dashboard.png", full_page=True)

    page.goto(f"{BASE}/app?sort=clicks")
    page.wait_for_timeout(250)
    page.screenshot(path=f"{OUT}/04-dashboard-sorted.png", full_page=True)

    page.goto(f"{BASE}/app/urls/1")
    page.wait_for_timeout(300)
    page.screenshot(path=f"{OUT}/05-url-detail.png", full_page=True)

    page.goto(f"{BASE}/app/account")
    page.wait_for_timeout(250)
    page.screenshot(path=f"{OUT}/06-account.png", full_page=True)

    shoot(page, "/deck", "07-password-page", full=False)
    shoot(page, "/archived", "08-expired-410", full=False)
    shoot(page, "/does-not-exist", "09-not-found-404", full=False)

    # Dark mode variant of the dashboard.
    dark = browser.new_context(
        viewport={"width": 1280, "height": 900}, device_scale_factor=2, color_scheme="dark"
    )
    dark_page = dark.new_page()
    dark_page.goto(f"{BASE}/app/login")
    dark_page.fill("#email", "owner@example.com")
    dark_page.fill("#password", "password123")
    dark_page.click("button[type=submit]")
    dark_page.wait_for_url(f"{BASE}/app")
    dark_page.wait_for_timeout(300)
    dark_page.screenshot(path=f"{OUT}/10-dashboard-dark.png", full_page=True)

    # Mobile layout.
    mobile = browser.new_context(viewport={"width": 390, "height": 844}, device_scale_factor=2)
    mobile_page = mobile.new_page()
    mobile_page.goto(f"{BASE}/app/login")
    mobile_page.fill("#email", "owner@example.com")
    mobile_page.fill("#password", "password123")
    mobile_page.click("button[type=submit]")
    mobile_page.wait_for_url(f"{BASE}/app")
    mobile_page.wait_for_timeout(300)
    mobile_page.screenshot(path=f"{OUT}/11-dashboard-mobile.png", full_page=True)

    browser.close()
