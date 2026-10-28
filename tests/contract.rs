//! Drift protection between the axum router and the published OpenAPI contract.
//!
//! `docs-site/` and the binary's `/docs` page both describe the API from
//! `openapi/`, and nothing in the type system connects that document to the
//! router. A handler added without a contract entry would ship silently, and a
//! contract entry that no longer matches a handler would be discovered by a
//! client rather than by CI.
//!
//! axum exposes no way to read a built router back, so the surface is read from
//! the router's own source: the `.route(...)` calls are the registration, and
//! the path literals in them are what the router actually serves. The parse
//! fails loudly on a shape it does not understand, and a size guard below makes
//! a parser that silently matches nothing fail the test rather than pass it.

use std::collections::BTreeSet;

use serde_json::Value;

/// Every router source this test reads, paired with its path.
///
/// `include_str!` needs a literal, so the pairing is written out rather than
/// derived from a list of names. Every consumer below iterates this one array,
/// which is the point: a guard that tells a maintainer to add a module has to be
/// the same collection the parser reads, or adding the name would silence the
/// guard without parsing anything.
const API_SOURCES: &[(&str, &str)] = &[
    ("src/routes/api.rs", include_str!("../src/routes/api.rs")),
    (
        "src/routes/previews.rs",
        include_str!("../src/routes/previews.rs"),
    ),
];

/// The root router, which serves the JSON endpoints outside `/api/v1`.
///
/// It is not part of `API_SOURCES`: its paths are absolute and most of them are
/// HTML pages, so folding it into the API surface would compare the wrong thing.
/// `the_endpoints_outside_the_api_match_the_router` reads it separately.
const ROOT_SOURCE: (&str, &str) = ("src/server.rs", include_str!("../src/server.rs"));

/// Prefix the JSON API is nested under.
const API_PREFIX: &str = "/api/v1";

/// Paths the contract documents that are not part of `/api/v1`.
///
/// They answer JSON but live at the root: the health probes and the link
/// unlock. They are compared against the root router rather than the API one.
const OUTSIDE_THE_API: &[&str] = &["/health", "/healthz", "/{slug}/unlock"];

/// Every method axum can register through a `MethodRouter` builder.
const METHODS: &[&str] = &[
    "get", "post", "put", "patch", "delete", "head", "options", "trace",
];

