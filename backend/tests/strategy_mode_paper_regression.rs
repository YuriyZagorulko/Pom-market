//! Paper execution regression after per-strategy operating_mode (pommarket_test).

mod test_support;

use async_trait::async_trait;
use chrono::Utc;
use pommarket::db::{begin_as, connect, migrate};
use pommarket::strategy_accounting::{allocate_to_strategy, ensure_strategy_capital_row};
use pommarket::engine::{BookLevel, OrderBook};
use pommarket::pipeline::{ingest, process_event, Books, PipelineConfig};
use pommarket::polymarket::RawTrade;
use pommarket::strategy_operating_mode::LIVE_TRADING_NOT_READY_CODE;
use rust_decimal::Decimal;
use serde_json::json;
use test_support::{assert_all_test_database_urls, bootstrap_paper_capital_for_account, truncate_integration_tables};
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

fn trade(side: &str, size: &str, wallet: &str, token: &str, tx: &str) -> RawTrade {
    let payload = json!({
        "proxyWallet": wallet,
        "transactionHash": tx,
        "asset": token,
        "conditionId": "cond",
        "side": side,
        "price": 0.5,
        "size": size,
        "timestamp": Utc::now().timestamp(),
        "title": "Mode regression"
    });
    pommarket::polymarket::parse_activity_payload(&payload, "test")
        .pop()
        .unwrap()
}

struct Harness {
    migration: sqlx::PgPool,
    worker: sqlx::PgPool,
    vault: pommarket::security::Vault,
    pipeline: PipelineConfig,
    book: FixedBook,
    data: pommarket::polymarket::http::PublicClient,
    app_cfg: pommarket::config::Config,
    user_id: Uuid,
    account_id: Uuid,
    trader_id: Uuid,
    strategy_paper: Uuid,
    strategy_paper_b: Uuid,
    wallet: &'static str,
}

async fn setup_harness(account_mode: &str) -> Harness {
    let migration_url = std::env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    pommarket::ensure_login_roles(
        &migration,
        &std::env::var("APP_DB_PASSWORD").unwrap(),
        &std::env::var("WORKER_DB_PASSWORD").unwrap(),
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
        paper_allow_synthetic_fills: false,
    };
    let (data, app_cfg) = test_support::pipeline_runtime();

    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ('mode-reg@example.com', 'Reg', 'x', 'user', 'active', now()) RETURNING id",
    )
    .fetch_one(&migration)
    .await
    .unwrap();

    let account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signer_address, funder_address, signature_type, operating_mode, status, live_confirmed_at)
         VALUES ($1, 'Acct', '0xabc000000000000000000000000000000000001', '0xabc000000000000000000000000000000000001', 'eoa', $2, 'connected',
           CASE WHEN $2 = 'live_trading' THEN now() ELSE NULL END) RETURNING id",
    )
    .bind(user_id)
    .bind(account_mode)
    .fetch_one(&migration)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_risk_limits (trading_account_id, user_id, max_order_usd, max_daily_loss_usd, max_exposure_usd,
         max_slippage_bps, max_orders_per_interval, interval_seconds, max_stale_data_seconds, paper_cash_usd)
         VALUES ($1,$2,500,5000,10000,500,100,60,3600,10000)",
    )
    .bind(account_id)
    .bind(user_id)
    .execute(&migration)
    .await
    .unwrap();

    let wallet = "0x2222222222222222222222222222222222222222";
    let trader_id: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias, monitoring_enabled)
         VALUES ($1,$2,'Leader',true) RETURNING id",
    )
    .bind(user_id)
    .bind(wallet)
    .fetch_one(&migration)
    .await
    .unwrap();

    let strategy_paper: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (
            user_id, trading_account_id, source_trader_id, enabled, sizing_mode, fixed_amount_usd,
            strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions,
            max_slippage_bps, min_order_usd, copy_buy, copy_sell, operating_mode
         ) VALUES ($1,$2,$3,true,'fixed_amount',25,$4,200,2000,10,500,1,true,true,'paper_trading')
         RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(trader_id)
    .bind(Decimal::from(500))
    .fetch_one(&migration)
    .await
    .unwrap();

    let trader_b: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias, monitoring_enabled)
         VALUES ($1,'0x3333333333333333333333333333333333333333','Other',true) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    let strategy_paper_b: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (
            user_id, trading_account_id, source_trader_id, enabled, sizing_mode, fixed_amount_usd,
            strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions,
            max_slippage_bps, min_order_usd, copy_buy, copy_sell, operating_mode
         ) VALUES ($1,$2,$3,false,'fixed_amount',10,$4,50,500,10,500,1,true,true,'paper_trading')
         RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(trader_b)
    .bind(Decimal::from(100))
    .fetch_one(&migration)
    .await
    .unwrap();

    {
        let mut tx = begin_as(&worker, None, "worker").await.unwrap();
        for (sid, amount) in [
            (strategy_paper, Decimal::from(500)),
            (strategy_paper_b, Decimal::from(100)),
        ] {
            ensure_strategy_capital_row(&mut tx, user_id, account_id, sid)
                .await
                .unwrap();
            allocate_to_strategy(
                &mut tx,
                user_id,
                account_id,
                sid,
                amount,
                &format!("mode-reg:{sid}"),
            )
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();
    }

    Harness {
        migration,
        worker,
        vault,
        pipeline,
        book: FixedBook,
        data,
        app_cfg,
        user_id,
        account_id,
        trader_id,
        strategy_paper,
        strategy_paper_b,
        wallet,
    }
}

