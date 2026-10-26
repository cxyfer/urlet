/* urlet — persisted light/dark preference.
 *
 * Loaded synchronously in <head> so the stored choice is applied before the
 * first paint. Without JavaScript the CSS media query keeps following the
 * operating system, and the toggle buttons stay hidden (the markup ships them
 * with the `hidden` attribute).
 */
(function () {
    "use strict";

    var KEY = "urlet-theme";
    var root = document.documentElement;
    var lock = root.getAttribute("data-theme-lock");
    var media = window.matchMedia ? window.matchMedia("(prefers-color-scheme: dark)") : null;

    function readPreference() {
        try {
            var value = window.localStorage.getItem(KEY);
            return value === "light" || value === "dark" ? value : null;
        } catch (error) {
            return null;
        }
    }

    function systemMode() {
        return media && media.matches ? "dark" : "light";
    }

    function currentMode() {
        var explicit = root.getAttribute("data-theme");
        return explicit === "light" || explicit === "dark" ? explicit : systemMode();
    }

    function storePreference(value) {
        try {
            if (value === "light" || value === "dark") {
                window.localStorage.setItem(KEY, value);
            } else {
                window.localStorage.removeItem(KEY);
            }
        } catch (error) {
            /* Private browsing or a blocked storage area: the choice still
               applies to this page, it just will not survive a reload. */
        }
    }

    function updateButtons() {
        var mode = currentMode();
        updateThemeColor(mode);
        document.querySelectorAll("[data-theme-toggle]").forEach(function (button) {
            // A light-only or dark-only theme locks the palette; there is
            // nothing for the visitor to toggle.
            if (lock === "light" || lock === "dark") {
                button.hidden = true;
                return;
            }
            button.hidden = false;
            button.setAttribute("data-mode", mode);
            button.setAttribute("aria-pressed", mode === "dark" ? "true" : "false");
        });
    }

    // A browser paints its chrome from `theme-color`, and that meta can only be
    // selected by a media query — never by `data-theme`. A design that pins the
    // chrome (Signal) marks its tags with the mode they belong to, and the
    // resolved mode turns the matching one on; the two media queries above stay
    // the no-JavaScript path.
    function updateThemeColor(mode) {
        document
            .querySelectorAll('meta[name="theme-color"][data-theme-mode]')
            .forEach(function (meta) {
                meta.setAttribute(
                    "media",
                    meta.getAttribute("data-theme-mode") === mode ? "all" : "not all"
                );
            });
    }

    function toggle() {
        if (lock === "light" || lock === "dark") { return; }
        var next = currentMode() === "dark" ? "light" : "dark";
        root.setAttribute("data-theme", next);
        storePreference(next);
        updateButtons();
    }

    var saved = readPreference();
    if (lock === "light" || lock === "dark") {
        // The server already wrote data-theme; keep the saved preference for
        // the next theme that follows the system.
        root.setAttribute("data-theme", lock);
    } else if (saved) {
        root.setAttribute("data-theme", saved);
    }
    // The marked meta tags sit above this script, so the chrome colour can be
    // corrected before the first paint instead of at DOMContentLoaded.
    updateThemeColor(currentMode());

    document.addEventListener("click", function (event) {
        var button = event.target.closest("[data-theme-toggle]");
        if (!button) { return; }
        event.preventDefault();
        toggle();
    });

    if (media) {
        var onSystemChange = function () {
            if (!root.getAttribute("data-theme")) {
                updateButtons();
            }
        };
        if (media.addEventListener) {
            media.addEventListener("change", onSystemChange);
        } else if (media.addListener) {
            media.addListener(onSystemChange);
        }
    }

    if (document.readyState === "loading") {
        document.addEventListener("DOMContentLoaded", updateButtons);
    } else {
        updateButtons();
    }
})();
