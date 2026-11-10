//! Execution-path integration for strategy capital accounting (pommarket_test + migration 0016).

mod common;
mod test_support;

use std::env;

use async_trait::async_trait;
use common::TestApi;
use pommarket::db::{begin_as, connect, migrate};
use pommarket::engine::{BookLevel, OrderBook};
use pommarket::hard_close::{process_pending_operations, start_hard_close};
use pommarket::pipeline::{ingest, process_event, Books, PipelineConfig};
use pommarket::strategy_accounting::{reconcile_account, reserve_order_cash};
use rust_decimal::Decimal;
use serde_json::json;
use sqlx::PgPool;
use test_support::{assert_all_test_database_urls, truncate_integration_tables};
use uuid::Uuid;

struct DeepBook;
#[async_trait]
impl Books for DeepBook {
    async fn book(&self, _token_id: &str) -> Result<OrderBook, String> {
        Ok(OrderBook {
            bids: vec![BookLevel {
                price: Decimal::new(50, 2),
                size: Decimal::from(10_000),
            }],
            asks: vec![],
            tick: Decimal::new(1, 2),
            min_order_size: Decimal::ZERO,
        })
    }
    async fn fee(&self, _token_id: &str) -> Option<pommarket::engine::FeeSchedule> {
        None
    }
}

async fn seed_copy_account(migration: &PgPool) -> (Uuid, Uuid, Uuid, String) {
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES (concat('exec-', gen_random_uuid()::text, '@example.com'), 'Exec', 'x', 'user', 'active', now())
         RETURNING id",
    )
    .fetch_one(migration)
    .await
    .unwrap();
    let account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signer_address, funder_address, signature_type, operating_mode, status)
         VALUES ($1,'Exec','0x70997970C51812dc3A010C7d01b50e0d17dc79C8','0x70997970C51812dc3A010C7d01b50e0d17dc79C8','eoa','paper_trading','connected')
         RETURNING id",
    )
    .bind(user_id)
    .fetch_one(migration)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_risk_limits (trading_account_id, user_id, max_order_usd, max_daily_loss_usd, max_exposure_usd,
         max_slippage_bps, max_orders_per_interval, interval_seconds, max_stale_data_seconds, paper_cash_usd)
         VALUES ($1,$2,500,500,5000,500,100,60,3600,2000)",
    )
    .bind(account_id)
    .bind(user_id)
    .execute(migration)
    .await
    .unwrap();
    let wallet = "0xcccccccccccccccccccccccccccccccccccccccc";
    let trader_id: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias, monitoring_enabled)
         VALUES ($1,$2,'L',true) RETURNING id",
    )
    .bind(user_id)
    .bind(wallet)
    .fetch_one(migration)
    .await
    .unwrap();
    let strategy_id: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (
            user_id, trading_account_id, source_trader_id, enabled, sizing_mode, fixed_amount_usd,
            strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions,
            max_slippage_bps, min_order_usd, copy_buy, copy_sell
         ) VALUES ($1,$2,$3,true,'fixed_amount',25,500,200,2000,10,500,1,true,true)
         RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(trader_id)
    .fetch_one(migration)
    .await
    .unwrap();
    (user_id, account_id, strategy_id, wallet.to_string())
}

