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
use uuid::Uuid;

struct FixedBook;

#[async_trait]
impl Books for FixedBook {
    async fn book(&self, _token_id: &str) -> Result<OrderBook, String> {
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

    async fn fee(&self, _token_id: &str) -> Option<pommarket::engine::FeeSchedule> {
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

fn trade(side: &str, notional_shares: &str, wallet: &str, token: &str) -> RawTrade {
    let payload = json!({
        "proxyWallet": wallet,
        "transactionHash": format!("0x{}-{}", side, token),
        "asset": token,
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
async fn proportional_budget_utilization_and_attribution() {
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

    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ('copy@example.com', 'Copy', 'x', 'user', 'active', now()) RETURNING id",
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
         VALUES ($1,$2,200,500,2000,500,100,60,3600,5000)",
    )
    .bind(account_id)
    .bind(user_id)
    .execute(&migration)
    .await
    .unwrap();

    let trader_id: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias, monitoring_enabled)
         VALUES ($1,'0x1111111111111111111111111111111111111111','Leader',true) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&migration)
    .await
    .unwrap();

    let strategy_id: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (
            user_id, trading_account_id, source_trader_id, enabled, sizing_mode, proportion,
            strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions,
            max_slippage_bps, min_order_usd, copy_buy, copy_sell
         ) VALUES ($1,$2,$3,true,'proportional',0.1,$4,200,2000,10,500,1,true,true)
         RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(trader_id)
    .bind(Decimal::from(2000))
    .fetch_one(&migration)
    .await
    .unwrap();

    let wallet = "0x1111111111111111111111111111111111111111";
    let buy1 = trade("buy", "2000", wallet, "tok-a");
    let out1 = ingest(&worker, &buy1).await.unwrap();
    process_event(&worker, &vault, &book, &pipeline, out1.event_id)
        .await
        .unwrap();

    let util = utilization_for_user(&worker, user_id, strategy_id)
        .await
        .unwrap();
    assert!(util.deployed_capital_usd > Decimal::ZERO);
    assert!(util.deployed_capital_usd <= Decimal::from(100));
    assert!(util.available_budget_usd >= Decimal::from(1900));

    let mut user_tx = begin_as(&worker, Some(user_id), "user").await.unwrap();
    let strategy_pos: Decimal =
        sqlx::query_scalar("SELECT open_shares FROM positions WHERE strategy_id = $1")
            .bind(strategy_id)
            .fetch_one(&mut *user_tx)
            .await
            .unwrap();
    assert!(strategy_pos > Decimal::ZERO);
    user_tx.commit().await.unwrap();

    let sell = trade("sell", "1000", wallet, "tok-a");
    let sell_out = ingest(&worker, &sell).await.unwrap();
    process_event(&worker, &vault, &book, &pipeline, sell_out.event_id)
        .await
        .unwrap();

    let util2 = utilization_for_user(&worker, user_id, strategy_id)
        .await
        .unwrap();
    assert!(
        util2.deployed_capital_usd < util.deployed_capital_usd,
        "partial SELL should release strategy cost basis"
    );

    let mut user_tx2 = begin_as(&worker, Some(user_id), "user").await.unwrap();
    let attributed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM positions WHERE strategy_id = $1 AND token_id = 'tok-a')",
    )
    .bind(strategy_id)
    .fetch_one(&mut *user_tx2)
    .await
    .unwrap();
    assert!(attributed);
}
