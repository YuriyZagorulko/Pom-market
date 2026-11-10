mod test_support;

use std::env;

use async_trait::async_trait;
use chrono::Utc;
use pommarket::db::{begin_as, connect, migrate};
use pommarket::engine::{BookLevel, OrderBook};
use pommarket::pipeline::{ingest, process_event, Books, PipelineConfig};
use pommarket::polymarket::RawTrade;
use pommarket::strategy_budget::utilization_for_user;
use rust_decimal::Decimal;
use serde_json::json;
use test_support::truncate_integration_tables;
use tokio::try_join;
use uuid::Uuid;

struct FixedBook;

#[async_trait]
impl Books for FixedBook {
    async fn book(&self, _: &str) -> Result<OrderBook, String> {
        Ok(OrderBook {
            bids: vec![BookLevel {
                price: Decimal::new(50, 2),
                size: Decimal::from(10_000),
            }],
            asks: vec![BookLevel {
                price: Decimal::new(50, 2),
                size: Decimal::from(10_000),
            }],
            tick: Decimal::new(1, 2),
            min_order_size: Decimal::ZERO,
        })
    }

    async fn fee(&self, _: &str) -> Option<pommarket::engine::FeeSchedule> {
        Some(pommarket::engine::FeeSchedule {
            fees_enabled: false,
            rate: Decimal::ZERO,
            exponent: 1,
            taker_only: true,
            rebate_rate: None,
            source: "test".into(),
        })
    }
}

fn trade(wallet: &str, side: &str, suffix: &str, notional_shares: &str) -> RawTrade {
    let payload = json!({
        "proxyWallet": wallet,
        "transactionHash": format!("0xconc-{suffix}"),
        "asset": format!("tok-{suffix}"),
        "conditionId": "cond",
        "side": side,
        "price": 0.5,
        "size": notional_shares,
        "timestamp": Utc::now().timestamp(),
        "title": "Market"
    });
    pommarket::polymarket::parse_activity_payload(&payload, "test")
        .pop()
        .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn concurrent_buys_do_not_overspend_strategy_budget() {
    test_support::assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    pommarket::ensure_login_roles(
        &migration,
        &env::var("APP_DB_PASSWORD").unwrap(),
        &env::var("WORKER_DB_PASSWORD").unwrap(),
    )
    .await
    .unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    pommarket::bootstrap_admin(&migration, &pommarket::config::Config::from_env().unwrap())
        .await
        .unwrap();

    let cfg = pommarket::config::Config::from_env().unwrap();
    let worker = connect(&cfg.worker_database_url).await.unwrap();
    let vault = pommarket::security::Vault::new(
        &cfg.encryption_master_key,
        &cfg.encryption_key_id,
        None,
        None,
    )
    .unwrap();
    let pipeline = PipelineConfig {
        clob_host: cfg.clob_host.clone(),
        confirm_live: false,
        default_fee_rate: Decimal::ZERO,
        default_fee_exponent: 1,
    };
    let book = FixedBook;
    let (data, app_cfg) = test_support::pipeline_runtime();
    let wallet = "0x1111111111111111111111111111111111111111";

    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ('concurrency@example.com', 'Concurrency', 'x', 'user', 'active', now()) RETURNING id",
    )
    .fetch_one(&migration)
    .await
    .unwrap();

    let account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signer_address, funder_address, signature_type, operating_mode, status)
         VALUES ($1, 'Paper', '0xabc000000000000000000000000000000000001', '0xabc000000000000000000000000000000000001', 'eoa', 'paper_trading', 'connected') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_risk_limits (trading_account_id, user_id, max_order_usd, max_daily_loss_usd, max_exposure_usd,
         max_slippage_bps, max_orders_per_interval, interval_seconds, max_stale_data_seconds, paper_cash_usd)
         VALUES ($1,$2,500,500,5000,500,100,60,3600,5000)",
    )
    .bind(account_id)
    .bind(user_id)
    .execute(&migration)
    .await
    .unwrap();

    let trader_id: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias, monitoring_enabled)
         VALUES ($1,$2,'Leader',true) RETURNING id",
    )
    .bind(user_id)
    .bind(wallet)
    .fetch_one(&migration)
    .await
    .unwrap();

    let strategy_id: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (
            user_id, trading_account_id, source_trader_id, enabled, sizing_mode, proportion,
            strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions,
            max_slippage_bps, min_order_usd, copy_buy, copy_sell
         ) VALUES ($1,$2,$3,true,'proportional',0.5,$4,500,5000,10,500,1,true,true)
         RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(trader_id)
    .bind(Decimal::from(150))
    .fetch_one(&migration)
    .await
    .unwrap();

    test_support::bootstrap_paper_capital_for_account(&worker, account_id).await;

    // Pre-fill deployed capital so only ~$0–50 remains for new BUY reserves (budget 150, deployed 130).
    sqlx::query(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode,
         open_shares, avg_price, cost_basis_usd, source_observed_shares)
         VALUES ($1,$2,$3,$4,'preseed','paper_trading',10,0.5,130,10)",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(strategy_id)
    .bind(wallet)
    .execute(&migration)
    .await
    .unwrap();

    let b1 = trade(wallet, "buy", "a", "2000");
    let b2 = trade(wallet, "buy", "b", "2000");
    let o1 = ingest(&worker, &b1).await.unwrap();
    let o2 = ingest(&worker, &b2).await.unwrap();

    try_join!(
        process_event(
            &worker,
            &vault,
            &book,
            &pipeline,
            &data,
            &app_cfg,
            o1.event_id
        ),
        process_event(
            &worker,
            &vault,
            &book,
            &pipeline,
            &data,
            &app_cfg,
            o2.event_id
        )
    )
    .unwrap();

    let util = utilization_for_user(&worker, user_id, strategy_id, &app_cfg)
        .await
        .unwrap();
    assert!(
        util.deployed_capital_usd + util.reserved_capital_usd <= Decimal::from(150),
        "exposure {:?} + {:?} exceeds budget",
        util.deployed_capital_usd,
        util.reserved_capital_usd
    );

    let approved: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM copy_decisions WHERE strategy_id = $1 AND status = 'approved'",
    )
    .bind(strategy_id)
    .fetch_one(&worker)
    .await
    .unwrap();
    assert!(approved <= 1, "at most one BUY should fit remaining budget");

    let mut tx = begin_as(&worker, Some(user_id), "user").await.unwrap();
    let reserved_sum: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(o.position_reserved_usd + o.fee_reserved_usd), 0)
         FROM orders o JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND o.status IN ('pending','submitting','submitted','partially_filled','unknown','filled')",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let _ = reserved_sum;
}
