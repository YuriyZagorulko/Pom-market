//! Shared guards for PostgreSQL integration tests.
#![allow(dead_code)]

use serde_json::Value;
use sqlx::PgPool;

/// List endpoints return either a bare JSON array (legacy) or `{ items, total, ... }`.
pub fn list_items(body: &Value) -> &[Value] {
    if let Some(arr) = body.as_array() {
        return arr;
    }
    body.get("items")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .expect("expected JSON array or paged object with items[]")
}

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

/// Initialize strategy capital rows for integration copy tests (does not auto-fund the main DB in prod).
pub async fn bootstrap_paper_capital_for_account(pool: &PgPool, trading_account_id: uuid::Uuid) {
    pommarket::strategy_accounting::bootstrap_account_strategies(pool, trading_account_id, true)
        .await
        .expect("bootstrap paper capital for test account");
}

pub fn pipeline_runtime() -> (
    pommarket::polymarket::http::PublicClient,
    pommarket::config::Config,
) {
    let cfg = pommarket::config::Config::from_env().expect("config");
    let data = pommarket::polymarket::http::PublicClient::new(
        cfg.data_api_host.clone(),
        cfg.clob_host.clone(),
        cfg.gamma_host.clone(),
    );
    (data, cfg)
}

pub async fn truncate_users(pool: &PgPool, migration_url: &str) {
    assert_test_database_url(migration_url);
    sqlx::query("TRUNCATE users RESTART IDENTITY CASCADE")
        .execute(pool)
        .await
        .expect("truncate users");
}
