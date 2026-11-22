//! The PostgreSQL backend (SPEC §4.19, §6.6, §10).
//!
//! Tests that need a PostgreSQL server read it from `URLET_TEST_DATABASE_URL`
//! and pass trivially without one, unless `URLET_REQUIRE_POSTGRES=1` (set by
//! the `PostgreSQL` CI job) turns a missing server into a failure. The
//! remaining tests run on whichever backend the suite uses, so the same
//! guarantee is checked on both.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use axum::http::StatusCode;
use common::{post_json_as, TestApp};
use serde_json::json;
use sqlx::Row;
use urlet::config::Config;
use urlet::db::{self, Pool};

/// The server under test, or `None` to skip. A required server that is
/// missing fails the test instead.
fn postgres_or_skip() -> Option<String> {
    let url = common::postgres_url();
    if url.is_none() && std::env::var("URLET_REQUIRE_POSTGRES").as_deref() == Ok("1") {
        panic!("URLET_REQUIRE_POSTGRES=1 but URLET_TEST_DATABASE_URL names no PostgreSQL server");
    }
    url
}

/// A migrated SQLite database in its own directory.
async fn sqlite_pool() -> (Pool, std::path::PathBuf) {
    let dir = common::unique_dir();
    std::fs::create_dir_all(&dir).expect("temp dir");
    let config = Config::for_tests(&dir);
    (db::init(&config).await.expect("sqlite"), dir)
}

/// One column as both backends can describe it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Column {
    name: String,
    /// `integer`, `text` or `blob`.
    kind: String,
    not_null: bool,
    /// The default expression, normalised by [`normalised_default`].
    default: Option<String>,
}

/// One single-column foreign key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ForeignKey {
    table: String,
    column: String,
    references: String,
    on_delete: String,
}

/// The parts of a schema that FR-DB-4 requires to match.
#[derive(Debug, Default, PartialEq, Eq)]
struct Schema {
    columns: BTreeMap<String, Vec<Column>>,
    indexes: BTreeSet<String>,
    foreign_keys: BTreeSet<ForeignKey>,
    checked_columns: BTreeSet<(String, String)>,
    triggers: BTreeSet<String>,
}

/// A column default in a form both backends share: PostgreSQL's casts
/// (`'active'::text`) and either side's redundant parentheses are dropped.
fn normalised_default(raw: Option<String>) -> Option<String> {
    let mut value = raw?.trim().to_string();
    while value.starts_with('(') && value.ends_with(')') {
        value = value[1..value.len() - 1].trim().to_string();
    }
    if let Some(cast) = value.rfind("::") {
        if value[cast + 2..]
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == ' ')
        {
            value.truncate(cast);
        }
    }
    while value.starts_with('(') && value.ends_with(')') {
        value = value[1..value.len() - 1].trim().to_string();
    }
    Some(value)
}

fn normalised_kind(declared: &str) -> String {
    match declared.to_ascii_lowercase().as_str() {
        "integer" | "bigint" => "integer".into(),
        "text" => "text".into(),
        "blob" | "bytea" => "blob".into(),
        other => other.into(),
    }
}

/// Column names referenced inside every `CHECK (…)` of a SQLite `CREATE
/// TABLE` statement.
fn sqlite_checked_columns(sql: &str, columns: &[Column]) -> BTreeSet<String> {
    let upper = sql.to_ascii_uppercase();
    let mut found = BTreeSet::new();
    let mut from = 0;
    while let Some(offset) = upper[from..].find("CHECK") {
        let start = from + offset;
        let Some(open) = sql[start..].find('(').map(|i| start + i) else {
            break;
        };
        let mut depth = 0;
        let mut end = open;
        for (i, c) in sql[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let body = &sql[open..=end];
        for word in body.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
            if columns.iter().any(|column| column.name == word) {
                found.insert(word.to_string());
            }
        }
        from = end + 1;
    }
    found
}

