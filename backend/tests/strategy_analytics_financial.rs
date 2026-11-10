//! Deterministic strategy analytics financial scenarios (pommarket_test).

mod test_support;

use pommarket::config::Config;
use pommarket::db::{begin_as, connect, migrate};
use pommarket::strategy_analytics::compute_metrics;
use rust_decimal::Decimal;
use test_support::assert_test_database_url;
use uuid::Uuid;

async fn seed(pool: &sqlx::PgPool) -> (Uuid, Uuid, Uuid, Uuid) {
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ($1,'Fin','x','user','active',now()) RETURNING id",
    )
    .bind(format!("fin-{}@example.com", Uuid::new_v4()))
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
         VALUES ($1,$2,500,1000,5000,150,20,60,120,10000)",
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
    .bind("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .fetch_one(pool)
    .await
    .unwrap();
    let s1: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (user_id, trading_account_id, source_trader_id, enabled, sizing_mode, proportion, strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions, max_slippage_bps, min_order_usd, copy_buy, copy_sell)
         VALUES ($1,$2,$3,false,'proportional',0.1,1000,200,1000,10,150,1,true,true) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(trader)
    .fetch_one(pool)
    .await
    .unwrap();
    (user, account, trader, s1)
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn manual_position_excluded_from_strategy_metrics() {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&url);
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let (user, account, _trader, strategy) = seed(&pool).await;
    sqlx::query(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,$3,'0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','tok1','paper_trading',10,0.5,5)",
    )
    .bind(user)
    .bind(account)
    .bind(strategy)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,NULL,'0xmanual','tok-m','paper_trading',100,0.5,50)",
    )
    .bind(user)
    .bind(account)
    .execute(&pool)
    .await
    .unwrap();
    let cfg = Config::from_env().expect("config");
    let m = compute_metrics(&pool, user, strategy, &cfg).await.unwrap();
    assert_eq!(m.deployed_capital_usd, Decimal::from(5));
    assert_eq!(m.open_positions_count, 1);
    assert_eq!(m.equity.equity_quality, "partial");
    assert!(m.equity.cash_attribution_usd.is_none());
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn unrealized_uses_last_fill_mark() {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&url);
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let (user, account, _trader, strategy) = seed(&pool).await;
    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    let event: Uuid = sqlx::query_scalar(
        "INSERT INTO source_events (dedup_key, source_wallet, token_id, side, price, size_shares, notional_usd, event_ts, origin, status, raw)
         VALUES ('fin-u', '0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','tok-u','buy',0.5,10,5,now(),'test','processed','{}'::jsonb) RETURNING id",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let decision: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_decisions (user_id, trading_account_id, source_trader_id, source_event_id, strategy_id, status, reason, operating_mode, side, strategy_snapshot, checks, detected_at)
         VALUES ($1,$2,(SELECT source_trader_id FROM copy_strategies WHERE id=$3),$4,$3,'approved','ok','paper_trading','buy','{}','{}',now()) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(strategy)
    .bind(event)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let order: Uuid = sqlx::query_scalar(
        "INSERT INTO orders (user_id, trading_account_id, decision_id, source_event_id, client_order_id, operating_mode, order_type, side, token_id, limit_price, size_shares, status)
         VALUES ($1,$2,$3,$4,gen_random_uuid(),'paper_trading','limit','buy','tok-u',0.5,10,'filled') RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(decision)
    .bind(event)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO fills (user_id, trading_account_id, order_id, price, shares, notional_usd, fee_usd, fee_status)
         VALUES ($1,$2,$3,0.5,10,5,0.01,'estimated')",
    )
    .bind(user)
    .bind(account)
    .bind(order)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,$3,'0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','tok-u','paper_trading',10,0.5,5)",
    )
    .bind(user)
    .bind(account)
    .bind(strategy)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let cfg = Config::from_env().expect("config");
    let m = compute_metrics(&pool, user, strategy, &cfg).await.unwrap();
    assert_eq!(m.unrealized_pnl_usd, Decimal::ZERO);
    assert_eq!(m.positions_market_value_usd, Decimal::from(5));
    assert_eq!(m.fees_usd, Decimal::new(1, 2));
}
