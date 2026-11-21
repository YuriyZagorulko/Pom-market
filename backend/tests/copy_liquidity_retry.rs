//! Liquidity retry and book rejection classification (pommarket_test).

mod test_support;

use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use chrono::Utc;
use pommarket::db::{connect, migrate};
use pommarket::engine::{walk_book, BookLevel, OrderBook, RejectReason};
use pommarket::pipeline::{ingest, process_event, Books, PipelineConfig};
use pommarket::polymarket::RawTrade;
use rust_decimal::Decimal;
use serde_json::json;
use test_support::{assert_all_test_database_urls, bootstrap_paper_capital_for_account, truncate_integration_tables};
use uuid::Uuid;

struct RetryBook {
    calls: AtomicU32,
}

#[async_trait]
impl Books for RetryBook {
    async fn book(&self, _token_id: &str) -> Result<OrderBook, String> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            Ok(OrderBook {
                bids: vec![],
                asks: vec![],
                tick: Decimal::new(1, 2),
                min_order_size: Decimal::ZERO,
            })
        } else {
            Ok(OrderBook {
                bids: vec![],
                asks: vec![BookLevel {
                    price: Decimal::new(50, 2),
                    size: Decimal::from(10_000),
                }],
                tick: Decimal::new(1, 2),
                min_order_size: Decimal::ZERO,
            })
        }
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

#[test]
fn slippage_when_best_ask_above_limit() {
    let book = OrderBook {
        bids: vec![],
        asks: vec![BookLevel {
            price: Decimal::new(60, 2),
            size: Decimal::from(100),
        }],
        tick: Decimal::new(1, 2),
        min_order_size: Decimal::ZERO,
    }
    .normalized();
    let err = walk_book(
        &book,
        true,
        Decimal::new(50, 2),
        Decimal::from(10),
        None,
        true,
    )
    .unwrap_err();
    assert_eq!(err, RejectReason::Slippage);
}

#[test]
fn insufficient_depth_when_partial_liquidity() {
    let book = OrderBook {
        bids: vec![],
        asks: vec![BookLevel {
            price: Decimal::new(50, 2),
            size: Decimal::from(2),
        }],
        tick: Decimal::new(1, 2),
        min_order_size: Decimal::ZERO,
    }
    .normalized();
    let err = walk_book(
        &book,
        true,
        Decimal::new(55, 2),
        Decimal::from(10),
        None,
        true,
    )
    .unwrap_err();
    assert_eq!(err, RejectReason::InsufficientDepth);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn retry_eventually_fills_when_book_appears() {
    assert_all_test_database_urls();
    let migration_url = std::env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    pommarket::bootstrap_admin(&migration, &pommarket::config::Config::from_env().unwrap())
        .await
        .unwrap();
    let cfg = pommarket::config::Config::from_env().unwrap();
    let pool = connect(&cfg.worker_database_url).await.unwrap();
    let book = RetryBook {
        calls: AtomicU32::new(0),
    };
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
        default_fee_rate: cfg.fee_rate,
        default_fee_exponent: cfg.fee_exponent,
        paper_allow_synthetic_fills: false,
    };
    let data = pommarket::polymarket::http::PublicClient::new(
        cfg.data_api_host.clone(),
        cfg.clob_host.clone(),
        cfg.gamma_host.clone(),
    );
    let wallet = format!("0xretry{}", Uuid::new_v4().simple());
    let user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE email = 'admin@example.com'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, operating_mode, account_type, status)
         VALUES ($1, 'Retry acct', 'paper_trading', 'paper', 'active') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    bootstrap_paper_capital_for_account(&pool, account_id).await;
    let trader_id: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, monitoring_enabled)
         VALUES ($1, $2, true) RETURNING id",
    )
    .bind(user_id)
    .bind(&wallet)
    .fetch_one(&pool)
    .await
    .unwrap();
    let strategy_id: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (
            user_id, trading_account_id, source_trader_id, enabled, sizing_mode, allocation_mode,
            strategy_budget_usd, max_slippage_bps, min_order_usd, copy_buy, copy_sell, operating_mode
         ) VALUES ($1,$2,$3,true,'fixed_amount','fixed_usd',2000,150,1,true,false,'paper_trading') RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(trader_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE copy_strategies SET fixed_amount_usd = 25, enabled = true WHERE id = $1")
        .bind(strategy_id)
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pommarket::db::begin_as(&pool, None, "worker").await.unwrap();
    pommarket::strategy_accounting::ensure_strategy_capital_row(&mut tx, user_id, account_id, strategy_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let trade = trade_payload(&wallet, "buy", "tok-retry", &format!("tx-retry-{}", Uuid::new_v4()));
    let out = ingest(&pool, &trade).await.unwrap();
    process_event(
        &pool,
        &vault,
        &book,
        &pipeline,
        &data,
        &cfg,
        out.event_id,
    )
    .await
    .unwrap();

    let status: String = sqlx::query_scalar(
        "SELECT status FROM copy_decisions WHERE strategy_id = $1 ORDER BY decided_at DESC LIMIT 1",
    )
    .bind(strategy_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "awaiting_liquidity");

    pommarket::liquidity_retry::process_liquidity_retries(
        &pool,
        &vault,
        &book,
        &pipeline,
        &data,
        &cfg,
    )
    .await
    .unwrap();

    let status2: String = sqlx::query_scalar(
        "SELECT status FROM copy_decisions WHERE strategy_id = $1 ORDER BY decided_at DESC LIMIT 1",
    )
    .bind(strategy_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status2, "approved");
    let orders: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders WHERE decision_id IN (SELECT id FROM copy_decisions WHERE strategy_id = $1)",
    )
    .bind(strategy_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(orders, 1);
}

fn trade_payload(wallet: &str, side: &str, token: &str, tx: &str) -> RawTrade {
    let payload = json!({
        "proxyWallet": wallet,
        "transactionHash": tx,
        "asset": token,
        "conditionId": "cond",
        "side": side,
        "price": 0.5,
        "size": "100",
        "timestamp": Utc::now().timestamp(),
    });
    pommarket::polymarket::parse_activity_payload(&payload, "test")
        .pop()
        .unwrap()
}