async fn sqlite_schema(pool: &Pool) -> Schema {
    let mut schema = Schema::default();
    let tables: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, sql FROM sqlite_master
          WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .expect("tables");
    for (table, sql) in tables {
        let mut columns = Vec::new();
        let rows = sqlx::query(
            "SELECT name, type, \"notnull\" AS not_null, pk, dflt_value
               FROM pragma_table_info($1)",
        )
        .bind(&table)
        .fetch_all(pool)
        .await
        .expect("columns");
        for row in rows {
            let pk: i64 = row.get("pk");
            columns.push(Column {
                name: row.get("name"),
                kind: normalised_kind(&row.get::<String, _>("type")),
                // SQLite does not report a primary key as NOT NULL; PostgreSQL
                // always makes it one.
                not_null: row.get::<i64, _>("not_null") == 1 || pk > 0,
                default: normalised_default(row.get("dflt_value")),
            });
        }
        for column in sqlite_checked_columns(&sql, &columns) {
            schema.checked_columns.insert((table.clone(), column));
        }
        let keys = sqlx::query(
            "SELECT \"from\" AS col, \"table\" AS ref, on_delete FROM pragma_foreign_key_list($1)",
        )
        .bind(&table)
        .fetch_all(pool)
        .await
        .expect("foreign keys");
        for key in keys {
            schema.foreign_keys.insert(ForeignKey {
                table: table.clone(),
                column: key.get("col"),
                references: key.get("ref"),
                on_delete: key.get::<String, _>("on_delete").to_ascii_uppercase(),
            });
        }
        columns.sort();
        schema.columns.insert(table, columns);
    }
    // Automatic indexes have no SQL; they back UNIQUE and PRIMARY KEY.
    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'index' AND sql IS NOT NULL",
    )
    .fetch_all(pool)
    .await
    .expect("indexes");
    schema.indexes.extend(indexes);
    let triggers: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'trigger'")
            .fetch_all(pool)
            .await
            .expect("triggers");
    schema.triggers.extend(triggers);
    schema
}

async fn postgres_schema(pool: &Pool) -> Schema {
    let mut schema = Schema::default();
    let rows = sqlx::query(
        "SELECT c.table_name::text AS tbl, c.column_name::text AS name, c.data_type::text AS kind,
                CASE WHEN c.is_nullable = 'NO' THEN 1 ELSE 0 END AS not_null,
                c.column_default::text AS dflt_value
           FROM information_schema.columns c
           JOIN information_schema.tables t
             ON t.table_schema = c.table_schema AND t.table_name = c.table_name
          WHERE c.table_schema = current_schema() AND t.table_type = 'BASE TABLE'",
    )
    .fetch_all(pool)
    .await
    .expect("columns");
    for row in rows {
        schema
            .columns
            .entry(row.get("tbl"))
            .or_default()
            .push(Column {
                name: row.get("name"),
                kind: normalised_kind(&row.get::<String, _>("kind")),
                not_null: row.get::<i64, _>("not_null") == 1,
                default: normalised_default(row.get("dflt_value")),
            });
    }
    for columns in schema.columns.values_mut() {
        columns.sort();
    }
    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT indexname::text FROM pg_indexes
          WHERE schemaname = current_schema()
            AND indexname NOT IN (SELECT conname FROM pg_constraint WHERE contype IN ('p', 'u'))",
    )
    .fetch_all(pool)
    .await
    .expect("indexes");
    schema.indexes.extend(indexes);
    let keys = sqlx::query(
        "SELECT cl.relname::text AS tbl, a.attname::text AS col, rcl.relname::text AS ref,
                CASE c.confdeltype WHEN 'c' THEN 'CASCADE' WHEN 'n' THEN 'SET NULL'
                                   WHEN 'r' THEN 'RESTRICT' WHEN 'd' THEN 'SET DEFAULT'
                                   ELSE 'NO ACTION' END AS on_delete
           FROM pg_constraint c
           JOIN pg_class cl ON cl.oid = c.conrelid
           JOIN pg_namespace n ON n.oid = cl.relnamespace
           JOIN pg_class rcl ON rcl.oid = c.confrelid
           JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = c.conkey[1]
          WHERE c.contype = 'f' AND n.nspname = current_schema()",
    )
    .fetch_all(pool)
    .await
    .expect("foreign keys");
    for key in keys {
        schema.foreign_keys.insert(ForeignKey {
            table: key.get("tbl"),
            column: key.get("col"),
            references: key.get("ref"),
            on_delete: key.get("on_delete"),
        });
    }
    let checks = sqlx::query(
        "SELECT cl.relname::text AS tbl, a.attname::text AS col
           FROM pg_constraint c
           JOIN pg_class cl ON cl.oid = c.conrelid
           JOIN pg_namespace n ON n.oid = cl.relnamespace
           JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = ANY (c.conkey)
          WHERE c.contype = 'c' AND n.nspname = current_schema()",
    )
    .fetch_all(pool)
    .await
    .expect("checks");
    for check in checks {
        schema
            .checked_columns
            .insert((check.get("tbl"), check.get("col")));
    }
    let triggers: Vec<String> = sqlx::query_scalar(
        "SELECT t.tgname::text FROM pg_trigger t
           JOIN pg_class cl ON cl.oid = t.tgrelid
           JOIN pg_namespace n ON n.oid = cl.relnamespace
          WHERE NOT t.tgisinternal AND n.nspname = current_schema()",
    )
    .fetch_all(pool)
    .await
    .expect("triggers");
    schema.triggers.extend(triggers);
    schema
}

