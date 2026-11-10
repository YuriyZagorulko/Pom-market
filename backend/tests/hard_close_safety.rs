mod test_support;

use std::env;

use async_trait::async_trait;
use pommarket::db::begin_as;
use pommarket::db::{connect, migrate};
use pommarket::engine::{BookLevel, OrderBook};
use pommarket::hard_close::{
    preview_hard_close, process_pending_operations, retry_operation, start_hard_close,
};
use pommarket::pipeline::Books;
use pommarket::polymarket::http::PublicClient;
use pommarket::security::hash_password;
use rust_decimal::Decimal;
use test_support::truncate_integration_tables;
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

struct ThinBook;
#[async_trait]
impl Books for ThinBook {
    async fn book(&self, _token_id: &str) -> Result<OrderBook, String> {
        Ok(OrderBook {
            bids: vec![BookLevel {
                price: Decimal::new(50, 2),
                size: Decimal::from(20),
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

struct MinSizeBook;
#[async_trait]
impl Books for MinSizeBook {
    async fn book(&self, _token_id: &str) -> Result<OrderBook, String> {
        Ok(OrderBook {
            bids: vec![BookLevel {
                price: Decimal::new(50, 2),
                size: Decimal::from(10_000),
            }],
            asks: vec![],
            tick: Decimal::new(1, 2),
            min_order_size: Decimal::from(10),
        })
    }
    async fn fee(&self, _token_id: &str) -> Option<pommarket::engine::FeeSchedule> {
        None
    }
}

struct EmptyBook;
#[async_trait]
impl Books for EmptyBook {
    async fn book(&self, _token_id: &str) -> Result<OrderBook, String> {
        Ok(OrderBook {
            bids: vec![],
            asks: vec![],
            tick: Decimal::new(1, 2),
            min_order_size: Decimal::ZERO,
        })
    }
    async fn fee(&self, _token_id: &str) -> Option<pommarket::engine::FeeSchedule> {
        None
    }
}

async fn seed_paper_account(migration: &sqlx::PgPool) -> (Uuid, Uuid) {
    let email = format!("hard-close-{}@example.com", Uuid::new_v4());
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ($1,'HardClose',$2,'user','active',now()) RETURNING id",
    )
    .bind(email)
    .bind(hash_password("hard-close-pass-1").unwrap())
    .fetch_one(migration)
    .await
    .unwrap();
    let account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signer_address, funder_address, signature_type, operating_mode, status)
         VALUES ($1,'HC','0x70997970C51812dc3A010C7d01b50e0d17dc79C8','0x70997970C51812dc3A010C7d01b50e0d17dc79C8','eoa','paper_trading','connected') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(migration)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_risk_limits (trading_account_id, user_id, max_order_usd, max_daily_loss_usd, max_exposure_usd,
         max_slippage_bps, max_orders_per_interval, interval_seconds, max_stale_data_seconds, paper_cash_usd)
         VALUES ($1,$2,50,100,500,500,20,60,120,1000)",
    )
    .bind(account_id)
    .bind(user_id)
    .execute(migration)
    .await
    .unwrap();
    (user_id, account_id)
}

async fn seed_copy_strategy(
    migration: &sqlx::PgPool,
    user_id: Uuid,
    account_id: Uuid,
    wallet: &str,
    alias: &str,
) -> Uuid {
    let trader_id: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias, monitoring_enabled)
         VALUES ($1,$2,$3,true) RETURNING id",
    )
    .bind(user_id)
    .bind(wallet)
    .bind(alias)
    .fetch_one(migration)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO copy_strategies (
            user_id, trading_account_id, source_trader_id, enabled, sizing_mode, proportion,
            strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions,
            max_slippage_bps, min_order_usd, copy_buy, copy_sell
         ) VALUES ($1,$2,$3,true,'proportional',1,5000,500,5000,20,500,1,true,true)
         RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(trader_id)
    .fetch_one(migration)
    .await
    .unwrap()
}

fn public_client() -> PublicClient {
    PublicClient::new(
        std::env::var("POLYMARKET_DATA_API_HOST")
            .unwrap_or_else(|_| "https://data-api.polymarket.com".into()),
        std::env::var("POLYMARKET_CLOB_HOST")
            .unwrap_or_else(|_| "https://clob.polymarket.com".into()),
        std::env::var("POLYMARKET_GAMMA_HOST")
            .unwrap_or_else(|_| "https://gamma-api.polymarket.com".into()),
    )
}

async fn run_hard_close_ticks(worker: &sqlx::PgPool, books: &dyn Books, ticks: u32) {
    for _ in 0..ticks {
        process_pending_operations(worker, books, Decimal::ZERO, 1)
            .await
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn hard_close_covers_manual_and_strategy_lots() {
    test_support::assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (user_id, account_id) = seed_paper_account(&migration).await;
    sqlx::query(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,NULL,'0xtrader-a','tok-a','paper_trading', 40, 0.5, 20)",
    )
    .bind(user_id)
    .bind(account_id)
    .execute(&migration)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,NULL,'manual','tok-b','paper_trading', 30, 0.5, 15)",
    )
    .bind(user_id)
    .bind(account_id)
    .execute(&migration)
    .await
    .unwrap();