async fn process(h: &Harness, t: &RawTrade) {
    let out = ingest(&h.worker, t).await.unwrap();
    process_event(
        &h.worker,
        &h.vault,
        &h.book,
        &h.pipeline,
        &h.data,
        &h.app_cfg,
        out.event_id,
    )
    .await
    .unwrap();
}

async fn assert_paper_mode_on_chain(h: &Harness, strategy_id: Uuid) {
    let mig = &h.migration;
    for (table, cond) in [
        ("copy_decisions", format!("strategy_id = '{strategy_id}'")),
        ("orders", format!(
            "decision_id IN (SELECT id FROM copy_decisions WHERE strategy_id = '{strategy_id}')"
        )),
        ("positions", format!("strategy_id = '{strategy_id}'")),
    ] {
        let n: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM {table} WHERE {cond} AND operating_mode <> 'paper_trading'"
        ))
        .fetch_one(mig)
        .await
        .unwrap();
        assert_eq!(n, 0, "{table} has non-paper operating_mode");
    }
    let fill_modes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fills f
         JOIN orders o ON o.id = f.order_id
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND o.operating_mode <> 'paper_trading'",
    )
    .bind(strategy_id)
    .fetch_one(mig)
    .await
    .unwrap();
    assert_eq!(fill_modes, 0, "fills must belong to paper orders");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn migration_0033_recorded_and_default_paper() {
    assert_all_test_database_urls();
    let url = std::env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let ok: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version = 33 AND success)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(ok, "migration 0033 must be applied");
    let max_ver: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(max_ver, 33, "expected linear head 0033");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn paper_buy_sell_ledger_and_mode_tags() {
    assert_all_test_database_urls();
    let h = setup_harness("paper_trading").await;
    let buy = trade("buy", "100", h.wallet, "tok-mode", "0xmode-buy-1");
    process(&h, &buy).await;

    let filled_orders: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders o
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND o.status = 'filled'",
    )
    .bind(h.strategy_paper)
    .fetch_one(&h.migration)
    .await
    .unwrap();
    assert_eq!(filled_orders, 1, "expected one paper fill from BUY");

    assert_paper_mode_on_chain(&h, h.strategy_paper).await;

    let buy2 = trade("buy", "80", h.wallet, "tok-mode", "0xmode-buy-2");
    process(&h, &buy2).await;

    let sell = trade("sell", "50", h.wallet, "tok-mode", "0xmode-sell-1");
    process(&h, &sell).await;

    let shares: Decimal = sqlx::query_scalar(
        "SELECT open_shares FROM positions WHERE strategy_id = $1 AND token_id = 'tok-mode'",
    )
    .bind(h.strategy_paper)
    .fetch_one(&h.migration)
    .await
    .unwrap();
    assert!(shares > Decimal::ZERO);
    assert_paper_mode_on_chain(&h, h.strategy_paper).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn paper_strategy_on_live_account_and_sibling_independence() {
    assert_all_test_database_urls();
    let h = setup_harness("live_trading").await;
    let buy = trade("buy", "100", h.wallet, "tok-live-acct", "0xmode-live-acct-buy");
    process(&h, &buy).await;

    let orders: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders o JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND o.status = 'filled'",
    )
    .bind(h.strategy_paper)
    .fetch_one(&h.migration)
    .await
    .unwrap();
    assert_eq!(orders, 1);

    let other_wallet = "0x3333333333333333333333333333333333333333";
    let buy_b = trade("buy", "100", other_wallet, "tok-b", "0xmode-other-buy");
    process(&h, &buy_b).await;
    let b_filled: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders o JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND o.status = 'filled'",
    )
    .bind(h.strategy_paper_b)
    .fetch_one(&h.migration)
    .await
    .unwrap();
    assert_eq!(b_filled, 0, "disabled sibling strategy must not fill orders");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn duplicate_source_event_not_double_executed() {
    assert_all_test_database_urls();
    let h = setup_harness("paper_trading").await;
    let t = trade("buy", "100", h.wallet, "tok-dedup", "0xmode-dedup-tx");
    process(&h, &t).await;
    let again = ingest(&h.worker, &t).await.unwrap();
    assert!(!again.created || again.duplicate_fragment);
    process_event(
        &h.worker,
        &h.vault,
        &h.book,
        &h.pipeline,
        &h.data,
        &h.app_cfg,
        again.event_id,
    )
    .await
    .unwrap();
    let decisions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM copy_decisions WHERE strategy_id = $1",
    )
    .bind(h.strategy_paper)
    .fetch_one(&h.migration)
    .await
    .unwrap();
    assert_eq!(decisions, 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn global_kill_switch_blocks_paper_strategy() {
    assert_all_test_database_urls();
    let h = setup_harness("paper_trading").await;
    sqlx::query(
        "UPDATE system_settings SET value = '{\"enabled\": true}'::jsonb WHERE key = 'global_kill_switch'",
    )
    .execute(&h.migration)
    .await
    .unwrap();
    let buy = trade("buy", "100", h.wallet, "tok-kill", "0xmode-kill-buy");
    let out = ingest(&h.worker, &buy).await.unwrap();
    process_event(
        &h.worker,
        &h.vault,
        &h.book,
        &h.pipeline,
        &h.data,
        &h.app_cfg,
        out.event_id,
    )
    .await
    .unwrap();
    let orders: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders o JOIN copy_decisions d ON d.id = o.decision_id WHERE d.strategy_id = $1",
    )
    .bind(h.strategy_paper)
    .fetch_one(&h.migration)
    .await
    .unwrap();
    assert_eq!(orders, 0);
    sqlx::query(
        "UPDATE system_settings SET value = '{\"enabled\": false}'::jsonb WHERE key = 'global_kill_switch'",
    )
    .execute(&h.migration)
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn account_monitoring_blocks_even_paper_strategy() {
    assert_all_test_database_urls();
    let h = setup_harness("monitoring").await;
    let buy = trade("buy", "100", h.wallet, "tok-mon", "0xmode-mon-buy");
    let out = ingest(&h.worker, &buy).await.unwrap();
    process_event(
        &h.worker,
        &h.vault,
        &h.book,
        &h.pipeline,
        &h.data,
        &h.app_cfg,
        out.event_id,
    )
    .await
    .unwrap();
    let filled: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders o JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND o.status = 'filled'",
    )
    .bind(h.strategy_paper)
    .fetch_one(&h.migration)
    .await
    .unwrap();
    assert_eq!(filled, 0, "monitoring account must not fill paper strategy orders");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn live_gates_unchanged() {
    assert_all_test_database_urls();
    let cfg = pommarket::config::Config::from_env().unwrap();
    assert!(!cfg.confirm_live_trading);
    assert!(!pommarket::strategy_operating_mode::LIVE_COPY_PRODUCT_READY);
    assert_eq!(
        LIVE_TRADING_NOT_READY_CODE,
        pommarket::strategy_operating_mode::LIVE_TRADING_NOT_READY_CODE
    );
    assert_ne!(cfg.env, "production");
}