/// Seed rows of `table` with every column as text, timestamps left out.
async fn seed_rows(
    pool: &Pool,
    table: &str,
    columns: &[Column],
    order: &str,
) -> Vec<Vec<Option<String>>> {
    let selected: Vec<&Column> = columns
        .iter()
        .filter(|column| !column.name.ends_with("_at"))
        .collect();
    let list = selected
        .iter()
        .map(|column| format!("CAST({} AS TEXT)", column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let rows = sqlx::query(&format!("SELECT {list} FROM {table} ORDER BY {order}"))
        .fetch_all(pool)
        .await
        .expect("seed rows");
    rows.iter()
        .map(|row| {
            (0..selected.len())
                .map(|i| row.get::<Option<String>, _>(i))
                .collect()
        })
        .collect()
}

/// `YYYY-MM-DDTHH:MM:SS.sssZ`, the §6.2 format.
fn is_iso_millis(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 24
        && bytes.iter().enumerate().all(|(i, b)| match i {
            4 | 7 => *b == b'-',
            10 => *b == b'T',
            13 | 16 => *b == b':',
            19 => *b == b'.',
            23 => *b == b'Z',
            _ => b.is_ascii_digit(),
        })
}

#[tokio::test]
async fn the_postgresql_baseline_matches_the_migrated_sqlite_schema() {
    if postgres_or_skip().is_none() {
        return;
    }
    let (sqlite, dir) = sqlite_pool().await;
    let database =
        common::ScratchDatabase(common::fresh_postgres_database().await.expect("database"));
    let mut config = Config::for_tests(&dir);
    config.database_url = database.0.clone();
    let postgres = db::init(&config).await.expect("postgres");
    let expected = sqlite_schema(&sqlite).await;
    let actual = postgres_schema(&postgres).await;
    // An introspection query that silently matched nothing would make every
    // comparison below pass.
    assert!(expected.columns.len() > 20, "{:?}", expected.columns.keys());
    assert!(expected.indexes.len() > 10 && expected.foreign_keys.len() > 10);
    assert!(expected.checked_columns.len() > 20 && !expected.triggers.is_empty());

    assert_eq!(
        expected.columns.keys().collect::<Vec<_>>(),
        actual.columns.keys().collect::<Vec<_>>(),
        "tables"
    );
    for (table, columns) in &expected.columns {
        assert_eq!(
            Some(columns),
            actual.columns.get(table),
            "columns of {table}"
        );
    }
    assert_eq!(expected.indexes, actual.indexes, "indexes");
    assert_eq!(expected.foreign_keys, actual.foreign_keys, "foreign keys");
    assert_eq!(
        expected.checked_columns, actual.checked_columns,
        "CHECK constraints"
    );
    assert_eq!(expected.triggers, actual.triggers, "triggers");

    for (table, order) in [
        ("settings", "key"),
        ("user_groups", "id"),
        ("themes", "id"),
        ("users", "id"),
        ("domains", "id"),
    ] {
        let columns = &expected.columns[table];
        assert_eq!(
            seed_rows(&sqlite, table, columns, order).await,
            seed_rows(&postgres, table, columns, order).await,
            "seed rows of {table}"
        );
    }
    let stamps: Vec<String> = sqlx::query_scalar(
        "SELECT created_at FROM user_groups UNION ALL SELECT created_at FROM themes
         UNION ALL SELECT created_at FROM users UNION ALL SELECT updated_at FROM settings",
    )
    .fetch_all(&postgres)
    .await
    .expect("timestamps");
    assert!(!stamps.is_empty());
    for stamp in stamps {
        assert!(
            is_iso_millis(&stamp),
            "{stamp} is not ISO-8601 with milliseconds"
        );
    }
    postgres.close().await;
    sqlite.close().await;
    drop(database);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn every_later_sqlite_migration_has_a_postgresql_twin() {
    // `0001_baseline` stands for SQLite 0001–0018 (FR-DB-4); from then on the
    // two lists carry the same versions in the same order.
    let baseline_covers = db::migrate::MIGRATIONS
        .iter()
        .position(|m| m.version == "0018_admin_url_lock")
        .expect("0018 is registered");
    let later: Vec<&str> = db::migrate::MIGRATIONS[baseline_covers + 1..]
        .iter()
        .map(|m| m.version)
        .collect();
    let twins: Vec<&str> = db::migrate::PG_MIGRATIONS[1..]
        .iter()
        .map(|m| m.version)
        .collect();
    assert_eq!(db::migrate::PG_MIGRATIONS[0].version, "0001_baseline");
    assert_eq!(
        later, twins,
        "SQLite and PostgreSQL migrations after the baseline"
    );
}

#[tokio::test]
async fn concurrent_postgresql_migrators_apply_the_baseline_once() {
    if postgres_or_skip().is_none() {
        return;
    }
    let database =
        common::ScratchDatabase(common::fresh_postgres_database().await.expect("database"));
    let dir = common::unique_dir();
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mut config = Config::for_tests(&dir);
    config.database_url = database.0.clone();
    let (a, b, c) = tokio::join!(db::init(&config), db::init(&config), db::init(&config));
    let pool = a.expect("first migrator");
    b.expect("second migrator");
    c.expect("third migrator");
    let applied = db::migrate::applied_count(&pool).await.expect("count");
    assert_eq!(applied, db::migrate::PG_MIGRATIONS.len() as i64);
    let groups: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_groups")
        .fetch_one(&pool)
        .await
        .expect("groups");
    assert_eq!(groups, 1, "the seed rows exist once");
    pool.close().await;
    drop(database);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn identities_continue_after_the_seed_rows() {
    let app = TestApp::spawn().await;
    let theme: i64 = sqlx::query_scalar(
        "INSERT INTO themes (key, is_enabled, is_default, sort_order, created_at, updated_at)
         VALUES ('extra', 1, 0, 99, 'x', 'x') RETURNING id",
    )
    .fetch_one(&app.state.db)
    .await
    .expect("theme");
    assert_eq!(theme, 7, "six themes are seeded");
    let group: i64 = sqlx::query_scalar(
        "INSERT INTO user_groups (name, created_at, updated_at) VALUES ('Second', 'x', 'x')
         RETURNING id",
    )
    .fetch_one(&app.state.db)
    .await
    .expect("group");
    assert_eq!(group, 2, "one group is seeded");
}

#[tokio::test]
async fn nocase_columns_are_unique_regardless_of_ascii_case() {
    let app = TestApp::spawn().await;
    let err = sqlx::query(
        "INSERT INTO user_groups (name, created_at, updated_at) VALUES ('STANDARD', 'x', 'x')",
    )
    .execute(&app.state.db)
    .await
    .expect_err("the seeded Standard group already holds the name");
    assert!(db::is_unique_violation(&err), "{err}");

    let found: Option<String> = sqlx::query_scalar("SELECT name FROM user_groups WHERE name = $1")
        .bind("sTaNdArD")
        .fetch_optional(&app.state.db)
        .await
        .expect("lookup");
    assert_eq!(found.as_deref(), Some("Standard"));
}

#[tokio::test]
async fn search_ignores_ascii_case_on_every_field() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let created = app
        .send(post_json_as(
            "/api/v1/urls",
            json!({ "target_url": "https://Example.COM/Report", "slug": "Beta-Link", "title": "Quarterly" }),
            &cookie,
        ))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    for query in ["beta", "BETA-link", "QUART", "example.com/report"] {
        let found = app
            .get_as(&format!("/api/v1/urls?q={query}"), &cookie)
            .await
            .json();
        assert_eq!(found["total"], 1, "q={query}");
    }
    let none = app.get_as("/api/v1/urls?q=beta%25", &cookie).await.json();
    assert_eq!(none["total"], 0, "% is literal");
}

async fn user_and_domain(app: &TestApp) -> (i64, i64) {
    app.register("quota@example.com", "supersecret").await;
    let user: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email_normalized = $1")
        .bind("quota@example.com")
        .fetch_one(&app.state.db)
        .await
        .expect("user");
    let domain: i64 = sqlx::query_scalar("SELECT id FROM domains WHERE is_default = 1")
        .fetch_one(&app.state.db)
        .await
        .expect("domain");
    (user, domain)
}

fn new_url(user_id: i64, domain_id: i64, slug: &str) -> db::urls::NewUrl {
    db::urls::NewUrl {
        user_id,
        domain_id,
        slug: slug.into(),
        target_url: "https://example.com/".into(),
        title: None,
        notes: None,
        password_hash: None,
        expires_at: None,
        max_clicks: None,
        redirect_type: 302,
        redirect_mode: "direct".into(),
        splash_delay_seconds: 5,
        splash_title: None,
        splash_message: None,
        is_custom_slug: true,
    }
}

#[tokio::test]
async fn concurrent_creations_at_the_quota_boundary_admit_exactly_one() {
    let app = TestApp::spawn().await;
    let (user, domain) = user_and_domain(&app).await;
    let limit = 5;
    for i in 0..limit - 1 {
        db::urls::create(&app.state.db, &new_url(user, domain, &format!("seed-{i}")))
            .await
            .expect("seed link");
    }

    let attempts = (0..8).map(|i| {
        let pool = app.state.db.clone();
        let link = new_url(user, domain, &format!("race-{i}"));
        tokio::spawn(async move { db::urls::create_within_quota(&pool, &link, limit).await })
    });
    let mut admitted = 0;
    for attempt in attempts {
        if attempt.await.expect("task").expect("insert").is_some() {
            admitted += 1;
        }
    }
    assert_eq!(admitted, 1, "only the last free slot is handed out");
    let links: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM urls WHERE user_id = $1")
        .bind(user)
        .fetch_one(&app.state.db)
        .await
        .expect("count");
    assert_eq!(links, limit);
}

#[tokio::test]
async fn a_write_transaction_waits_for_the_one_before_it() {
    let app = TestApp::spawn().await;
    let first = db::begin_write(&app.state.db).await.expect("first");
    let pool = app.state.db.clone();
    let mut second = tokio::spawn(async move { db::begin_write(&pool).await.map(|_| ()) });
    assert!(
        tokio::time::timeout(Duration::from_millis(300), &mut second)
            .await
            .is_err(),
        "a second write transaction must wait for the first"
    );
    first.commit().await.expect("commit");
    tokio::time::timeout(Duration::from_secs(5), second)
        .await
        .expect("the second one proceeds once the first commits")
        .expect("task")
        .expect("second");
}

#[tokio::test]
async fn input_postgresql_cannot_store_is_refused_alike_on_both_backends() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("nul@example.com", "supersecret").await;
    let created = app
        .send(post_json_as(
            "/api/v1/urls",
            json!({ "target_url": "https://example.com/", "slug": "beta" }),
            &cookie,
        ))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);

    // An encoded NUL byte anywhere in the URL.
    for uri in ["/beta%00", "/api/v1/urls?q=a%00b", "/api/v1/urls?tag=%00"] {
        let response = app.get_as(uri, &cookie).await;
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "{uri}: {}",
            response.body
        );
    }
    // ICU `nocase` would match full-width letters to the ASCII slug.
    let full_width = app.get("/%EF%BD%82%EF%BD%85%EF%BD%94%EF%BD%81").await;
    assert_eq!(full_width.status, StatusCode::NOT_FOUND);
    assert_eq!(app.get("/BETA").await.status, StatusCode::FOUND);

    // A NUL byte in a stored field or a lookup key from a JSON body.
    let notes = app
        .send(post_json_as(
            "/api/v1/urls",
            json!({ "target_url": "https://example.com/", "notes": "a\u{0}b" }),
            &cookie,
        ))
        .await;
    assert!(
        notes.status.is_client_error(),
        "{} {}",
        notes.status,
        notes.body
    );
    let login = app
        .post_json(
            "/api/v1/auth/login",
            json!({ "email": "nul\u{0}@example.com", "password": "supersecret" }),
        )
        .await;
    assert!(
        login.status.is_client_error(),
        "{} {}",
        login.status,
        login.body
    );
}

#[tokio::test]
async fn a_disabled_domain_cannot_become_the_default() {
    let app = TestApp::spawn().await;
    let id = app.add_domain("go.example.com", "disabled").await;
    let err = db::domains::set_default(&app.state.db, id)
        .await
        .expect_err("only an active domain can be the default");
    assert!(matches!(err, sqlx::Error::RowNotFound), "{err}");
    let default: Option<String> =
        sqlx::query_scalar("SELECT hostname FROM domains WHERE is_default = 1")
            .fetch_optional(&app.state.db)
            .await
            .expect("default");
    assert!(default.is_some(), "the previous default is kept");
}
