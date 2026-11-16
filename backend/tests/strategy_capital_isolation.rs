//! Per-strategy capital isolation on a shared paper account (requires pommarket_test).

mod test_support;

use pommarket::db::{begin_as, connect, migrate};
use pommarket::strategy_accounting::{
    allocate_to_strategy, apply_paper_resolution_settlement, apply_paper_sell_fill,
    reconcile_account, release_order_reserve, reserve_order_cash, withdraw_from_strategy,
};
use rust_decimal::Decimal;
use test_support::{assert_test_database_url, truncate_integration_tables};
use uuid::Uuid;

async fn seed_shared_account(pool: &sqlx::PgPool) -> (Uuid, Uuid) {
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ($1,'Iso','x','user','active',now()) RETURNING id",
    )
    .bind(format!("iso-{}@example.com", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .unwrap();
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signature_type, signer_address, funder_address, operating_mode, status)
         VALUES ($1,'Shared','eoa','0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc','0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc','paper_trading','connected')
         RETURNING id",
    )
    .bind(user)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_risk_limits (trading_account_id, user_id, max_order_usd, max_daily_loss_usd, max_exposure_usd, max_slippage_bps, max_orders_per_interval, interval_seconds, max_stale_data_seconds, paper_cash_usd)
         VALUES ($1,$2,500,1000,5000,150,20,60,120,25000)",
    )
    .bind(account)
    .bind(user)
    .execute(pool)
    .await
    .unwrap();
    (user, account)
}