/// Strip whole-line `//` comments.
///
/// Without this a commented-out `.route(...)` becomes a phantom registration and
/// the comparison fails on a contract that is in fact correct. Only whole-line
/// comments are removed: a trailing `//` cannot be told apart from the `//` in a
/// URL without tracking string literals, and guessing would be worse than
/// leaving it.
fn without_comments(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The `(METHOD, path)` pairs a router source registers.
///
/// Reads `.route("<path>", <method router>)` calls from flattened source. The
/// path must be a plain string literal and the parentheses must balance; either
/// failing is a parse error rather than a silently skipped route.
fn registrations(source: &str, file: &str) -> Vec<(String, String)> {
    let flat = without_comments(source)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut found = Vec::new();
    let mut rest = flat.as_str();

    while let Some(start) = rest.find(".route(") {
        let after = &rest[start + ".route(".len()..];
        let mut depth = 1usize;
        let mut end = None;
        for (index, character) in after.char_indices() {
            match character {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(index);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = end.unwrap_or_else(|| panic!("{file}: unbalanced .route( arguments"));
        let arguments = &after[..close];

        let open_quote = arguments
            .find('"')
            .unwrap_or_else(|| panic!("{file}: .route( has no path literal"));
        let close_quote = arguments[open_quote + 1..]
            .find('"')
            .unwrap_or_else(|| panic!("{file}: .route( path literal is not closed"))
            + open_quote
            + 1;
        let path = &arguments[open_quote + 1..close_quote];
        assert!(
            path.starts_with('/'),
            "{file}: .route path {path:?} is not absolute"
        );

        // The methods are the `get(...)`, `post(...)` … builders chained after
        // the path. A handler reference such as `api_urls::get` has no call
        // parenthesis, so it cannot be mistaken for one.
        let handlers = &arguments[close_quote + 1..];
        let mut served: Vec<&str> = METHODS
            .iter()
            .copied()
            .filter(|method| handlers.contains(&format!("{method}(")))
            .collect();
        served.sort_unstable();
        assert!(
            !served.is_empty(),
            "{file}: .route({path:?}, …) registers no method"
        );
        for method in served {
            found.push((method.to_uppercase(), path.to_string()));
        }

        rest = &after[close + 1..];
    }

    found
}

/// The router's surface, as `(METHOD, /api/v1/path)`.
fn router_surface() -> BTreeSet<(String, String)> {
    let mut surface = BTreeSet::new();
    for (file, source) in API_SOURCES {
        for (method, path) in registrations(source, file) {
            surface.insert((method, format!("{API_PREFIX}{path}")));
        }
    }
    surface
}

/// The contract's surface, excluding the JSON endpoints outside `/api/v1`.
fn contract_surface(document: &Value) -> BTreeSet<(String, String)> {
    let mut surface = BTreeSet::new();
    for (path, item) in document["paths"].as_object().expect("paths object") {
        if OUTSIDE_THE_API.contains(&path.as_str()) {
            continue;
        }
        for (method, operation) in item.as_object().expect("path item") {
            if !METHODS.contains(&method.as_str()) {
                continue;
            }
            assert!(
                operation["operationId"].is_string(),
                "{method} {path} has no operationId"
            );
            surface.insert((method.to_uppercase(), path.clone()));
        }
    }
    surface
}

fn report(
    missing: &BTreeSet<(String, String)>,
    undocumented: &BTreeSet<(String, String)>,
) -> String {
    let mut message = String::new();
    if !missing.is_empty() {
        message.push_str("\nregistered but not documented in openapi/:\n");
        for (method, path) in missing {
            message.push_str(&format!("  {method} {path}\n"));
        }
    }
    if !undocumented.is_empty() {
        message.push_str("\ndocumented but not registered by the router:\n");
        for (method, path) in undocumented {
            message.push_str(&format!("  {method} {path}\n"));
        }
    }
    message
}

/// Modules the two router sources pull in with `merge` or `nest`.
///
/// Those routes are registered in another file. If that file is not in the read
/// set, its routes are invisible to the comparison — and if they are also
/// undocumented, the test passes when it should fail, which is the exact drift
/// it exists to catch.
fn merged_modules(source: &str) -> Vec<String> {
    let flat = without_comments(source)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut found = Vec::new();
    for call in [".merge(", ".nest("] {
        let mut rest = flat.as_str();
        while let Some(start) = rest.find(call) {
            let after = &rest[start + call.len()..];
            let end = after.find(')').unwrap_or(after.len());
            let argument = &after[..end];
            // `crate::routes::name::…` names a module file; a bare
            // `api::router()` is already inside a file in the read set.
            if let Some(index) = argument.find("crate::routes::") {
                let name: String = argument[index + "crate::routes::".len()..]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    found.push(name);
                }
            }
            rest = &after[end..];
        }
    }
    found.sort();
    found.dedup();
    found
}

#[test]
fn every_merged_router_is_read() {
    for (file, source) in API_SOURCES {
        for module in merged_modules(source) {
            assert!(
                API_SOURCES
                    .iter()
                    .any(|(path, _)| *path == format!("src/routes/{module}.rs")),
                "{file} merges `crate::routes::{module}`, whose routes this test \
                 does not read; add it to API_SOURCES or the comparison will miss them"
            );
        }
    }
}

#[test]
fn the_router_sources_do_not_nest() {
    // `nest` adds a prefix to every path inside it, and this parser does not
    // read that prefix: a nested router would produce paths that are wrong
    // rather than missing, and the comparison would report a disagreement that
    // is really a parse limitation. Failing here says so instead.
    for (file, source) in API_SOURCES {
        assert!(
            !without_comments(source).contains(".nest("),
            "{file} nests a router; teach the parser the prefix before using one"
        );
    }
}

#[test]
fn the_parser_reads_the_whole_router() {
    // A parser that stopped matching would make the comparison below vacuous.
    // Each `.route(...)` call registers exactly one path, so the number of
    // distinct paths parsed has to equal the number of calls; a parse that
    // skipped one fails here rather than silently shrinking the surface.
    // Comparing registrations instead would not work: one call can register
    // several methods.
    for (file, source) in API_SOURCES {
        // Count on the same comment-stripped text the parser reads, or a
        // commented-out call makes the two counts disagree.
        let calls = without_comments(source).matches(".route(").count();
        assert!(calls > 0, "{file}: no .route( calls found at all");
        let parsed: BTreeSet<String> = registrations(source, file)
            .into_iter()
            .map(|(_, path)| path)
            .collect();
        assert_eq!(
            parsed.len(),
            calls,
            "{file}: {calls} .route( calls but {} distinct paths parsed; \
             the parser is not reading every route",
            parsed.len()
        );
    }
    assert!(
        router_surface().len() >= 35,
        "the router surface looks too small; the parser is probably broken"
    );
}

#[test]
fn the_english_contract_matches_the_router() {
    let document: Value = serde_json::from_str(urlet::assets::OPENAPI_EN).expect("contract parses");
    let contract = contract_surface(&document);
    let router = router_surface();

    let missing: BTreeSet<_> = router.difference(&contract).cloned().collect();
    let undocumented: BTreeSet<_> = contract.difference(&router).cloned().collect();
    assert!(
        missing.is_empty() && undocumented.is_empty(),
        "the contract and the router disagree:{}",
        report(&missing, &undocumented)
    );
}

#[test]
fn the_chinese_contract_matches_the_router() {
    let document: Value =
        serde_json::from_str(urlet::assets::OPENAPI_ZH_TW).expect("contract parses");
    let contract = contract_surface(&document);
    let router = router_surface();

    let missing: BTreeSet<_> = router.difference(&contract).cloned().collect();
    let undocumented: BTreeSet<_> = contract.difference(&router).cloned().collect();
    assert!(
        missing.is_empty() && undocumented.is_empty(),
        "the Chinese contract and the router disagree:{}",
        report(&missing, &undocumented)
    );
}

#[test]
fn the_endpoints_outside_the_api_match_the_root_router() {
    // These answer JSON at the root rather than under `/api/v1`, so they are not
    // part of the API surface compared above. Checking only that the paths exist
    // would miss a method mismatch: a contract documenting `POST /health` while
    // the router registers `GET /health` would pass. Compare the methods too.
    let (file, source) = ROOT_SOURCE;
    let router: BTreeSet<(String, String)> = registrations(source, file)
        .into_iter()
        .filter(|(_, path)| OUTSIDE_THE_API.contains(&path.as_str()))
        .collect();
    assert!(
        !router.is_empty(),
        "{file}: none of {OUTSIDE_THE_API:?} were found in the root router; \
         the parser is probably broken"
    );

    for (name, spec) in [
        ("en", urlet::assets::OPENAPI_EN),
        ("zh-TW", urlet::assets::OPENAPI_ZH_TW),
    ] {
        let document: Value = serde_json::from_str(spec).expect("contract parses");
        let contract: BTreeSet<(String, String)> = document["paths"]
            .as_object()
            .expect("paths object")
            .iter()
            .filter(|(path, _)| OUTSIDE_THE_API.contains(&path.as_str()))
            .flat_map(|(path, item)| {
                item.as_object()
                    .expect("path item")
                    .iter()
                    .filter(|(method, _)| METHODS.contains(&method.as_str()))
                    .map(|(method, _)| (method.to_uppercase(), path.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();

        let missing: BTreeSet<_> = router.difference(&contract).cloned().collect();
        let undocumented: BTreeSet<_> = contract.difference(&router).cloned().collect();
        assert!(
            missing.is_empty() && undocumented.is_empty(),
            "{name}: the root endpoints and the router disagree:{}",
            report(&missing, &undocumented)
        );
    }
}
