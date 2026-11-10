mod test_support;

use pommarket::account_allocation::{
    parse_allocation_from_request, validate_new_allocation_in_tx, AllocationMode,
    StrategyAllocation,
};
use pommarket::db::{begin_as, connect, migrate};
use rust_decimal::Decimal;
use test_support::truncate_integration_tables;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn concurrent_allocation_cannot_exceed_equity() {
    let url = std::env::var("MIGRATION_DATABASE_URL")
        .or_else(|_| std::env::var("TEST_DATABASE_URL"))
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("MIGRATION_DATABASE_URL");
    migrate(&url).await.expect("migrate");
    let pool = connect(&url).await.expect("connect");
    truncate_integration_tables(&pool, &url).await;

    let user_id = Uuid::new_v4();
    let mut tx = begin_as(&pool, None, "admin").await.unwrap();
    sqlx::query(
        "INSERT INTO users (id, email, display_name, password_hash, role, status, email_verified_at)
         VALUES ($1,$2,$3,$4,'user','active',now())",
    )
    .bind(user_id)
    .bind(format!("alloc-{user_id}@test.local"))
    .bind("Alloc test")
    .bind("x")
    .execute(&mut *tx)
    .await
    .unwrap();
    let account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signature_type, signer_address, funder_address, operating_mode, status)
         VALUES ($1,'Paper','eoa','0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc','0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc','paper_trading','connected') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_risk_limits (trading_account_id, user_id, max_order_usd, max_daily_loss_usd, max_exposure_usd, max_slippage_bps, max_orders_per_interval, interval_seconds, max_stale_data_seconds, paper_cash_usd)
         VALUES ($1,$2,500,1000,5000,150,20,60,120,5000)",
    )
    .bind(account_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .unwrap();
    let trader_id: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias, monitoring_enabled)
         VALUES ($1,'0xabc','t',true) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let fixed_a =
        parse_allocation_from_request(Some("fixed_usd"), Decimal::from(3000), None).unwrap();
    let fixed_b =
        parse_allocation_from_request(Some("fixed_usd"), Decimal::from(2500), None).unwrap();

    let mut tx1 = begin_as(&pool, Some(user_id), "user").await.unwrap();
    validate_new_allocation_in_tx(&mut tx1, account_id, &fixed_a, None)
        .await
        .expect("first allocation ok");
    sqlx::query(
        "INSERT INTO copy_strategies (user_id, trading_account_id, source_trader_id, enabled, sizing_mode, strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions, max_slippage_bps, min_order_usd, copy_buy, copy_sell)
         VALUES ($1,$2,$3,false,'portfolio_ratio',3000,200,3000,10,150,1,true,true)",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(trader_id)
    .execute(&mut *tx1)
    .await
    .unwrap();
    tx1.commit().await.unwrap();

    let mut tx2 = begin_as(&pool, Some(user_id), "user").await.unwrap();
    let err = validate_new_allocation_in_tx(&mut tx2, account_id, &fixed_b, None)
        .await
        .unwrap_err();
    assert!(err.contains("exceed account equity"), "{err}");
    tx2.rollback().await.ok();

    let percent = StrategyAllocation {
        allocation_mode: AllocationMode::PercentOfAccount,
        strategy_budget_usd: Decimal::ZERO,
        allocation_percent: Some(Decimal::from(60)),
    };
    let mut tx3 = begin_as(&pool, Some(user_id), "user").await.unwrap();
    let err2 = validate_new_allocation_in_tx(&mut tx3, account_id, &percent, None)
        .await
        .unwrap_err();
    assert!(err2.contains("exceed account equity"), "{err2}");
}