async fn run_hard_close_ticks(worker: &PgPool, books: &dyn Books, ticks: u32) {
    for _ in 0..ticks {
        process_pending_operations(worker, books, Decimal::ZERO, 1)
            .await
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn copy_buy_skipped_until_capital_bootstrap() {
    assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
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
    let (data, app_cfg) = test_support::pipeline_runtime();
    let book = common::FixedBook;

    let (_user, _account, strategy_id, wallet) = seed_copy_account(&migration).await;
    let trade = common::TestApi::sample_trade(&wallet, "buy", "10", "0xnobootstrap");
    let out = ingest(&worker, &trade).await.unwrap();
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

    let reason: String = sqlx::query_scalar(
        "SELECT reason FROM copy_decisions WHERE strategy_id = $1 ORDER BY detected_at DESC LIMIT 1",
    )
    .bind(strategy_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(reason, "strategy_capital_not_initialized");
    let orders: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders o
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1",
    )
    .bind(strategy_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(orders, 0);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn hard_close_strategy_position_updates_ledger_not_paper_cash() {
    assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (user_id, account_id, strategy_id, wallet) = seed_copy_account(&migration).await;
    test_support::bootstrap_paper_capital_for_account(&migration, account_id).await;
    let mut boot_chk = begin_as(&migration, None, "worker").await.unwrap();
    let boot_q: String = sqlx::query_scalar(
        "SELECT bootstrap_quality FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .fetch_one(&mut *boot_chk)
    .await
    .unwrap();
    boot_chk.commit().await.unwrap();
    assert!(
        boot_q == "migrated" || boot_q == "estimated",
        "capital must be bootstrapped, got {boot_q}"
    );

    let cash_before: Decimal = sqlx::query_scalar(
        "SELECT paper_cash_usd FROM account_risk_limits WHERE trading_account_id = $1",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    let avail_before: Decimal = sqlx::query_scalar(
        "SELECT available_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .fetch_one(&migration)
    .await
    .unwrap();

    let position_id: Uuid = sqlx::query_scalar(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode,
         open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,$3,$4,'tok-hc','paper_trading',100,0.5,50) RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(strategy_id)
    .bind(wallet)
    .fetch_one(&migration)
    .await
    .unwrap();

    let op = start_hard_close(&worker, user_id, account_id, Some("hc-ledger".into()))
        .await
        .unwrap();
    run_hard_close_ticks(&worker, &DeepBook, 6).await;

    let cash_after: Decimal = sqlx::query_scalar(
        "SELECT paper_cash_usd FROM account_risk_limits WHERE trading_account_id = $1",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(
        cash_after, cash_before,
        "bootstrapped strategy hard close must not bump account paper_cash"
    );

    let avail_after: Decimal = sqlx::query_scalar(
        "SELECT available_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert!(
        avail_after > avail_before,
        "strategy should receive sell proceeds"
    );

    let ledger_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM capital_ledger_entries WHERE strategy_id = $1 AND entry_type = 'hard_close_sell'",
    )
    .bind(strategy_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(ledger_rows, 1);

    let open: Decimal = sqlx::query_scalar("SELECT open_shares FROM positions WHERE id = $1")
        .bind(position_id)
        .fetch_one(&migration)
        .await
        .unwrap();
    assert_eq!(open, Decimal::ZERO);

    let status: String =
        sqlx::query_scalar("SELECT status FROM hard_close_operations WHERE id = $1")
            .bind(op)
            .fetch_one(&migration)
            .await
            .unwrap();
    assert_eq!(status, "completed");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn hard_close_retry_does_not_duplicate_ledger_fill() {
    assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (user_id, account_id, strategy_id, wallet) = seed_copy_account(&migration).await;
    test_support::bootstrap_paper_capital_for_account(&migration, account_id).await;

    sqlx::query(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode,
         open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,$3,$4,'tok-retry','paper_trading',40,0.5,20)",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(strategy_id)
    .bind(wallet)
    .execute(&migration)
    .await
    .unwrap();

    let _op = start_hard_close(&worker, user_id, account_id, None)
        .await
        .unwrap();
    run_hard_close_ticks(&worker, &DeepBook, 9).await;

    let fills: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fills f
         JOIN orders o ON o.id = f.order_id
         WHERE o.trading_account_id = $1 AND o.origin = 'hard_close' AND o.token_id = 'tok-retry'",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(fills, 1);
    let ledger: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM capital_ledger_entries WHERE strategy_id = $1 AND entry_type = 'hard_close_sell'",
    )
    .bind(strategy_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(ledger, 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn bulk_cancel_buy_releases_reservation_once() {
    assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let pool = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&pool, &migration_url).await;

    let (user_id, account_id, strategy_id, _wallet) = seed_copy_account(&pool).await;
    test_support::bootstrap_paper_capital_for_account(&pool, account_id).await;

    let trader_id: Uuid =
        sqlx::query_scalar("SELECT source_trader_id FROM copy_strategies WHERE id = $1")
            .bind(strategy_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let mut tx = begin_as(&pool, Some(user_id), "user").await.unwrap();
    let event_id: Uuid = sqlx::query_scalar(
        "INSERT INTO source_events (dedup_key, source_wallet, token_id, side, price, size_shares, notional_usd, event_ts, origin, status, raw)
         VALUES ('exec-cancel', '0xcccccccccccccccccccccccccccccccccccccccc','tok','buy',0.5,10,5,now(),'test','processed','{}'::jsonb)
         RETURNING id",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let decision_id: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_decisions (user_id, trading_account_id, source_trader_id, source_event_id, strategy_id, status, reason, operating_mode, side, strategy_snapshot, checks, detected_at)
         VALUES ($1,$2,$3,$4,$5,'approved','ok','paper_trading','buy','{}','{}',now()) RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(trader_id)
    .bind(event_id)
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let order_id: Uuid = sqlx::query_scalar(
        "INSERT INTO orders (user_id, trading_account_id, decision_id, source_event_id, client_order_id, operating_mode, order_type, side,
         token_id, limit_price, size_shares, status, origin, position_reserved_usd, fee_reserved_usd)
         VALUES ($1,$2,$3,$4,gen_random_uuid(),'paper_trading','GTC','buy','tok',0.5,10,'submitted','copy',38,2)
         RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(decision_id)
    .bind(event_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    reserve_order_cash(
        &mut tx,
        user_id,
        account_id,
        strategy_id,
        order_id,
        Decimal::from(40),
        "test-reserve-1",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let mut tx = begin_as(&pool, None, "worker").await.unwrap();
    pommarket::pipeline::cancel_open_buy_orders(&mut tx, account_id, "paper_trading")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let (avail, reserved): (Decimal, Decimal) = sqlx::query_as(
        "SELECT available_cash_usd, reserved_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(reserved, Decimal::ZERO);
    assert!(avail > Decimal::ZERO);

    let status: String = sqlx::query_scalar("SELECT status FROM orders WHERE id = $1")
        .bind(order_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "cancelled");

    let report = reconcile_account(&pool, account_id).await.unwrap();
    assert!(report.reconciled, "after cancel: {:?}", report.notes);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn risk_patch_rejects_direct_paper_cash_mutation() {
    let api = TestApi::boot().await;
    let admin_login = api
        .post(
            "/api/v1/auth/login",
            json!({"email": "admin@example.com", "password": "admin-password-1"}),
        )
        .await;
    let admin_token = admin_login["access_token"].as_str().unwrap();
    let email = format!("patch-cash-{}@example.com", Uuid::new_v4());
    let password = api.invite_user(admin_token, &email, "PatchCash").await;
    let token = api.login_user(&email, &password).await;

    let account = api
        .send(
            "POST",
            "/api/v1/accounts",
            &token,
            json!({
                "name": "Patch",
                "signature_type": "eoa",
                "signer_address": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
            }),
        )
        .await;
    let account_id = account["id"].as_str().unwrap();

    let patch_status = api
        .http
        .patch(format!("{}/api/v1/accounts/{account_id}/risk", api.base))
        .header("authorization", format!("Bearer {token}"))
        .json(&json!({"paper_cash_usd": "999"}))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(patch_status, reqwest::StatusCode::BAD_REQUEST);
}
