/* urlet — progressive enhancement only. Every page works without JS. */
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

    /* Carry the current path into the theme picker too. */
    document.querySelectorAll("[data-theme-switcher] a").forEach(function (link) {
        var href = link.getAttribute("href");
        if (!href || href.indexOf("next=") !== -1) { return; }
        var next = window.location.pathname + window.location.search;
        if (!next) { return; }
        link.setAttribute("href", href + "?next=" + encodeURIComponent(next));
    });

    /* Close the language dropdown on an outside click. The native <details>
       handles the toggle itself, so this is purely a polish enhancement. */
    document.addEventListener("click", function (event) {
        document.querySelectorAll("details[data-lang-switcher][open], details[data-theme-switcher][open]").forEach(function (details) {
            if (!details.contains(event.target)) {
                details.removeAttribute("open");
            }
        });
    });

    document.addEventListener("keydown", function (event) {
        if (event.key !== "Escape") { return; }
        document.querySelectorAll("details[data-lang-switcher][open], details[data-theme-switcher][open]").forEach(function (details) {
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

    /* 遊客設定只在允許遊客時顯示；隱藏時仍提交原值，避免遺失設定。
       無 JavaScript 時儲存權限後重新載入，即可顯示對應設定。 */
    var publicAccess = document.getElementById("public_create_access");
    var guestSettings = document.getElementById("public-guest-settings");
    if (publicAccess && guestSettings) {
        function refreshGuestSettings() {
            guestSettings.hidden = publicAccess.value !== "guest";
        }
        publicAccess.addEventListener("change", refreshGuestSettings);
        refreshGuestSettings();
    }

    /* Group form: count the capabilities and offer the bulk switches. Both stay
       hidden until here, so a form without JavaScript is not left with dead
       buttons; the checkboxes themselves work either way. The initializer is
       reusable because the same form is inserted into the dialog after a fetch. */
    function initCapBlock(block) {
        if (block.hasAttribute("data-cap-ready")) { return; }
        block.setAttribute("data-cap-ready", "");
        var boxes = block.querySelectorAll('.cap-item input[type="checkbox"]');
        var count = block.querySelector("[data-cap-count]");
        var template = count ? (count.getAttribute("data-cap-template") || "") : "";

        function refresh() {
            if (!count || !template) { return; }
            var on = 0;
            boxes.forEach(function (box) { if (box.checked) { on += 1; } });
            count.textContent = template
                .replace("{on}", String(on))
                .replace("{total}", String(boxes.length));
            count.hidden = false;
        }

        block.querySelectorAll("[data-cap-bulk]").forEach(function (button) {
            button.hidden = false;
            button.addEventListener("click", function () {
                var on = button.getAttribute("data-cap-bulk") === "on";
                boxes.forEach(function (box) { box.checked = on; });
                refresh();
            });
        });
        boxes.forEach(function (box) { box.addEventListener("change", refresh); });
        refresh();
    }

    function initCapBlocks(root) {
        root.querySelectorAll("[data-cap-block]").forEach(initCapBlock);
    }

    /* The domain allowlist only applies to the "list" policy. Mark the rest
       inert instead of hiding it, so the checked domains still submit and the
       group keeps its allowlist while another policy is selected. */
    function initPolicyBlock(block) {
        if (block.hasAttribute("data-policy-ready")) { return; }
        block.setAttribute("data-policy-ready", "");
        var radios = block.querySelectorAll('input[name="domain_policy"]');
        var pane = block.querySelector("[data-policy-pane]");
        if (!pane || !radios.length) { return; }

        function refresh() {
            var chosen = block.querySelector('input[name="domain_policy"]:checked');
            var active = chosen ? chosen.value === "list" : false;
            pane.classList.toggle("is-inactive", !active);
            if (active) {
                pane.removeAttribute("inert");
            } else {
                pane.setAttribute("inert", "");
            }
        }

        radios.forEach(function (radio) { radio.addEventListener("change", refresh); });
        refresh();
    }

    function initPolicyBlocks(root) {
        root.querySelectorAll("[data-policy-block]").forEach(initPolicyBlock);
    }

    initCapBlocks(document);
    initPolicyBlocks(document);

    /* Create and edit forms in a floating <dialog>. Each list keeps ordinary
       links, so the dedicated form pages remain the no-JavaScript fallback.
       A rejected submission is rendered back into the same dialog with its
       values and error banner intact. One implementation serves the group and
       the theme form; the differences are passed in as config. */
    function initFormDialog(config) {
        var dialog = document.querySelector(config.root);
        if (!dialog || typeof dialog.showModal !== "function"
            || typeof window.fetch !== "function"
            || typeof window.DOMParser !== "function") {
            return;
        }
        var body = dialog.querySelector(config.body);
        var heading = dialog.querySelector(config.heading);
        if (!body) { return; }
        var loadingText = dialog.getAttribute("data-loading") || "Loading…";
        var errorText = dialog.getAttribute("data-error") || "Something went wrong.";
        var sequence = 0;

        function setBusy(busy) {
            if (busy) {
                body.setAttribute("aria-busy", "true");
            } else {
                body.removeAttribute("aria-busy");
            }
        }

        function parse(html) {
            var doc = new DOMParser().parseFromString(html, "text/html");
            return {
                panel: doc.querySelector(config.panel),
                flash: doc.querySelector(".app-main > .alert"),
                title: doc.querySelector(".page-title")
            };
        }

        /* Put the file the visitor already chose back into the re-rendered
           form. Its bytes never reached the server, so without this the
           stylesheet would have to be picked again after a rejected upload. */
        function restoreFiles(root, files) {
            if (!files || !files.length) { return; }
            var form = root.querySelector(config.form);
            var input = form ? form.querySelector('input[type="file"]') : null;
            if (!input) { return; }
            try {
                var transfer = new DataTransfer();
                for (var i = 0; i < files.length; i += 1) {
                    transfer.items.add(files[i]);
                }
                input.files = transfer.files;
                input.dispatchEvent(new Event("change", { bubbles: true }));
            } catch (err) {
                // An old browser without DataTransfer: choose the file again.
            }
        }

        function render(html, files) {
            var parsed = parse(html);
            if (!parsed.panel) { return false; }
            body.textContent = "";
            if (parsed.flash) { body.appendChild(parsed.flash); }
            body.appendChild(parsed.panel);
            if (heading) {
                heading.textContent = parsed.title ? parsed.title.textContent.trim() : "";
            }
            if (config.enhance) { config.enhance(body); }
            restoreFiles(body, files);
            var first = body.querySelector('input:not([type="hidden"]), select, textarea');
            if (first) { first.focus(); }
            return true;
        }

        function showLoading() {
            body.textContent = "";
            var note = document.createElement("p");
            note.className = "hint form-dialog-loading";
            note.textContent = loadingText;
            body.appendChild(note);
            if (heading) { heading.textContent = loadingText; }
            setBusy(true);
        }

        function showError(clear) {
            var previous = body.querySelector(".form-dialog-error");
            if (previous) { previous.remove(); }
            if (clear) { body.textContent = ""; }
            var note = document.createElement("p");
            note.className = "alert alert-error form-dialog-error";
            note.textContent = errorText;
            body.insertBefore(note, body.firstChild);
            setBusy(false);
        }

        /* Carry the page the dialog was opened from, so a saved edit returns
           there instead of to the form's default page. The banner of the
           previous submission is dropped so it is not shown twice. */
        function withNext(url) {
            if (!config.carryNext || url.indexOf("next=") !== -1) { return url; }
            var params = new URLSearchParams(window.location.search);
            params.delete("ok");
            params.delete("err");
            var query = params.toString();
            var next = window.location.pathname + (query ? "?" + query : "");
            return url + (url.indexOf("?") === -1 ? "?" : "&") + "next=" + encodeURIComponent(next);
        }

        function open(url) {
            url = withNext(url);
            sequence += 1;
            var ticket = sequence;
            if (!dialog.open) { dialog.showModal(); }
            showLoading();
            fetch(url, { credentials: "same-origin" })
                .then(function (response) {
                    if (ticket !== sequence) { return null; }
                    if (!response.ok) { return null; }
                    return response.text();
                })
                .then(function (html) {
                    if (ticket !== sequence) { return; }
                    if (html === null || !render(html)) {
                        window.location.assign(url);
                        return;
                    }
                    setBusy(false);
                })
                .catch(function () {
                    if (ticket !== sequence) { return; }
                    showError(true);
                });
        }

        function submit(form) {
            // A submit supersedes a load still in flight, and its own response
            // is dropped if the dialog is closed or reopened while it runs.
            sequence += 1;
            var ticket = sequence;
            var data;
            if (config.multipart) {
                // The upload routes read a multipart body, so the FormData is
                // sent as it is: the file has to stay a File part.
                try {
                    data = new FormData(form);
                } catch (err) {
                    form.submit();
                    return;
                }
            } else {
                try {
                    // The group routes use axum's urlencoded Form extractor, so
                    // convert the FormData instead of sending multipart.
                    data = new URLSearchParams(new FormData(form));
                } catch (err) {
                    // Very old browsers without URLSearchParams still get a
                    // normal urlencoded submission, just without the in-dialog
                    // update.
                    form.submit();
                    return;
                }
            }
            var fileInput = form.querySelector('input[type="file"]');
            var chosen = fileInput ? fileInput.files : null;
            var submitButton = form.querySelector('[type="submit"]');
            setBusy(true);
            if (submitButton) { submitButton.disabled = true; }
            fetch(form.getAttribute("action"), {
                method: (form.getAttribute("method") || "POST").toUpperCase(),
                body: data,
                credentials: "same-origin"
            }).then(function (response) {
                if (response.redirected) {
                    // The submission changed something, so show the result even
                    // if the dialog moved on while the request was in flight.
                    window.location.assign(response.url);
                    return null;
                }
                if (ticket !== sequence) { return null; }
                return response.text();
            }).then(function (html) {
                if (ticket !== sequence || html === null || !dialog.open) { return; }
                if (!render(html, chosen)) {
                    // The server answered with something other than the shared
                    // form (a body limit rejection, for example). Its submit URL
                    // is POST-only and cannot be loaded as a page, so keep the
                    // form that is still on screen and report it in place.
                    if (submitButton) { submitButton.disabled = false; }
                    showError(false);
                    return;
                }
                setBusy(false);
                if (submitButton) { submitButton.disabled = false; }
            }).catch(function () {
                if (ticket !== sequence || !dialog.open) { return; }
                if (submitButton) { submitButton.disabled = false; }
                showError(false);
            });
        }

        document.addEventListener("click", function (event) {
            var link = event.target.closest("[" + config.open + "]");
            // A modified or middle click keeps the browser's own behaviour, so
            // the form page can still be opened in a new tab.
            if (link && (event.button !== 0 || event.metaKey || event.ctrlKey
                || event.shiftKey || event.altKey)) {
                return;
            }
            if (link) {
                event.preventDefault();
                open(link.getAttribute("href"));
                return;
            }
            var close = event.target.closest("[" + config.close + "]");
            if (close) {
                event.preventDefault();
                dialog.close();
                return;
            }
            if (event.target === dialog) {
                dialog.close();
            }
        });

        dialog.addEventListener("submit", function (event) {
            var form = event.target.closest(config.form);
            if (!form) { return; }
            event.preventDefault();
            submit(form);
        });

        dialog.addEventListener("close", function () {
            sequence += 1;
            body.textContent = "";
            setBusy(false);
            if (heading) { heading.textContent = ""; }
        });
    }

    /* Theme uploads: turn the styled label into a real drop target. The native
       input remains the form control, so selecting a file still works when
       JavaScript is unavailable. */
    function formatBytes(bytes) {
        if (!bytes) { return ""; }
        if (bytes >= 1048576) { return (bytes / 1048576).toFixed(1) + " MB"; }
        if (bytes >= 1024) { return (bytes / 1024).toFixed(1) + " KB"; }
        return bytes + " B";
    }

    function initThemeDrop(container) {
        if (container.hasAttribute("data-theme-drop-ready")) { return; }
        container.setAttribute("data-theme-drop-ready", "");
        var input = container.querySelector("[data-theme-drop-input]");
        var zone = container.querySelector("[data-theme-drop-zone]");
        var fileBox = container.querySelector("[data-theme-drop-file]");
        var nameBox = container.querySelector("[data-theme-drop-name]");
        var sizeBox = container.querySelector("[data-theme-drop-size]");
        var errorBox = container.querySelector("[data-theme-drop-error]");
        var clear = container.querySelector("[data-theme-drop-clear]");
        if (!input || !zone || !fileBox) { return; }

        function showError(message) {
            if (!errorBox) { return; }
            errorBox.textContent = message || "";
            errorBox.hidden = !message;
            container.classList.toggle("is-invalid", !!message);
        }

        function render() {
            var chosen = input.files && input.files.length ? input.files[0] : null;
            container.classList.toggle("has-file", !!chosen);
            fileBox.hidden = !chosen;
            zone.hidden = !!chosen;
            if (nameBox) { nameBox.textContent = chosen ? chosen.name : ""; }
            if (sizeBox) { sizeBox.textContent = chosen ? formatBytes(chosen.size) : ""; }
        }

        function accept(files) {
            if (!files || !files.length) { return; }
            var candidate = files[0];
            var isCss = /\.css$/i.test(candidate.name) || candidate.type === "text/css";
            if (!isCss) {
                showError(container.getAttribute("data-drop-invalid") || "Choose a .css file.");
                return;
            }
            try {
                var transfer = new DataTransfer();
                transfer.items.add(candidate);
                input.files = transfer.files;
            } catch (err) {
                try {
                    input.files = files;
                } catch (err2) {
                    return;
                }
            }
            showError("");
            render();
        }

        input.addEventListener("change", function () {
            showError("");
            render();
        });
        input.addEventListener("invalid", function () {
            showError(container.getAttribute("data-drop-required") || "Choose a CSS file.");
        });

        ["dragenter", "dragover"].forEach(function (type) {
            container.addEventListener(type, function (event) {
                event.preventDefault();
                if (event.dataTransfer) { event.dataTransfer.dropEffect = "copy"; }
                container.classList.add("is-dragging");
            });
        });
        ["dragleave", "dragend", "drop"].forEach(function (type) {
            container.addEventListener(type, function (event) {
                if (type === "dragleave" && container.contains(event.relatedTarget)) { return; }
                container.classList.remove("is-dragging");
            });
        });
        container.addEventListener("drop", function (event) {
            event.preventDefault();
            if (event.dataTransfer && event.dataTransfer.files && event.dataTransfer.files.length) {
                accept(event.dataTransfer.files);
            }
        });

        if (clear) {
            clear.addEventListener("click", function () {
                input.value = "";
                showError("");
                render();
                input.focus();
            });
        }
        render();
    }

    function initThemeDrops(root) {
        root.querySelectorAll("[data-theme-drop]").forEach(initThemeDrop);
    }

    initThemeDrops(document);

    /* The lists above carry the links; wire each one to its dialog. */
    initFormDialog({
        root: "[data-group-dialog-root]",
        open: "data-group-dialog",
        close: "data-group-dialog-close",
        body: "[data-group-dialog-body]",
        heading: "#group-dialog-title",
        panel: "[data-group-form-panel]",
        form: "form[data-group-form]",
        enhance: function (root) {
            initCapBlocks(root);
            initPolicyBlocks(root);
        }
    });

    initFormDialog({
        root: "[data-theme-dialog-root]",
        open: "data-theme-dialog",
        close: "data-theme-dialog-close",
        body: "[data-theme-dialog-body]",
        heading: "#theme-dialog-title",
        panel: "[data-theme-form-panel]",
        form: "form[data-theme-form]",
        multipart: true,
        enhance: initThemeDrops
    });

    /* 預覽來源只啟用對應欄位；無 JS 時仍可明確選擇來源並提交。 */
    function initPreviewForm(form) {
        if (form.hasAttribute("data-preview-ready")) { return; }
        form.setAttribute("data-preview-ready", "");
        var source = form.querySelector("[data-preview-source]");
        if (!source) { return; }
        function updatePreviewSource() {
            form.querySelectorAll("[data-preview-input]").forEach(function (group) {
                var active = group.getAttribute("data-preview-input") === source.value;
                group.hidden = !active;
                group.querySelectorAll("input").forEach(function (input) {
                    input.disabled = !active;
                });
            });
        }
        source.addEventListener("change", updatePreviewSource);
        updatePreviewSource();
    }

    /* 中轉頁的倒數與文案只在選擇中轉頁時顯示。隱藏的欄位照常送出，
       切回直接跳轉再切回來時先前輸入的文案仍在。 */
    function initSplashFields(form) {
        if (form.hasAttribute("data-splash-ready")) { return; }
        form.setAttribute("data-splash-ready", "");
        var select = form.querySelector("[data-mode-select]");
        var fields = form.querySelector("[data-splash-fields]");
        if (!select || !fields) { return; }
        function refresh() {
            fields.hidden = select.value !== "splash";
        }
        select.addEventListener("change", refresh);
        refresh();
    }

    /* 圖片超過上限時在瀏覽器端就攔下，不必把整份表單送出才得知失敗。
       以 capture 階段執行，先於懸浮窗的送出處理。 */
    document.addEventListener("submit", function (event) {
        var form = event.target;
        if (!form.matches || !form.matches("form[data-link-form]")) { return; }
        var inputs = form.querySelectorAll('input[type="file"][data-max-bytes]');
        for (var i = 0; i < inputs.length; i += 1) {
            var input = inputs[i];
            var limit = parseInt(input.getAttribute("data-max-bytes"), 10);
            var file = input.files && input.files[0];
            if (!input.disabled && file && limit && file.size > limit) {
                event.preventDefault();
                event.stopImmediatePropagation();
                var section = input.closest("details");
                if (section) { section.open = true; }
                input.setCustomValidity(input.getAttribute("data-too-large") || "");
                input.reportValidity();
                return;
            }
        }
    }, true);

    document.addEventListener("change", function (event) {
        if (event.target.matches && event.target.matches('input[type="file"][data-max-bytes]')) {
            event.target.setCustomValidity("");
        }
    });

    function initLinkForms(root) {
        root.querySelectorAll("[data-preview-form]").forEach(initPreviewForm);
        root.querySelectorAll("form[data-link-form]").forEach(initSplashFields);
    }

    initLinkForms(document);

    initFormDialog({
        root: "[data-link-dialog-root]",
        open: "data-link-dialog",
        close: "data-link-dialog-close",
        body: "[data-link-dialog-body]",
        heading: "#link-dialog-title",
        panel: "[data-link-form-panel]",
        form: "form[data-link-form]",
        multipart: true,
        carryNext: true,
        enhance: initLinkForms
    });

    /* Server clock: the landing page renders the server's time once, with the
       epoch it was read at and the zone it was read in. Ticking locally from
       that anchor keeps the display honest — no polling, and the browser's own
       time zone is never substituted for the server's. Without JavaScript the
       rendered time stays as it was captured, which is still true. */
    function initServerClock() {
        var anchors = document.querySelectorAll("[data-server-clock][data-epoch-ms]");
        if (!anchors.length) { return; }
        var anchor = anchors[0];
        var epoch = Number(anchor.getAttribute("data-epoch-ms"));
        var zone = anchor.getAttribute("data-zone") || "";
        if (!isFinite(epoch) || zone.charAt(0) === "+" || zone.charAt(0) === "-") {
            /* No IANA zone name to format in: leave the server's text alone. */
            return;
        }
        var format;
        try {
            format = new Intl.DateTimeFormat("en-GB", {
                timeZone: zone,
                hourCycle: "h23",
                hour: "2-digit",
                minute: "2-digit",
                second: "2-digit"
            });
        } catch (err) {
            return;
        }
        var rendered = Date.now();
        function tick() {
            var at = epoch + (Date.now() - rendered);
            var text = format.format(new Date(at));
            anchors.forEach(function (node) { node.textContent = text; });
            /* Re-align to the next whole second so the readout does not drift. */
            window.setTimeout(tick, 1000 - (Date.now() % 1000));
        }
        tick();
    }

    initServerClock();

    /* Dismiss alerts after a while so success banners do not pile up. */
    window.setTimeout(function () {
        document.querySelectorAll(".alert-ok[data-auto-dismiss]").forEach(function (node) {
            node.style.transition = "opacity .4s";
            node.style.opacity = "0";
            window.setTimeout(function () { node.remove(); }, 400);
        });
    }, 4000);
})();