    let op = start_hard_close(&worker, user_id, account_id, Some("idem-1".into()))
        .await
        .unwrap();
    let items: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM hard_close_items WHERE operation_id = $1")
            .bind(op)
            .fetch_one(&migration)
            .await
            .unwrap();
    assert_eq!(items, 2);

    process_pending_operations(&worker, &DeepBook, Decimal::ZERO, 1)
        .await
        .unwrap();
    process_pending_operations(&worker, &DeepBook, Decimal::ZERO, 1)
        .await
        .unwrap();

    let open_left: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM positions WHERE trading_account_id = $1 AND open_shares > 0",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(open_left, 0);
    let hc_orders: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders WHERE trading_account_id = $1 AND origin = 'hard_close'",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(hc_orders, 2);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn pending_sell_cancelled_before_hard_close() {
    test_support::assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (user_id, account_id) = seed_paper_account(&migration).await;
    let pos_id: Uuid = sqlx::query_scalar(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd, pending_exit_shares)
         VALUES ($1,$2,NULL,'manual','tok-sell','paper_trading', 100, 0.5, 50, 25) RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orders (user_id, trading_account_id, client_order_id, operating_mode, order_type, side, token_id, limit_price, size_shares, status, origin, position_reserved_usd, fee_reserved_usd)
         VALUES ($1,$2,gen_random_uuid(),'paper_trading','FAK','sell','tok-sell',0.5,40,'submitted','copy',0,0)",
    )
    .bind(user_id)
    .bind(account_id)
    .execute(&migration)
    .await
    .unwrap();

    let _op = start_hard_close(&worker, user_id, account_id, None)
        .await
        .unwrap();
    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders WHERE trading_account_id = $1 AND side = 'sell' AND status IN ('submitted','pending')",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(pending, 0);
    let exit: Decimal =
        sqlx::query_scalar("SELECT pending_exit_shares FROM positions WHERE id = $1")
            .bind(pos_id)
            .fetch_one(&migration)
            .await
            .unwrap();
    assert_eq!(exit, Decimal::ZERO);

    process_pending_operations(&worker, &DeepBook, Decimal::ZERO, 1)
        .await
        .unwrap();
    let open: Decimal = sqlx::query_scalar("SELECT open_shares FROM positions WHERE id = $1")
        .bind(pos_id)
        .fetch_one(&migration)
        .await
        .unwrap();
    assert_eq!(open, Decimal::ZERO);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn concurrent_second_hard_close_rejected() {
    test_support::assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (user_id, account_id) = seed_paper_account(&migration).await;
    sqlx::query(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,NULL,'manual','tok-dup','paper_trading', 10, 0.5, 5)",
    )
    .bind(user_id)
    .bind(account_id)
    .execute(&migration)
    .await
    .unwrap();
    let _ = start_hard_close(&worker, user_id, account_id, Some("key-a".into()))
        .await
        .unwrap();
    let err = start_hard_close(&worker, user_id, account_id, Some("key-b".into()))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("already active"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn idempotency_and_worker_resume_without_duplicate_fills() {
    test_support::assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (user_id, account_id) = seed_paper_account(&migration).await;
    sqlx::query(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,NULL,'manual','tok-resume','paper_trading', 80, 0.5, 40)",
    )
    .bind(user_id)
    .bind(account_id)
    .execute(&migration)
    .await
    .unwrap();

    let op_a = start_hard_close(&worker, user_id, account_id, Some("same-key".into()))
        .await
        .unwrap();
    let op_b = start_hard_close(&worker, user_id, account_id, Some("same-key".into()))
        .await
        .unwrap();
    assert_eq!(op_a, op_b);

    process_pending_operations(&worker, &DeepBook, Decimal::ZERO, 1)
        .await
        .unwrap();
    let fills: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fills f JOIN orders o ON o.id = f.order_id WHERE o.origin = 'hard_close' AND o.trading_account_id = $1",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(fills, 1);

    process_pending_operations(&worker, &DeepBook, Decimal::ZERO, 1)
        .await
        .unwrap();
    let fills2: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fills f JOIN orders o ON o.id = f.order_id WHERE o.origin = 'hard_close' AND o.trading_account_id = $1",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(fills2, 1, "resume must not duplicate hard close fills");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn partial_liquidity_and_no_liquidity_items() {
    test_support::assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (user_id, account_id) = seed_paper_account(&migration).await;
    sqlx::query(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,NULL,'manual','tok-thin','paper_trading', 100, 0.5, 50)",
    )
    .bind(user_id)
    .bind(account_id)
    .execute(&migration)
    .await
    .unwrap();

    let op = start_hard_close(&worker, user_id, account_id, None)
        .await
        .unwrap();
    process_pending_operations(&worker, &ThinBook, Decimal::ZERO, 1)
        .await
        .unwrap();

    let thin_left: Decimal = sqlx::query_scalar(
        "SELECT open_shares FROM positions WHERE trading_account_id = $1 AND token_id = 'tok-thin'",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(thin_left, Decimal::from(80));
    let fills: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fills f JOIN orders o ON o.id = f.order_id WHERE o.trading_account_id = $1 AND o.origin = 'hard_close'",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(fills, 1);
    let thin_status: String = sqlx::query_scalar(
        "SELECT hi.status FROM hard_close_items hi JOIN positions p ON p.id = hi.position_id
         WHERE hi.operation_id = $1 AND p.token_id = 'tok-thin'",
    )
    .bind(op)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(thin_status, "partial");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn hard_close_no_liquidity_marks_item_failed() {
    test_support::assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (user_id, account_id) = seed_paper_account(&migration).await;
    sqlx::query(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,NULL,'manual','tok-empty','paper_trading', 20, 0.5, 10)",
    )
    .bind(user_id)
    .bind(account_id)
    .execute(&migration)
    .await
    .unwrap();
    let op = start_hard_close(&worker, user_id, account_id, None)
        .await
        .unwrap();
    process_pending_operations(&worker, &EmptyBook, Decimal::ZERO, 1)
        .await
        .unwrap();
    let empty_status: String = sqlx::query_scalar(
        "SELECT hi.status FROM hard_close_items hi JOIN positions p ON p.id = hi.position_id
         WHERE hi.operation_id = $1 AND p.token_id = 'tok-empty'",
    )
    .bind(op)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(empty_status, "failed");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn kill_switch_pauses_in_progress_hard_close() {
    test_support::assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (user_id, account_id) = seed_paper_account(&migration).await;
    sqlx::query(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,NULL,'manual','tok-pause','paper_trading', 50, 0.5, 25)",
    )
    .bind(user_id)
    .bind(account_id)
    .execute(&migration)
    .await
    .unwrap();
    let op = start_hard_close(&worker, user_id, account_id, None)
        .await
        .unwrap();
    sqlx::query("UPDATE trading_accounts SET trading_control_state = 'kill_switch' WHERE id = $1")
        .bind(account_id)
        .execute(&migration)
        .await
        .unwrap();
    pommarket::hard_close::pause_operations_for_kill_switch(&worker, account_id)
        .await
        .unwrap();

    process_pending_operations(&worker, &DeepBook, Decimal::ZERO, 1)
        .await
        .unwrap();

    let status: String =
        sqlx::query_scalar("SELECT status FROM hard_close_operations WHERE id = $1")
            .bind(op)
            .fetch_one(&migration)
            .await
            .unwrap();
    assert_eq!(status, "paused");
    let open: Decimal = sqlx::query_scalar(
        "SELECT open_shares FROM positions WHERE trading_account_id = $1 AND token_id = 'tok-pause'",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(open, Decimal::from(50));
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn hard_close_same_token_multi_strategy_and_manual() {
    test_support::assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (user_id, account_id) = seed_paper_account(&migration).await;
    let strat_a = seed_copy_strategy(
        &migration,
        user_id,
        account_id,
        "0xstra00000000000000000000000000000000a1",
        "A",
    )
    .await;
    let strat_b = seed_copy_strategy(
        &migration,
        user_id,
        account_id,
        "0xstra00000000000000000000000000000000b2",
        "B",
    )
    .await;
    let cash_before: Decimal = sqlx::query_scalar(
        "SELECT paper_cash_usd FROM account_risk_limits WHERE trading_account_id = $1",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    for (strat, shares, wallet) in [
        (strat_a, 100_i32, "0xstra00000000000000000000000000000000a1"),
        (strat_b, 150_i32, "0xstra00000000000000000000000000000000b2"),
    ] {
        sqlx::query(
            "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd)
             VALUES ($1,$2,$3,$4,'tok-shared','paper_trading',$5,0.5,$6)",
        )
        .bind(user_id)
        .bind(account_id)
        .bind(strat)
        .bind(wallet)
        .bind(Decimal::from(shares))
        .bind(Decimal::new(shares as i64 * 50, 2))
        .execute(&migration)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,NULL,'manual','tok-shared','paper_trading', 25, 0.5, 12.5)",
    )
    .bind(user_id)
    .bind(account_id)
    .execute(&migration)
    .await
    .unwrap();

    let op = start_hard_close(&worker, user_id, account_id, None)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM hard_close_items WHERE operation_id = $1"
        )
        .bind(op)
        .fetch_one(&migration)
        .await
        .unwrap(),
        3
    );
    run_hard_close_ticks(&worker, &DeepBook, 6).await;

    let open_total: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(open_shares),0) FROM positions WHERE trading_account_id = $1",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(open_total, Decimal::ZERO);
    let hc_shares: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(f.shares),0) FROM fills f
         JOIN orders o ON o.id = f.order_id
         WHERE o.trading_account_id = $1 AND o.origin = 'hard_close'",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(hc_shares, Decimal::from(275));
    let hc_orders: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders WHERE trading_account_id = $1 AND origin = 'hard_close'",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(hc_orders, 3);
    let cash_after: Decimal = sqlx::query_scalar(
        "SELECT paper_cash_usd FROM account_risk_limits WHERE trading_account_id = $1",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(cash_after - cash_before, Decimal::new(275 * 50, 2));
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn hard_close_after_partially_filled_copy_sell() {
    test_support::assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (user_id, account_id) = seed_paper_account(&migration).await;
    let pos_id: Uuid = sqlx::query_scalar(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,NULL,'manual','tok-part','paper_trading', 75, 0.5, 37.5) RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    let order_id: Uuid = sqlx::query_scalar(
        "INSERT INTO orders (user_id, trading_account_id, client_order_id, operating_mode, order_type, side, token_id, limit_price, size_shares, status, origin, position_reserved_usd, fee_reserved_usd)
         VALUES ($1,$2,gen_random_uuid(),'paper_trading','FAK','sell','tok-part',0.5,60,'partially_filled','copy',0,0) RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO fills (user_id, trading_account_id, order_id, price, shares, notional_usd, fee_status)
         VALUES ($1,$2,$3,0.5,25,12.5,'none')",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(order_id)
    .execute(&migration)
    .await
    .unwrap();

    let reserved_before: Decimal = {
        let mut tx = begin_as(&migration, None, "worker").await.unwrap();
        pommarket::pipeline::reserved_sell_shares(&mut tx, account_id, "tok-part", "paper_trading")
            .await
            .unwrap()
    };
    assert_eq!(reserved_before, Decimal::from(35));

    let _op = start_hard_close(&worker, user_id, account_id, None)
        .await
        .unwrap();
    let pending_sells: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders WHERE trading_account_id = $1 AND side = 'sell' AND status IN ('submitted','pending','partially_filled')",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(pending_sells, 0);

    let copy_fills: Decimal =
        sqlx::query_scalar("SELECT COALESCE(SUM(shares),0) FROM fills WHERE order_id = $1")
            .bind(order_id)
            .fetch_one(&migration)
            .await
            .unwrap();
    assert_eq!(copy_fills, Decimal::from(25));

    process_pending_operations(&worker, &DeepBook, Decimal::ZERO, 1)
        .await
        .unwrap();
    let open: Decimal = sqlx::query_scalar("SELECT open_shares FROM positions WHERE id = $1")
        .bind(pos_id)
        .fetch_one(&migration)
        .await
        .unwrap();
    assert_eq!(open, Decimal::ZERO);
    let hc_sold: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(f.shares),0) FROM fills f JOIN orders o ON o.id = f.order_id
         WHERE o.origin = 'hard_close' AND o.trading_account_id = $1",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(hc_sold, Decimal::from(75));
    assert_eq!(
        copy_fills + hc_sold,
        Decimal::from(100),
        "total sold shares must equal original economic exposure"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn hard_close_dust_below_min_order_size() {
    test_support::assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (user_id, account_id) = seed_paper_account(&migration).await;
    sqlx::query(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,NULL,'manual','tok-dust','paper_trading', 8, 0.5, 4)",
    )
    .bind(user_id)
    .bind(account_id)
    .execute(&migration)
    .await
    .unwrap();
    let op = start_hard_close(&worker, user_id, account_id, None)
        .await
        .unwrap();
    process_pending_operations(&worker, &MinSizeBook, Decimal::ZERO, 1)
        .await
        .unwrap();
    let left: Decimal = sqlx::query_scalar(
        "SELECT open_shares FROM positions WHERE trading_account_id = $1 AND token_id = 'tok-dust'",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(left, Decimal::from(8));
    let (status, err): (String, Option<String>) =
        sqlx::query_as("SELECT status, error FROM hard_close_items WHERE operation_id = $1")
            .bind(op)
            .fetch_one(&migration)
            .await
            .unwrap();
    assert_eq!(status, "failed");
    assert!(
        err.as_deref()
            .unwrap_or("")
            .contains("below_minimum_order_size"),
        "unexpected error: {:?}",
        err
    );
    let op_status: String =
        sqlx::query_scalar("SELECT status FROM hard_close_operations WHERE id = $1")
            .bind(op)
            .fetch_one(&migration)
            .await
            .unwrap();
    assert_eq!(op_status, "partial");
    let acct: String =
        sqlx::query_scalar("SELECT trading_control_state FROM trading_accounts WHERE id = $1")
            .bind(account_id)
            .fetch_one(&migration)
            .await
            .unwrap();
    assert_eq!(acct, "hard_close_partial");

    retry_operation(&worker, user_id, account_id, op)
        .await
        .unwrap();
    process_pending_operations(&worker, &MinSizeBook, Decimal::ZERO, 1)
        .await
        .unwrap();
    let fills: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fills f JOIN orders o ON o.id = f.order_id WHERE o.origin = 'hard_close' AND o.trading_account_id = $1",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(fills, 0, "retry must not loop successful dust sells");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn hard_close_partial_leaves_dust_item_blocked() {
    test_support::assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (user_id, account_id) = seed_paper_account(&migration).await;
    sqlx::query(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,NULL,'manual','tok-dust2','paper_trading', 28, 0.5, 14)",
    )
    .bind(user_id)
    .bind(account_id)
    .execute(&migration)
    .await
    .unwrap();
    let op = start_hard_close(&worker, user_id, account_id, None)
        .await
        .unwrap();
    process_pending_operations(&worker, &ThinBook, Decimal::ZERO, 1)
        .await
        .unwrap();
    let left: Decimal = sqlx::query_scalar(
        "SELECT open_shares FROM positions WHERE trading_account_id = $1 AND token_id = 'tok-dust2'",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(left, Decimal::from(8));
    process_pending_operations(&worker, &MinSizeBook, Decimal::ZERO, 1)
        .await
        .unwrap();
    let left2: Decimal = sqlx::query_scalar(
        "SELECT open_shares FROM positions WHERE trading_account_id = $1 AND token_id = 'tok-dust2'",
    )
    .bind(account_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(left2, Decimal::from(8));
    let item_status: String =
        sqlx::query_scalar("SELECT status FROM hard_close_items WHERE operation_id = $1")
            .bind(op)
            .fetch_one(&migration)
            .await
            .unwrap();
    assert_eq!(item_status, "failed");
    let (closed, rem): (Decimal, Decimal) = sqlx::query_as(
        "SELECT closed_shares, remaining_shares FROM hard_close_items WHERE operation_id = $1",
    )
    .bind(op)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(closed, Decimal::from(20));
    assert_eq!(rem, Decimal::from(8));
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn hard_close_preview_matches_start_items_and_is_read_only() {
    test_support::assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    let app = connect(&env::var("APP_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (user_id, account_id) = seed_paper_account(&migration).await;
    let strat = seed_copy_strategy(
        &migration,
        user_id,
        account_id,
        "0xpreview00000000000000000000000001",
        "P",
    )
    .await;
    for (shares, strat_id, wallet) in [
        (
            Decimal::from(40),
            Some(strat),
            "0xpreview00000000000000000000000001",
        ),
        (Decimal::from(15), None, "manual"),
    ] {
        sqlx::query(
            "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd)
             VALUES ($1,$2,$3,$4,'tok-prev','paper_trading',$5,0.5,$6)",
        )
        .bind(user_id)
        .bind(account_id)
        .bind(strat_id)
        .bind(wallet)
        .bind(shares)
        .bind(shares * Decimal::new(50, 2))
        .execute(&migration)
        .await
        .unwrap();
    }
    let orders_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders")
        .fetch_one(&migration)
        .await
        .unwrap();
    let ops_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM hard_close_operations")
        .fetch_one(&migration)
        .await
        .unwrap();
    let preview = preview_hard_close(&app, user_id, account_id, &public_client())
        .await
        .unwrap();
    assert_eq!(preview["read_only"], true);
    assert_eq!(preview["position_count"], 2);
    let positions = preview["positions"].as_array().unwrap();
    assert_eq!(positions.len(), 2);
    fn json_dec(v: &serde_json::Value) -> Decimal {
        if let Some(s) = v.as_str() {
            return s.parse().expect("decimal string");
        }
        if let Some(f) = v.as_f64() {
            return Decimal::from_f64_retain(f).expect("decimal f64");
        }
        panic!("decimal json");
    }
    for row in positions {
        assert_eq!(
            json_dec(&row["sellable_shares"]),
            json_dec(&row["open_shares"])
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM orders")
            .fetch_one(&migration)
            .await
            .unwrap(),
        orders_before
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM hard_close_operations")
            .fetch_one(&migration)
            .await
            .unwrap(),
        ops_before
    );

    let op = start_hard_close(&worker, user_id, account_id, None)
        .await
        .unwrap();
    let items: Vec<(Uuid, Decimal)> = sqlx::query_as(
        "SELECT position_id, remaining_shares FROM hard_close_items WHERE operation_id = $1 ORDER BY created_at",
    )
    .bind(op)
    .fetch_all(&migration)
    .await
    .unwrap();
    assert_eq!(items.len(), 2);
    for row in positions {
        let pid: Uuid = row["position_id"].as_str().unwrap().parse().unwrap();
        let sellable = json_dec(&row["sellable_shares"]);
        let item = items
            .iter()
            .find(|(p, _)| *p == pid)
            .expect("item for preview position");
        assert_eq!(item.1, sellable);
    }
    run_hard_close_ticks(&worker, &DeepBook, 2).await;
    let progressed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hard_close_items WHERE operation_id = $1 AND status <> 'pending'",
    )
    .bind(op)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert!(
        progressed >= 1,
        "worker ticks should execute at least one eligible item"
    );
}
