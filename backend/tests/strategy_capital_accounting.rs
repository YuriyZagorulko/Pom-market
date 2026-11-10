//! Strategy capital ledger (requires pommarket_test + migration 0016).

mod test_support;

use pommarket::db::{begin_as, connect, migrate};
use pommarket::strategy_accounting::{
    allocate_to_strategy, bootstrap_account_strategies, is_capital_bootstrapped, reconcile_account,
    sync_strategy_capital_to_configured_cap,
};
use rust_decimal::Decimal;
use test_support::assert_test_database_url;
use uuid::Uuid;

async fn seed_account(pool: &sqlx::PgPool) -> (Uuid, Uuid, Uuid) {
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ($1,'Cap','x','user','active',now()) RETURNING id",
    )
    .bind(format!("cap-{}@example.com", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .unwrap();
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signature_type, signer_address, funder_address, operating_mode, status)
         VALUES ($1,'CapAcct','eoa','0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc','0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc','paper_trading','connected')
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
        "INSERT INTO source_traders (user_id, wallet_address, alias) VALUES ($1,$2,'L') RETURNING id",
    )
    .bind(user)
    .bind("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    .fetch_one(pool)
    .await
    .unwrap();
    let strategy: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (user_id, trading_account_id, source_trader_id, enabled, sizing_mode, proportion, strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions, max_slippage_bps, min_order_usd, copy_buy, copy_sell)
         VALUES ($1,$2,$3,false,'proportional',0.1,200,200,1000,10,150,1,true,true) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(trader)
    .fetch_one(pool)
    .await
    .unwrap();
    (user, account, strategy)
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn allocate_fund_and_reconcile() {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&url);
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let (user, account, strategy) = seed_account(&pool).await;
    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    allocate_to_strategy(
        &mut tx,
        user,
        account,
        strategy,
        Decimal::from(200),
        "test-alloc-1",
    )
    .await
    .expect("allocate");
    tx.commit().await.unwrap();
    let report = reconcile_account(&pool, account).await.unwrap();
    assert!(report.unallocated_cash_usd >= Decimal::from(800));
    bootstrap_account_strategies(&pool, account, true)
        .await
        .unwrap();
    let quality: String = sqlx::query_scalar(
        "SELECT bootstrap_quality FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_ne!(quality, "pending");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn sync_on_enable_target_allocates_and_bootstraps() {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&url);
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let (user, _account, strategy) = seed_account(&pool).await;
    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    sync_strategy_capital_to_configured_cap(&mut tx, user, strategy, "test-sync")
        .await
        .expect("sync cap");
    tx.commit().await.unwrap();
    let funded: Decimal = sqlx::query_scalar(
        "SELECT funded_capital_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(funded, Decimal::from(200));
    let mut tx = begin_as(&pool, None, "worker").await.unwrap();
    assert!(is_capital_bootstrapped(&mut tx, strategy).await.unwrap());
    tx.commit().await.unwrap();
}
