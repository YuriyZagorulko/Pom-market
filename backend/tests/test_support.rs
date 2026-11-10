//! Shared guards for PostgreSQL integration tests.
#![allow(dead_code)]

use sqlx::PgPool;

/// Refuses URLs that do not clearly target an isolated test database.
fn database_segment(url: &str) -> &str {
    url.rsplit('/')
        .next()
        .unwrap_or(url)
        .split('?')
        .next()
        .unwrap_or(url)
}

pub fn assert_e2e_database_url(url: &str) {
    let db = database_segment(url);
    assert_eq!(
        db, "pommarket_e2e",
        "refusing E2E inject against database {db:?}; expected pommarket_e2e"
    );
}

pub fn assert_primary_database_url(url: &str) {
    let db = database_segment(url);
    assert_eq!(
        db, "pommarket",
        "refusing primary inject against database {db:?}; expected pommarket"
    );
}

pub fn assert_test_database_url(url: &str) {
    let db = database_segment(url);
    assert_eq!(
        db, "pommarket_test",
        "refusing destructive integration test against database {db:?}; expected pommarket_test"
    );
}

pub fn assert_all_test_database_urls() {
    let migration = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    let app = std::env::var("APP_DATABASE_URL").expect("APP_DATABASE_URL");
    let worker = std::env::var("WORKER_DATABASE_URL").expect("WORKER_DATABASE_URL");
    for url in [migration.as_str(), app.as_str(), worker.as_str()] {
        assert_test_database_url(url);
    }
}

pub async fn truncate_integration_tables(pool: &PgPool, migration_url: &str) {
    assert_test_database_url(migration_url);
    sqlx::query(
        "TRUNCATE users, source_events, source_event_fragments, system_events RESTART IDENTITY CASCADE",
    )
    .execute(pool)
    .await
    .expect("truncate integration tables");
}

pub async fn truncate_users(pool: &PgPool, migration_url: &str) {
    assert_test_database_url(migration_url);
    sqlx::query("TRUNCATE users RESTART IDENTITY CASCADE")
        .execute(pool)
        .await
        .expect("truncate users");
}
