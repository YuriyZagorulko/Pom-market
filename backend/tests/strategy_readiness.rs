//! Strategy readiness endpoint.

mod test_support;

use pommarket::db::{begin_as, connect, migrate};
use pommarket::strategy_readiness::ReadinessCode;
use test_support::assert_test_database_url;
use uuid::Uuid;

async fn seed(pool: &sqlx::PgPool) -> (Uuid, Uuid) {
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ($1,'R','x','user','active',now()) RETURNING id",
    )
    .bind(format!("ready-{}@example.com", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .unwrap();
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signature_type, signer_address, funder_address, operating_mode, status)
         VALUES ($1,'A','eoa','0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc','0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc','paper_trading','connected')
         RETURNING id",
    )
    .bind(user)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_risk_limits (trading_account_id, user_id, max_order_usd, max_daily_loss_usd, max_exposure_usd, max_slippage_bps, max_orders_per_interval, interval_seconds, max_stale_data_seconds, paper_cash_usd)
         VALUES ($1,$2,500,1000,5000,150,20,60,120,1000)",
    )
    .bind(account)
    .bind(user)
    .execute(pool)
    .await
    .unwrap();
    let trader: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias, monitoring_enabled) VALUES ($1,$2,'L',true) RETURNING id",
    )
    .bind(user)
    .bind("0xcccccccccccccccccccccccccccccccccccccccc")
    .fetch_one(pool)
    .await
    .unwrap();
    let strategy: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (user_id, trading_account_id, source_trader_id, enabled, sizing_mode, fixed_amount_usd, strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions, max_slippage_bps, min_order_usd, copy_buy, copy_sell)
         VALUES ($1,$2,$3,true,'fixed_amount',10,200,200,1000,10,150,1,true,true) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(trader)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO strategy_capital_accounts (strategy_id, user_id, trading_account_id, funded_capital_usd, available_cash_usd, bootstrap_quality, initialized_at)
         VALUES ($1,$2,$3,200,200,'migrated',now())",
    )
    .bind(strategy)
    .bind(user)
    .bind(account)
    .execute(pool)
    .await
    .unwrap();
    (user, strategy)
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn readiness_fixed_amount_when_funded() {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&url);
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let (user, strategy) = seed(&pool).await;
    let cfg = pommarket::config::Config::from_env().unwrap();
    let r = pommarket::strategy_readiness::readiness_for_user(&pool, user, strategy, &cfg)
        .await
        .unwrap();
    assert_eq!(r.code, ReadinessCode::ReadyToTrade);
    assert!(r.ready);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn readiness_capital_not_funded_when_pending() {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&url);
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let (user, strategy) = seed(&pool).await;
    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    sqlx::query(
        "UPDATE strategy_capital_accounts SET funded_capital_usd=0, available_cash_usd=0, bootstrap_quality='pending' WHERE strategy_id=$1",
    )
    .bind(strategy)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let cfg = pommarket::config::Config::from_env().unwrap();
    let r = pommarket::strategy_readiness::readiness_for_user(&pool, user, strategy, &cfg)
        .await
        .unwrap();
    assert_eq!(r.code, ReadinessCode::CapitalNotFunded);
    assert!(!r.ready);
}
