//! The petri test schema and the server migration set cannot drift
//! (work#1421): the shared in-memory fixture bootstrap and a file-backed
//! database migrated the way the server migrates must agree, object for
//! object and definition for definition, on their schema. When a migration
//! lands in the production set without reaching the fixture path, this
//! guard fails loudly instead of letting fold tests run against a schema
//! that lies about the production shape.

use std::collections::BTreeMap;

use fabro_db::Database;
use fabro_db::test_support::fully_migrated_in_memory_pool;

/// Every schema object of a database, keyed by (kind, name), with its
/// defining SQL. Bookkeeping tables (`_sqlx_migrations`) and SQLite
/// internals stay out: they are not the product schema.
async fn schema_objects(pool: &sqlx::SqlitePool) -> BTreeMap<(String, String), String> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT type, name, sql FROM sqlite_master \
         WHERE name NOT LIKE 'sqlite_%' \
         AND name NOT LIKE '\\_sqlx\\_migrations%' ESCAPE '\\' \
         ORDER BY type, name",
    )
    .fetch_all(pool)
    .await
    .expect("the schema reads");
    rows.into_iter()
        .map(|(kind, name, sql)| ((kind, name), sql.unwrap_or_default()))
        .collect()
}

#[tokio::test]
async fn the_petri_test_schema_matches_the_server_migration_set() {
    let fixture = fully_migrated_in_memory_pool()
        .await
        .expect("the fixture pool migrates");

    let dir = tempfile::tempdir().expect("a temp dir");
    let server = Database::connect(dir.path().join("server.sqlite3"))
        .await
        .expect("the server database connects");
    server
        .migrate()
        .await
        .expect("the server database migrates");

    let fixture_objects = schema_objects(&fixture).await;
    let server_objects = schema_objects(server.pool()).await;

    let mut drift: Vec<String> = Vec::new();
    for ((kind, name), sql) in &server_objects {
        let key = (kind.clone(), name.clone());
        match fixture_objects.get(&key) {
            Some(fixture_sql) if fixture_sql == sql => {}
            Some(_) => drift.push(format!("{kind} {name}: definition differs")),
            None => drift.push(format!("{kind} {name}: only in the server schema")),
        }
    }
    for (kind, name) in fixture_objects.keys() {
        if !server_objects.contains_key(&(kind.clone(), name.clone())) {
            drift.push(format!("{kind} {name}: only in the petri test schema"));
        }
    }
    assert!(
        drift.is_empty(),
        "the petri test schema drifted from the server migration set (work#1421): {}",
        drift.join("; ")
    );
}