async fn seed_strategy(
    pool: &sqlx::PgPool,
    user: Uuid,
    account: Uuid,
    budget: i32,
    wallet_suffix: &str,
) -> Uuid {
    let trader: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias) VALUES ($1,$2,$3) RETURNING id",
    )
    .bind(user)
    .bind(format!("0x{wallet_suffix}"))
    .bind(wallet_suffix)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO copy_strategies (user_id, trading_account_id, source_trader_id, enabled, sizing_mode, allocation_mode, strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions, max_slippage_bps, min_order_usd, copy_buy, copy_sell, operating_mode)
         VALUES ($1,$2,$3,true,'proportional','fixed_usd',$4,500,5000,10,150,1,true,true,'paper_trading') RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(trader)
    .bind(Decimal::from(budget))
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn seed_order_for_strategy(
    pool: &sqlx::PgPool,
    user: Uuid,
    account: Uuid,
    strategy: Uuid,
    token: &str,
    side: &str,
) -> Uuid {
    let trader_id: Uuid = sqlx::query_scalar("SELECT source_trader_id FROM copy_strategies WHERE id = $1")
        .bind(strategy)
        .fetch_one(pool)
        .await
        .unwrap();
    let event_id: Uuid = sqlx::query_scalar(
        "INSERT INTO source_events (dedup_key, source_wallet, token_id, side, price, size_shares, notional_usd, event_ts, origin, status, raw)
         VALUES ($1,'0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',$2,$3,0.5,10,5,now(),'test','processed','{}'::jsonb)
         RETURNING id",
    )
    .bind(format!("iso-evt-{}", Uuid::new_v4()))
    .bind(token)
    .bind(side)
    .fetch_one(pool)
    .await
    .unwrap();
    let decision_id: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_decisions (user_id, trading_account_id, source_trader_id, source_event_id, strategy_id, status, reason, operating_mode, side, strategy_snapshot, checks, detected_at)
         VALUES ($1,$2,$3,$4,$5,'approved','ok','paper_trading',$6,'{}','{}',now()) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(trader_id)
    .bind(event_id)
    .bind(strategy)
    .bind(side)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO orders (user_id, trading_account_id, decision_id, source_event_id, client_order_id, operating_mode, order_type, side,
         token_id, limit_price, size_shares, status, origin)
         VALUES ($1,$2,$3,$4,gen_random_uuid(),'paper_trading','GTC',$5,$6,0.5,10,'submitted','copy')
         RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(decision_id)
    .bind(event_id)
    .bind(side)
    .bind(token)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn strategy_a_buy_does_not_reduce_strategy_b_available() {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&url);
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    truncate_integration_tables(&pool, &url).await;
    let (user, account) = seed_shared_account(&pool).await;
    let strategy_a = seed_strategy(&pool, user, account, 4000, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").await;
    let strategy_b = seed_strategy(&pool, user, account, 10000, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").await;

    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    allocate_to_strategy(&mut tx, user, account, strategy_a, Decimal::from(4000), "alloc-a")
        .await
        .unwrap();
    allocate_to_strategy(&mut tx, user, account, strategy_b, Decimal::from(10000), "alloc-b")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let order_a =
        seed_order_for_strategy(&pool, user, account, strategy_a, "token-buy", "buy").await;
    let mut tx = begin_as(&pool, None, "worker").await.unwrap();
    reserve_order_cash(
        &mut tx,
        user,
        account,
        strategy_a,
        order_a,
        Decimal::from(500),
        "reserve-a",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let avail_b: Decimal = sqlx::query_scalar(
        "SELECT available_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_b)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(avail_b, Decimal::from(10000));
    let avail_a: Decimal = sqlx::query_scalar(
        "SELECT available_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_a)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(avail_a, Decimal::from(3500));
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn strategy_a_sell_and_resolution_do_not_credit_strategy_b() {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&url);
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    truncate_integration_tables(&pool, &url).await;
    let (user, account) = seed_shared_account(&pool).await;
    let strategy_a = seed_strategy(&pool, user, account, 1000, "cccccccccccccccccccccccccccccccccccccccc").await;
    let strategy_b = seed_strategy(&pool, user, account, 2000, "dddddddddddddddddddddddddddddddddddddddd").await;

    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    allocate_to_strategy(&mut tx, user, account, strategy_a, Decimal::from(1000), "alloc-a2")
        .await
        .unwrap();
    allocate_to_strategy(&mut tx, user, account, strategy_b, Decimal::from(2000), "alloc-b2")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let position_a = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO positions (id, user_id, trading_account_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd, strategy_id)
         VALUES ($1,$2,$3,'0xcccc','token-a','paper_trading',10,0.5,5,$4)",
    )
    .bind(position_a)
    .bind(user)
    .bind(account)
    .bind(strategy_a)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE strategy_capital_accounts
         SET invested_capital_usd = invested_capital_usd + 5,
             available_cash_usd = available_cash_usd - 5
         WHERE strategy_id = $1",
    )
    .bind(strategy_a)
    .execute(&pool)
    .await
    .unwrap();

    let order =
        seed_order_for_strategy(&pool, user, account, strategy_a, "token-a", "sell").await;
    let fill = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO fills (id, user_id, trading_account_id, order_id, price, shares, notional_usd, fee_usd, fee_status)
         VALUES ($1,$2,$3,$4,0.5,10,8,0,'none')",
    )
    .bind(fill)
    .bind(user)
    .bind(account)
    .bind(order)
    .execute(&pool)
    .await
    .unwrap();
    let mut tx = begin_as(&pool, None, "worker").await.unwrap();
    apply_paper_sell_fill(
        &mut tx,
        user,
        account,
        Some(strategy_a),
        order,
        fill,
        Decimal::from(8),
        Decimal::from(5),
        "fill-sell-a",
        "sell_fill",
    )
    .await
    .unwrap();
    apply_paper_resolution_settlement(
        &mut tx,
        user,
        account,
        strategy_a,
        position_a,
        Decimal::from(2),
        Decimal::ZERO,
        "paper-resolution:other",
        "resolution_redeem",
        serde_json::json!({}),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let avail_b: Decimal = sqlx::query_scalar(
        "SELECT available_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_b)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(avail_b, Decimal::from(2000));
    let avail_a: Decimal = sqlx::query_scalar(
        "SELECT available_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_a)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        avail_a > Decimal::from(1000),
        "strategy A should have sell + redeem proceeds, got {avail_a}"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn failed_reserve_release_only_touches_owning_strategy() {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&url);
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    truncate_integration_tables(&pool, &url).await;
    let (user, account) = seed_shared_account(&pool).await;
    let strategy_a = seed_strategy(&pool, user, account, 500, "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee").await;
    let strategy_b = seed_strategy(&pool, user, account, 500, "ffffffffffffffffffffffffffffffffffffffff").await;

    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    allocate_to_strategy(&mut tx, user, account, strategy_a, Decimal::from(500), "alloc-a3")
        .await
        .unwrap();
    allocate_to_strategy(&mut tx, user, account, strategy_b, Decimal::from(500), "alloc-b3")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let order =
        seed_order_for_strategy(&pool, user, account, strategy_a, "token-res", "buy").await;
    let mut tx = begin_as(&pool, None, "worker").await.unwrap();
    reserve_order_cash(
        &mut tx,
        user,
        account,
        strategy_a,
        order,
        Decimal::from(100),
        "res",
    )
    .await
    .unwrap();
    release_order_reserve(&mut tx, user, account, strategy_a, order, Decimal::from(100), "rel")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let (avail_a, reserved_a): (Decimal, Decimal) = sqlx::query_as(
        "SELECT available_cash_usd, reserved_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_a)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(avail_a, Decimal::from(500));
    assert_eq!(reserved_a, Decimal::ZERO);
    let avail_b: Decimal = sqlx::query_scalar(
        "SELECT available_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_b)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(avail_b, Decimal::from(500));
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn explicit_withdraw_leaves_sibling_unchanged() {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&url);
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    truncate_integration_tables(&pool, &url).await;
    let (user, account) = seed_shared_account(&pool).await;
    let strategy_a = seed_strategy(&pool, user, account, 800, "1111111111111111111111111111111111111111").await;
    let strategy_b = seed_strategy(&pool, user, account, 1200, "2222222222222222222222222222222222222222").await;

    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    allocate_to_strategy(&mut tx, user, account, strategy_a, Decimal::from(800), "alloc-a4")
        .await
        .unwrap();
    allocate_to_strategy(&mut tx, user, account, strategy_b, Decimal::from(1200), "alloc-b4")
        .await
        .unwrap();
    withdraw_from_strategy(&mut tx, user, account, strategy_a, Decimal::from(200), "wd-a")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let funded_b: Decimal = sqlx::query_scalar(
        "SELECT funded_capital_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_b)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(funded_b, Decimal::from(1200));
    let report = reconcile_account(&pool, account).await.unwrap();
    assert!(report.strategies.len() >= 2);
    assert!(
        report.unallocated_cash_usd >= Decimal::from(200),
        "withdrawn cash returns to unallocated pool"
    );
}