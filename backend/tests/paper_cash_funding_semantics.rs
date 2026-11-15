//! Regression: account `paper_cash_usd` is total funding only; trading does not change it.

mod test_support;

use pommarket::db::{begin_as, connect, migrate};
use pommarket::strategy_accounting::{
    allocate_to_strategy, apply_account_paper_funding, apply_paper_buy_fill,
    apply_paper_resolution_settlement, apply_paper_sell_fill, reserve_order_cash,
};
use rust_decimal::Decimal;
use test_support::assert_all_test_database_urls;
use uuid::Uuid;

async fn seed(pool: &sqlx::PgPool) -> (Uuid, Uuid, Uuid) {
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ($1,'PaperSem','x','user','active',now()) RETURNING id",
    )
    .bind(format!("paper-sem-{}@example.com", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .unwrap();
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signature_type, signer_address, funder_address, operating_mode, status)
         VALUES ($1,'Acct','eoa','0x1000000000000000000000000000000000000001','0x1000000000000000000000000000000000000001','paper_trading','connected') RETURNING id",
    )
    .bind(user)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_risk_limits (trading_account_id, user_id, max_order_usd, max_daily_loss_usd, max_exposure_usd,
         max_slippage_bps, max_orders_per_interval, interval_seconds, max_stale_data_seconds, paper_cash_usd)
         VALUES ($1,$2,500,2000,5000,500,100,60,3600,0)",
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
    .bind("0x2222222222222222222222222222222222222222")
    .fetch_one(pool)
    .await
    .unwrap();
    let strategy: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (user_id, trading_account_id, source_trader_id, enabled, sizing_mode, proportion,
         strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions, max_slippage_bps, min_order_usd, copy_buy, copy_sell)
         VALUES ($1,$2,$3,true,'proportional',1,5000,500,5000,10,500,1,true,true) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(trader)
    .fetch_one(pool)
    .await
    .unwrap();
    (user, account, strategy)
}

async fn paper(pool: &sqlx::PgPool, account: Uuid) -> Decimal {
    sqlx::query_scalar("SELECT paper_cash_usd FROM account_risk_limits WHERE trading_account_id = $1")
        .bind(account)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn paper_cash_unchanged_through_fund_allocate_trade_settlement() {
    assert_all_test_database_urls();
    let url = std::env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let (user, account, strategy) = seed(&pool).await;
    let run_id = Uuid::new_v4();

    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    apply_account_paper_funding(
        &mut tx,
        user,
        account,
        Decimal::from(10_000),
        &format!("paper-sem:{run_id}:fund-10k"),
        "test",
    )
    .await
    .expect("fund");
    tx.commit().await.unwrap();
    assert_eq!(paper(&pool, account).await, Decimal::from(10_000));

    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    allocate_to_strategy(
        &mut tx,
        user,
        account,
        strategy,
        Decimal::from(5_000),
        &format!("paper-sem:{run_id}:alloc-5k"),
    )
    .await
    .expect("alloc");
    tx.commit().await.unwrap();
    assert_eq!(paper(&pool, account).await, Decimal::from(10_000));

    let order_id: Uuid = sqlx::query_scalar(
        "INSERT INTO orders (user_id, trading_account_id, client_order_id, operating_mode, order_type, side, token_id, limit_price, size_shares, status, origin)
         VALUES ($1,$2,gen_random_uuid(),'paper_trading','FAK','buy','tok1',0.5,100,'pending','copy') RETURNING id",
    )
    .bind(user)
    .bind(account)
    .fetch_one(&pool)
    .await
    .unwrap();
    let fill_id: Uuid = sqlx::query_scalar(
        "INSERT INTO fills (user_id, trading_account_id, order_id, price, shares, notional_usd, fee_usd, fee_status)
         VALUES ($1,$2,$3,0.49,100,49,1,'estimated') RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(order_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let mut tx = begin_as(&pool, None, "worker").await.unwrap();
    reserve_order_cash(
        &mut tx,
        user,
        account,
        strategy,
        order_id,
        Decimal::from(50),
        &format!("paper-sem:{run_id}:reserve"),
    )
    .await
    .unwrap();
    apply_paper_buy_fill(
        &mut tx,
        user,
        account,
        strategy,
        order_id,
        fill_id,
        Decimal::from(49),
        Decimal::from(1),
        &format!("paper-sem:{run_id}:buy"),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(paper(&pool, account).await, Decimal::from(10_000));

    let (avail, reserved, invested): (Decimal, Decimal, Decimal) = sqlx::query_as(
        "SELECT available_cash_usd, reserved_cash_usd, invested_capital_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(reserved < Decimal::from(50));
    assert!(invested >= Decimal::from(49));

    let sell_order: Uuid = sqlx::query_scalar(
        "INSERT INTO orders (user_id, trading_account_id, client_order_id, operating_mode, order_type, side, token_id, limit_price, size_shares, status, origin)
         VALUES ($1,$2,gen_random_uuid(),'paper_trading','FAK','sell','tok1',0.5,50,'filled','copy') RETURNING id",
    )
    .bind(user)
    .bind(account)
    .fetch_one(&pool)
    .await
    .unwrap();
    let sell_fill: Uuid = sqlx::query_scalar(
        "INSERT INTO fills (user_id, trading_account_id, order_id, price, shares, notional_usd, fee_usd, fee_status)
         VALUES ($1,$2,$3,0.48,50,24,0,'none') RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(sell_order)
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut tx = begin_as(&pool, None, "worker").await.unwrap();
    apply_paper_sell_fill(
        &mut tx,
        user,
        account,
        Some(strategy),
        sell_order,
        sell_fill,
        Decimal::from(24),
        Decimal::from(25),
        &format!("paper-sem:{run_id}:sell"),
        "sell_fill",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(paper(&pool, account).await, Decimal::from(10_000));

    let pos_id: Uuid = sqlx::query_scalar(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, condition_id, outcome,
         market_title, operating_mode, open_shares, avg_price, cost_basis_usd, source_observed_shares)
         VALUES ($1,$2,$3,'0x2222222222222222222222222222222222222222','tok1','0xc','Yes','T','paper_trading',0,0.5,0,0)
         RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(strategy)
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut tx = begin_as(&pool, None, "worker").await.unwrap();
    apply_paper_resolution_settlement(
        &mut tx,
        user,
        account,
        strategy,
        pos_id,
        Decimal::from(10),
        Decimal::from(5),
        &format!("paper-sem:{run_id}:resolve"),
        "resolution_redeem",
        serde_json::json!({}),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(paper(&pool, account).await, Decimal::from(10_000));

    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    apply_account_paper_funding(
        &mut tx,
        user,
        account,
        Decimal::from(1_000),
        &format!("paper-sem:{run_id}:topup"),
        "test",
    )
    .await
    .expect("topup");
    tx.commit().await.unwrap();
    assert_eq!(paper(&pool, account).await, Decimal::from(11_000));

    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    apply_account_paper_funding(
        &mut tx,
        user,
        account,
        Decimal::from(-500),
        &format!("paper-sem:{run_id}:withdraw"),
        "test",
    )
    .await
    .expect("withdraw");
    tx.commit().await.unwrap();
    assert_eq!(paper(&pool, account).await, Decimal::from(10_500));

    let _ = avail;
}