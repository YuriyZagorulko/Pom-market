//! Strategy capital ledger (requires pommarket_test + migration 0016).

mod test_support;

use pommarket::db::{begin_as, connect, migrate};
use pommarket::strategy_accounting::{
    allocate_to_strategy, bootstrap_account_strategies, ensure_strategy_capital_row,
    is_capital_bootstrapped, reconcile_account, sync_strategy_capital_to_configured_cap,
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
    let outcome = sync_strategy_capital_to_configured_cap(&mut tx, user, strategy, "test-sync")
        .await
        .expect("sync cap");
    tx.commit().await.unwrap();
    assert_eq!(outcome.funded_capital_usd, Decimal::from(200));
    assert_eq!(outcome.delta_applied_usd, Decimal::from(200));
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

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn sync_two_thousand_idempotent_and_insufficient_peer() {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&url);
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let (user, account, strategy_a) = seed_account(&pool).await;
    sqlx::query(
        "UPDATE copy_strategies SET strategy_budget_usd = 2000, allocation_mode = 'fixed_usd' WHERE id = $1",
    )
    .bind(strategy_a)
    .execute(&pool)
    .await
    .unwrap();
    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    ensure_strategy_capital_row(&mut tx, user, account, strategy_a)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    let o1 = sync_strategy_capital_to_configured_cap(&mut tx, user, strategy_a, "fund-a")
        .await
        .expect("first fund");
    tx.commit().await.unwrap();
    assert_eq!(o1.configured_capital_usd, Decimal::from(2000));
    assert_eq!(o1.funded_capital_usd, Decimal::from(2000));
    assert_eq!(o1.delta_applied_usd, Decimal::from(2000));

    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    let o2 = sync_strategy_capital_to_configured_cap(&mut tx, user, strategy_a, "fund-a-repeat")
        .await
        .expect("repeat");
    tx.commit().await.unwrap();
    assert_eq!(o2.delta_applied_usd, Decimal::ZERO);
    assert_eq!(o2.funded_capital_usd, Decimal::from(2000));

    let trader_b: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias) VALUES ($1,$2,'L2') RETURNING id",
    )
    .bind(user)
    .bind("0xcccccccccccccccccccccccccccccccccccccccc")
    .fetch_one(&pool)
    .await
    .unwrap();
    let strategy_b: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (user_id, trading_account_id, source_trader_id, enabled, sizing_mode, allocation_mode, strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions, max_slippage_bps, min_order_usd, copy_buy, copy_sell)
         VALUES ($1,$2,$3,true,'proportional','fixed_usd',3500,200,1000,10,150,1,true,true) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(trader_b)
    .fetch_one(&pool)
    .await
    .unwrap();

    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    ensure_strategy_capital_row(&mut tx, user, account, strategy_b)
        .await
        .unwrap();
    let err = sync_strategy_capital_to_configured_cap(&mut tx, user, strategy_b, "fund-b")
        .await
        .unwrap_err();
    tx.rollback().await.ok();
    assert!(
        err.to_string().contains("insufficient unallocated cash"),
        "{err}"
    );

    let ledger: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM capital_ledger_entries WHERE strategy_id = $1 AND entry_type = 'strategy_allocation'",
    )
    .bind(strategy_a)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ledger, 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn partial_fund_to_configured_cap() {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&url);
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let (user, account, strategy) = seed_account(&pool).await;
    sqlx::query(
        "UPDATE account_risk_limits SET paper_cash_usd = 1500 WHERE trading_account_id = $1",
    )
    .bind(account)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE copy_strategies SET strategy_budget_usd = 2000, allocation_mode = 'fixed_usd' WHERE id = $1",
    )
    .bind(strategy)
    .execute(&pool)
    .await
    .unwrap();
    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    ensure_strategy_capital_row(&mut tx, user, account, strategy)
        .await
        .unwrap();
    let err = sync_strategy_capital_to_configured_cap(&mut tx, user, strategy, "partial-try")
        .await
        .unwrap_err();
    tx.rollback().await.ok();
    assert!(
        err.to_string().contains("insufficient unallocated cash"),
        "{err}"
    );

    sqlx::query(
        "UPDATE copy_strategies SET strategy_budget_usd = 1200 WHERE id = $1",
    )
    .bind(strategy)
    .execute(&pool)
    .await
    .unwrap();
    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    let o = sync_strategy_capital_to_configured_cap(&mut tx, user, strategy, "partial-ok")
        .await
        .expect("fits");
    tx.commit().await.unwrap();
    assert_eq!(o.funded_capital_usd, Decimal::from(1200));
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn concurrent_sync_two_strategies_respects_unallocated() {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&url);
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let (user, account, strategy_a) = seed_account(&pool).await;
    sqlx::query(
        "UPDATE account_risk_limits SET paper_cash_usd = 5000 WHERE trading_account_id = $1",
    )
    .bind(account)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE copy_strategies SET strategy_budget_usd = 2000, allocation_mode = 'fixed_usd' WHERE id = $1",
    )
    .bind(strategy_a)
    .execute(&pool)
    .await
    .unwrap();
    let trader_b: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias) VALUES ($1,$2,'L2') RETURNING id",
    )
    .bind(user)
    .bind("0xdddddddddddddddddddddddddddddddddddddddd")
    .fetch_one(&pool)
    .await
    .unwrap();
    let strategy_b: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (user_id, trading_account_id, source_trader_id, enabled, sizing_mode, allocation_mode, strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions, max_slippage_bps, min_order_usd, copy_buy, copy_sell)
         VALUES ($1,$2,$3,true,'proportional','fixed_usd',2000,200,1000,10,150,1,true,true) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(trader_b)
    .fetch_one(&pool)
    .await
    .unwrap();

    let pool_a = pool.clone();
    let pool_b = pool.clone();
    let (r1, r2) = tokio::join!(
        async {
            let mut tx = begin_as(&pool_a, Some(user), "user").await.unwrap();
            ensure_strategy_capital_row(&mut tx, user, account, strategy_a)
                .await
                .unwrap();
            let out = sync_strategy_capital_to_configured_cap(&mut tx, user, strategy_a, "conc-a")
                .await
                .map_err(|e| e.to_string())?;
            tx.commit().await.map_err(|e| e.to_string())?;
            Ok::<_, String>(out)
        },
        async {
            let mut tx = begin_as(&pool_b, Some(user), "user").await.unwrap();
            ensure_strategy_capital_row(&mut tx, user, account, strategy_b)
                .await
                .unwrap();
            let out = sync_strategy_capital_to_configured_cap(&mut tx, user, strategy_b, "conc-b")
                .await
                .map_err(|e| e.to_string())?;
            tx.commit().await.map_err(|e| e.to_string())?;
            Ok::<_, String>(out)
        },
    );
    assert!(r1.is_ok(), "{r1:?}");
    assert!(r2.is_ok(), "{r2:?}");
    let total: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(funded_capital_usd),0) FROM strategy_capital_accounts WHERE trading_account_id = $1",
    )
    .bind(account)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(total, Decimal::from(4000));
}
