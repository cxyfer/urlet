// Mount the embedded API reference.
//
// The Content Security Policy allows scripts only from this origin
// (`script-src 'self'`, with no `'unsafe-inline'`), so the page cannot inline a
// configuration object. The spec URL travels in a data attribute instead, which
// leaves the policy untouched.
//
// The element id and the attribute name deliberately avoid `api-reference` and
// `data-spec-url`: Scalar scans the document for those and configures itself
// through its deprecated HTML API, which ignores the options passed here —
// including `withDefaultFonts`, so the page would then pull a web font from
// fonts.scalar.com and have it blocked by `font-src 'self'`.
(() => {
    "use strict";

    const mount = document.getElementById("urlet-api-reference");
    if (!mount || typeof Scalar === "undefined") {
        return;
    }

    Scalar.createApiReference(mount, {
        url: mount.dataset.openapiUrl,
        // The reference must not fetch a web font: `font-src 'self'` would
        // refuse it, and a self-hosted instance may have no outbound network at
        // all. Scalar falls back to the system font stack.
        withDefaultFonts: false,
    });
})();
