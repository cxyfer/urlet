/* uwurls — progressive enhancement only. Every page works without JS. */
(function () {
    "use strict";

    /* The handful of strings JavaScript itself produces. Everything else is
       rendered server side; these are keyed by <html lang="...">. */
    var STRINGS = {
        en: { copied: "Copied", copyFailed: "Copy failed" },
        "zh-Hant": { copied: "已複製", copyFailed: "複製失敗" },
        "zh-Hans": { copied: "已复制", copyFailed: "复制失败" }
    };

    function strings() {
        var lang = document.documentElement.getAttribute("lang") || "en";
        return STRINGS[lang] || STRINGS.en;
    }

    function copyText(text) {
        if (navigator.clipboard && window.isSecureContext) {
            return navigator.clipboard.writeText(text);
        }
        return new Promise(function (resolve, reject) {
            var area = document.createElement("textarea");
            area.value = text;
            area.setAttribute("readonly", "");
            area.style.position = "fixed";
            area.style.opacity = "0";
            document.body.appendChild(area);
            area.select();
            try {
                document.execCommand("copy");
                resolve();
            } catch (err) {
                reject(err);
            } finally {
                document.body.removeChild(area);
            }
        });
    }

    function flash(button, message) {
        var original = button.getAttribute("data-label") || button.textContent;
        button.setAttribute("data-label", original);
        button.textContent = message;
        window.setTimeout(function () { button.textContent = original; }, 1400);
    }

    document.addEventListener("click", function (event) {
        var copyButton = event.target.closest("[data-copy]");
        if (copyButton) {
            event.preventDefault();
            var selector = copyButton.getAttribute("data-copy");
            var source = selector ? document.querySelector(selector) : null;
            var value = source ? (source.value !== undefined ? source.value : source.textContent) : "";
            if (!value) { return; }
            copyText(value.trim()).then(function () {
                flash(copyButton, strings().copied);
            }).catch(function () {
                flash(copyButton, strings().copyFailed);
            });
            return;
        }

        var confirmable = event.target.closest("[data-confirm]");
        if (confirmable && !window.confirm(confirmable.getAttribute("data-confirm"))) {
            event.preventDefault();
        }
    });

    /* Carry the current path into the language switcher so the visitor stays on
       the page they were reading — including the landing page at `/`. Without
       JS the links still work and land on the dashboard. */
    document.querySelectorAll("[data-lang-switcher] a").forEach(function (link) {
        var href = link.getAttribute("href");
        if (!href || href.indexOf("next=") !== -1) { return; }
        var next = window.location.pathname + window.location.search;
        if (!next) { return; }
        link.setAttribute("href", href + "?next=" + encodeURIComponent(next));
    });

    /* Close the language dropdown on an outside click. The native <details>
       handles the toggle itself, so this is purely a polish enhancement. */
    document.addEventListener("click", function (event) {
        document.querySelectorAll("details[data-lang-switcher][open]").forEach(function (details) {
            if (!details.contains(event.target)) {
                details.removeAttribute("open");
            }
        });
    });

    document.addEventListener("keydown", function (event) {
        if (event.key !== "Escape") { return; }
        document.querySelectorAll("details[data-lang-switcher][open]").forEach(function (details) {
            details.removeAttribute("open");
            var summary = details.querySelector("summary");
            if (summary) { summary.focus(); }
        });
    });

    /* Reveal password inputs on demand. */
    document.addEventListener("change", function (event) {
        var toggle = event.target.closest("[data-toggle-target]");
        if (!toggle) { return; }
        var target = document.querySelector(toggle.getAttribute("data-toggle-target"));
        if (target) { target.hidden = !toggle.checked; }
    });

    /* Dismiss alerts after a while so success banners do not pile up. */
    window.setTimeout(function () {
        document.querySelectorAll(".alert-ok[data-auto-dismiss]").forEach(function (node) {
            node.style.transition = "opacity .4s";
            node.style.opacity = "0";
            window.setTimeout(function () { node.remove(); }, 400);
        });
    }, 4000);
})();
