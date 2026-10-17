/* uwurls — persisted light/dark preference.
 *
 * Loaded synchronously in <head> so the stored choice is applied before the
 * first paint. Without JavaScript the CSS media query keeps following the
 * operating system, and the toggle buttons stay hidden (the markup ships them
 * with the `hidden` attribute).
 */
(function () {
    "use strict";

    var KEY = "uwurls-theme";
    var root = document.documentElement;
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
        document.querySelectorAll("[data-theme-toggle]").forEach(function (button) {
            button.hidden = false;
            button.setAttribute("data-mode", mode);
            button.setAttribute("aria-pressed", mode === "dark" ? "true" : "false");
        });
    }

    function toggle() {
        var next = currentMode() === "dark" ? "light" : "dark";
        root.setAttribute("data-theme", next);
        storePreference(next);
        updateButtons();
    }

    var saved = readPreference();
    if (saved) {
        root.setAttribute("data-theme", saved);
    }

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
