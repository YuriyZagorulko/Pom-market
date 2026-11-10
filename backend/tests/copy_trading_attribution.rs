mod test_support;

use std::env;

use async_trait::async_trait;
use chrono::Utc;
use pommarket::db::{begin_as, connect, migrate};
use pommarket::engine::{BookLevel, OrderBook};
use pommarket::pipeline::{ingest, process_event, Books, PipelineConfig};
use pommarket::polymarket::RawTrade;
use rust_decimal::Decimal;
use serde_json::json;
use test_support::truncate_integration_tables;
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

fn trade(wallet: &str, side: &str, suffix: &str, size: &str, token: &str) -> RawTrade {
    let payload = json!({
        "proxyWallet": wallet,
        "transactionHash": format!("0xattr-{suffix}"),
        "asset": token,
        "conditionId": "cond",
        "side": side,
        "price": 0.5,
        "size": size,
        "timestamp": Utc::now().timestamp(),
        "title": "Market"
    });
    pommarket::polymarket::parse_activity_payload(&payload, "test")
        .pop()
        .unwrap()
}

async fn seed_strategy(worker: &sqlx::PgPool) -> (Uuid, Uuid, Uuid, String) {
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES (concat('attr-', gen_random_uuid()::text, '@example.com'), 'Attr', 'x', 'user', 'active', now())
         RETURNING id",
    )
    .fetch_one(worker)
    .await
    .unwrap();
    let account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signer_address, funder_address, signature_type, operating_mode, status)
         VALUES ($1, 'Paper', '0xabc000000000000000000000000000000000001', '0xabc000000000000000000000000000000000001', 'eoa', 'paper_trading', 'connected') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(worker)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_risk_limits (trading_account_id, user_id, max_order_usd, max_daily_loss_usd, max_exposure_usd,
         max_slippage_bps, max_orders_per_interval, interval_seconds, max_stale_data_seconds, paper_cash_usd)
         VALUES ($1,$2,500,500,5000,500,100,60,3600,5000)",
    )
    .bind(account_id)
    .bind(user_id)
    .execute(worker)
    .await
    .unwrap();
    let wallet = "0x2222222222222222222222222222222222222222";
    let trader_id: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias, monitoring_enabled)
         VALUES ($1,$2,'Leader',true) RETURNING id",
    )
    .bind(user_id)
    .bind(wallet)
    .fetch_one(worker)
    .await
    .unwrap();
    let strategy_id: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (
            user_id, trading_account_id, source_trader_id, enabled, sizing_mode, proportion,
            strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions,
            max_slippage_bps, min_order_usd, copy_buy, copy_sell
         ) VALUES ($1,$2,$3,true,'proportional',0.1,2000,500,5000,10,500,1,true,true)
         RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(trader_id)
    .fetch_one(worker)
    .await
    .unwrap();
    (user_id, strategy_id, account_id, wallet.to_string())
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn sell_idempotency_and_manual_position_isolation() {
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

    let (user_id, strategy_id, account_id, wallet) = seed_strategy(&migration).await;
    test_support::bootstrap_paper_capital_for_account(&migration, account_id).await;
    let token = "shared-tok";

    sqlx::query(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode,
         open_shares, avg_price, cost_basis_usd, source_observed_shares)
         VALUES ($1,$2,NULL,'manual', $3, 'paper_trading', 50, 0.5, 25, 0)",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(token)
    .execute(&migration)
    .await
    .unwrap();

    let buy = trade(&wallet, "buy", "1", "1000", token);
    let buy_out = ingest(&worker, &buy).await.unwrap();
    process_event(
        &worker,
        &vault,
        &book,
        &pipeline,
        &data,
        &app_cfg,
        buy_out.event_id,
    )
    .await
    .unwrap();

    let mut tx = begin_as(&worker, Some(user_id), "user").await.unwrap();
    let manual: Decimal = sqlx::query_scalar(
        "SELECT open_shares FROM positions WHERE trading_account_id = $1 AND strategy_id IS NULL AND token_id = $2",
    )
    .bind(account_id)
    .bind(token)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let copy: Decimal = sqlx::query_scalar(
        "SELECT open_shares FROM positions WHERE strategy_id = $1 AND token_id = $2",
    )
    .bind(strategy_id)
    .bind(token)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(manual, Decimal::from(50));
    assert!(copy > Decimal::ZERO);

    let sell = trade(&wallet, "sell", "1", "500", token);
    let sell_out = ingest(&worker, &sell).await.unwrap();
    process_event(
        &worker,
        &vault,
        &book,
        &pipeline,
        &data,
        &app_cfg,
        sell_out.event_id,
    )
    .await
    .unwrap();
    process_event(
        &worker,
        &vault,
        &book,
        &pipeline,
        &data,
        &app_cfg,
        sell_out.event_id,
    )
    .await
    .unwrap();

    let manual_after: Decimal = sqlx::query_scalar(
        "SELECT open_shares FROM positions WHERE trading_account_id = $1 AND strategy_id IS NULL AND token_id = $2",
    )
    .bind(account_id)
    .bind(token)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(manual_after, Decimal::from(50));

    let sell_orders: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders o JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND o.side = 'sell'",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(sell_orders, 1);
    tx.commit().await.unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn two_strategies_same_token_isolated() {
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

    let (user_id, strategy_a, _account_a, wallet) = seed_strategy(&migration).await;

    let account_b: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signer_address, funder_address, signature_type, operating_mode, status)
         VALUES ($1, 'Paper B', '0xdef000000000000000000000000000000000001', '0xdef000000000000000000000000000000000001', 'eoa', 'paper_trading', 'connected') RETURNING id",
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
    .bind(account_b)
    .bind(user_id)
    .execute(&migration)
    .await
    .unwrap();
    let trader_id: Uuid = sqlx::query_scalar("SELECT id FROM source_traders WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&migration)
        .await
        .unwrap();
    let strategy_b: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (
            user_id, trading_account_id, source_trader_id, enabled, sizing_mode, fixed_amount_usd, proportion,
            strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions,
            max_slippage_bps, min_order_usd, copy_buy, copy_sell
         ) VALUES ($1,$2,$3,true,'fixed_amount',20,null,2000,500,5000,10,500,1,true,true)
         RETURNING id",
    )
    .bind(user_id)
    .bind(account_b)
    .bind(trader_id)
    .fetch_one(&migration)
    .await
    .unwrap();

    test_support::bootstrap_paper_capital_for_account(&migration, _account_a).await;
    test_support::bootstrap_paper_capital_for_account(&migration, account_b).await;

    let token = "iso-tok";
    let buy = trade(&wallet, "buy", "iso", "100", token);
    let out = ingest(&worker, &buy).await.unwrap();
    process_event(
        &worker,
        &vault,
        &book,
        &pipeline,
        &data,
        &app_cfg,
        out.event_id,
    )
    .await
    .unwrap();

    let mut tx = begin_as(&worker, Some(user_id), "user").await.unwrap();
    let shares_a: Decimal = sqlx::query_scalar(
        "SELECT open_shares FROM positions WHERE strategy_id = $1 AND token_id = $2",
    )
    .bind(strategy_a)
    .bind(token)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let shares_b: Decimal = sqlx::query_scalar(
        "SELECT open_shares FROM positions WHERE strategy_id = $1 AND token_id = $2",
    )
    .bind(strategy_b)
    .bind(token)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert!(shares_a > Decimal::ZERO);
    assert!(shares_b > Decimal::ZERO);

    let sell = trade(&wallet, "sell", "iso-s", "100", token);
    let sell_out = ingest(&worker, &sell).await.unwrap();
    process_event(
        &worker,
        &vault,
        &book,
        &pipeline,
        &data,
        &app_cfg,
        sell_out.event_id,
    )
    .await
    .unwrap();

    let shares_a_after: Decimal = sqlx::query_scalar(
        "SELECT open_shares FROM positions WHERE strategy_id = $1 AND token_id = $2",
    )
    .bind(strategy_a)
    .bind(token)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let shares_b_after: Decimal = sqlx::query_scalar(
        "SELECT open_shares FROM positions WHERE strategy_id = $1 AND token_id = $2",
    )
    .bind(strategy_b)
    .bind(token)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert!(
        shares_a_after <= shares_a && shares_b_after <= shares_b,
        "SELL must not increase either strategy position"
    );
    assert!(
        shares_a_after < shares_a || shares_b_after < shares_b,
        "at least one strategy should reduce on source SELL"
    );
    tx.commit().await.unwrap();
}
