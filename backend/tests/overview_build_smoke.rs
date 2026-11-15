//! Smoke: build_overview does not panic when capital row is absent.

mod common;
#[path = "helpers/strategy_delete_harness.rs"]
mod strategy_delete_harness;
mod test_support;

use common::TestApi;
use pommarket::config::Config;
use pommarket::db::{begin_as, connect};
use pommarket::strategy_analytics::build_overview;
use test_support::assert_test_database_url;
use uuid::Uuid;

use strategy_delete_harness::seed_user_strategy;

#[tokio::test]
#[ignore = "set OVERVIEW_SMOKE_STRATEGY_ID and use pommarket or pommarket_test"]
async fn build_overview_smoke() {
    let url = std::env::var("APP_DATABASE_URL").expect("APP_DATABASE_URL");
    if std::env::var("OVERVIEW_SMOKE_ALLOW_DEV").ok().as_deref() != Some("1") {
        return;
    }
    if url.contains("pommarket_test") {
        return;
    }
    let strategy_id = Uuid::parse_str(
        &std::env::var("OVERVIEW_SMOKE_STRATEGY_ID").expect("OVERVIEW_SMOKE_STRATEGY_ID"),
    )
    .unwrap();
    let user_id = Uuid::parse_str(
        &std::env::var("OVERVIEW_SMOKE_USER_ID").expect("OVERVIEW_SMOKE_USER_ID"),
    )
    .unwrap();
    let pool = connect(&url).await.unwrap();
    let cfg = Config::from_env().expect("config");
    build_overview(&pool, user_id, strategy_id, &cfg)
        .await
        .expect("build_overview should succeed");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn overview_without_capital_row_still_builds() {
    let url = std::env::var("APP_DATABASE_URL").expect("APP_DATABASE_URL");
    assert_test_database_url(&url);
    let api = TestApi::boot().await;
    let email = format!("overview-cap-{}@example.com", Uuid::new_v4());
    let (token, user_id, account, _, strategy) = seed_user_strategy(&api, &email).await;
    let _ = token;
    let _ = account;

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    sqlx::query("DELETE FROM strategy_capital_accounts WHERE strategy_id = $1")
        .bind(strategy)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let cfg = Config::from_env().expect("config");
    build_overview(&api.worker, user_id, strategy, &cfg)
        .await
        .expect("overview must not fail when capital row is missing");
}
