//! End-to-end PAPER position lifecycle: pipeline SELL, hard close, settlement, multi-cycle P&L.

mod test_support;

use std::env;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use chrono::Utc;
use pommarket::db::{begin_as, connect, migrate};
use pommarket::engine::{limit_price, truncate_lot, BookLevel, OrderBook};
use pommarket::hard_close::{process_pending_operations, retry_operation, start_hard_close};
use pommarket::paper_settlement;
use pommarket::pipeline::{ingest, process_event, Books, PipelineConfig};
use pommarket::polymarket::http::PublicClient;
use pommarket::polymarket::RawTrade;
use pommarket::position_close::prior_close_realized_sum;
use pommarket::security::Vault;
use pommarket::strategy_analytics::{list_activity, ActivityListParams};
use rust_decimal::Decimal;
use serde_json::json;
use test_support::{bootstrap_paper_capital_for_account, truncate_integration_tables};
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

static TRADE_SEQ: AtomicU32 = AtomicU32::new(1);

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

struct ThinSellBook;
#[async_trait]
impl Books for ThinSellBook {
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

struct DeepSellBook;
#[async_trait]
impl Books for DeepSellBook {
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

fn trade(side: &str, leader_shares: &str, wallet: &str, token: &str) -> RawTrade {
    let n = TRADE_SEQ.fetch_add(1, Ordering::SeqCst);
    let payload = json!({
        "proxyWallet": wallet,
        "transactionHash": format!("0x{:064x}", n),
        "asset": token,
        "conditionId": "0xe2econditioneeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        "side": side,
        "price": 0.5,
        "size": leader_shares,
        "timestamp": Utc::now().timestamp(),
        "title": "E2E Market",
        "outcome": "Yes"
    });
    pommarket::polymarket::parse_activity_payload(&payload, "test")
        .pop()
        .unwrap()
}

async fn setup_pipeline_env() -> (
    sqlx::PgPool,
    Vault,
    FixedBook,
    PipelineConfig,
    pommarket::polymarket::http::PublicClient,
    pommarket::config::Config,
    Uuid,
    Uuid,
    Uuid,
    String,
) {
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
    let vault = Vault::new(
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
    let book = FixedBook;
    let (data, app_cfg) = test_support::pipeline_runtime();

    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ('e2e-close@example.com','E2E','x','user','active',now()) RETURNING id",
    )
    .fetch_one(&migration)
    .await
    .unwrap();
    let account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signer_address, funder_address, signature_type, operating_mode, status)
         VALUES ($1,'Paper','0xabc000000000000000000000000000000000001','0xabc000000000000000000000000000000000001','eoa','paper_trading','connected') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_risk_limits (trading_account_id, user_id, max_order_usd, max_daily_loss_usd, max_exposure_usd,
         max_slippage_bps, max_orders_per_interval, interval_seconds, max_stale_data_seconds, paper_cash_usd)
         VALUES ($1,$2,500,2000,5000,500,100,60,3600,50000)",
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
         ) VALUES ($1,$2,$3,true,'proportional',1,2000,500,5000,10,500,1,true,true) RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(trader_id)
    .bind(Decimal::from(2000))
    .fetch_one(&migration)
    .await
    .unwrap();
    bootstrap_paper_capital_for_account(&worker, account_id).await;
    fund_strategy_from_bootstrap(&migration, strategy_id).await;
    let wallet = "0x1111111111111111111111111111111111111111".to_string();
    (
        worker,
        vault,
        book,
        pipeline,
        data,
        app_cfg,
        user_id,
        strategy_id,
        account_id,
        wallet,
    )
}

async fn fund_strategy_from_bootstrap(pool: &sqlx::PgPool, strategy_id: Uuid) {
    let mut tx = begin_as(pool, Some(
        sqlx::query_scalar::<_, Uuid>("SELECT user_id FROM copy_strategies WHERE id = $1")
            .bind(strategy_id)
            .fetch_one(pool)
            .await
            .unwrap(),
    ), "user")
    .await
    .unwrap();
    let account: Uuid = sqlx::query_scalar("SELECT trading_account_id FROM copy_strategies WHERE id = $1")
        .bind(strategy_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let user: Uuid = sqlx::query_scalar("SELECT user_id FROM copy_strategies WHERE id = $1")
        .bind(strategy_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    pommarket::strategy_accounting::allocate_to_strategy(
        &mut tx,
        user,
        account,
        strategy_id,
        Decimal::from(2000),
        &format!("e2e-fund-{}", Uuid::new_v4()),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

async fn run_trade(
    pool: &sqlx::PgPool,
    vault: &Vault,
    book: &FixedBook,
    pipeline: &PipelineConfig,
    data: &PublicClient,
    app_cfg: &pommarket::config::Config,
    side: &str,
    leader_shares: &str,
    wallet: &str,
    token: &str,
) {
    let t = trade(side, leader_shares, wallet, token);
    let out = ingest(pool, &t).await.unwrap();
    process_event(pool, vault, book, pipeline, data, app_cfg, out.event_id)
        .await
        .unwrap();
}

async fn close_rows(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
) -> Vec<(i64, Decimal, Decimal)> {
    let mut tx = begin_as(pool, Some(user_id), "user").await.unwrap();
    let rows = sqlx::query_as(
        "SELECT close_cycle, realized_pnl_usd, closed_shares
         FROM position_close_events WHERE strategy_id = $1 ORDER BY close_cycle",
    )
    .bind(strategy_id)
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    rows
}

async fn open_shares(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    token: &str,
) -> Decimal {
    let mut tx = begin_as(pool, Some(user_id), "user").await.unwrap();
    let v: Decimal = sqlx::query_scalar(
        "SELECT open_shares FROM positions WHERE strategy_id = $1 AND token_id = $2",
    )
    .bind(strategy_id)
    .bind(token)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    v
}

async fn position_id_for_token(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    token: &str,
) -> Uuid {
    let mut tx = begin_as(pool, Some(user_id), "user").await.unwrap();
    let id: Uuid = sqlx::query_scalar(
        "SELECT id FROM positions WHERE strategy_id = $1 AND token_id = $2",
    )
    .bind(strategy_id)
    .bind(token)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    id
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn e2e_partial_then_full_leader_sell_one_closed() {
    let (pool, vault, book, pipeline, data, app_cfg, user_id, strategy_id, _, wallet) =
        setup_pipeline_env().await;
    let token = "tok-e2e-a";
    run_trade(
        &pool, &vault, &book, &pipeline, &data, &app_cfg, "buy", "100", &wallet, token,
    )
    .await;
    let pos_after_buy = open_shares(&pool, user_id, strategy_id, token).await;
    assert!(pos_after_buy > Decimal::ZERO);

    run_trade(
        &pool, &vault, &book, &pipeline, &data, &app_cfg, "sell", "60", &wallet, token,
    )
    .await;
    let partial_open = open_shares(&pool, user_id, strategy_id, token).await;
    // Proportional sizing uses Equal Copy SELL: copy notional = leader_notional × coefficient,
    // then shares = truncate_lot(capped_notional / copy_limit). Limit is worse than source (slippage).
    let leader_sell_notional = Decimal::from(60) * Decimal::new(50, 2);
    let sell_limit = limit_price(false, Decimal::new(50, 2), 500, Decimal::new(1, 2));
    let expected_copy_sell_shares = truncate_lot(leader_sell_notional / sell_limit);
    let expected_partial = (pos_after_buy - expected_copy_sell_shares).round_dp(4);
    assert!(
        (partial_open - expected_partial).abs() <= Decimal::new(1, 4),
        "equal-copy sell: expected {expected_partial} open (sold {expected_copy_sell_shares} @ limit {sell_limit}), got {partial_open}"
    );
    let closes_mid = close_rows(&pool, user_id, strategy_id).await;
    assert!(closes_mid.is_empty(), "partial sell must not create CLOSED");

    let (sells, _) = list_activity(
        &pool,
        user_id,
        strategy_id,
        ActivityListParams {
            activity_type: Some("sell"),
            side: None,
            status: None,
            search: None,
            from: None,
            to: None,
            limit: 10,
            offset: 0,
        },
    )
    .await
    .unwrap();
    let partial_row = sells
        .iter()
        .find(|r| {
            r.position_open_shares
                .map(|s| (s - expected_partial).abs() <= Decimal::new(1, 4))
                .unwrap_or(false)
        })
        .expect("partial row shows historical open shares after first sell");
    assert!(
        partial_row
            .fill_shares
            .map(|s| (s - expected_copy_sell_shares).abs() <= Decimal::new(1, 4))
            .unwrap_or(false),
        "filled shares should match equal-copy sizing"
    );
    assert!(partial_row.filled_at.is_some());

    run_trade(
        &pool, &vault, &book, &pipeline, &data, &app_cfg, "sell", "40", &wallet, token,
    )
    .await;
    let open = open_shares(&pool, user_id, strategy_id, token).await;
    assert_eq!(open, Decimal::ZERO);

    let closes = close_rows(&pool, user_id, strategy_id).await;
    assert_eq!(closes.len(), 1);
    assert_eq!(closes[0].0, 0);
    assert!(
        closes[0].2 >= Decimal::from(9),
        "closed shares should reflect full cycle sells, got {}",
        closes[0].2
    );

    let mut tx = begin_as(&pool, Some(user_id), "user").await.unwrap();
    let cumulative: Decimal = sqlx::query_scalar(
        "SELECT realized_pnl_usd FROM positions WHERE strategy_id = $1 AND token_id = $2",
    )
    .bind(strategy_id)
    .bind(token)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(closes[0].1, cumulative);

    assert_eq!(
        partial_row.position_open_shares.map(|d| d.round_dp(4)),
        Some(expected_partial)
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn e2e_two_cycles_incremental_closed_pnl() {
    let (pool, vault, book, pipeline, data, app_cfg, user_id, strategy_id, _, wallet) =
        setup_pipeline_env().await;
    let token = "tok-e2e-b";
    let position_id: Uuid = {
        run_trade(
            &pool, &vault, &book, &pipeline, &data, &app_cfg, "buy", "100", &wallet, token,
        )
        .await;
        run_trade(
            &pool, &vault, &book, &pipeline, &data, &app_cfg, "sell", "100", &wallet, token,
        )
        .await;
        position_id_for_token(&pool, user_id, strategy_id, token).await
    };
    let cycle1 = close_rows(&pool, user_id, strategy_id).await;
    assert_eq!(cycle1.len(), 1);
    let pnl_after_c1 = cycle1[0].1;
    let mut tx = begin_as(&pool, Some(user_id), "user").await.unwrap();
    let baseline_before_reopen: Decimal = sqlx::query_scalar(
        "SELECT realized_pnl_usd FROM positions WHERE id = $1",
    )
    .bind(position_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    run_trade(
        &pool, &vault, &book, &pipeline, &data, &app_cfg, "buy", "100", &wallet, token,
    )
    .await;
    let mut tx = begin_as(&pool, Some(user_id), "user").await.unwrap();
    let (cycle, baseline): (i64, Decimal) = sqlx::query_as(
        "SELECT close_cycle, close_cycle_realized_baseline FROM positions WHERE id = $1",
    )
    .bind(position_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(cycle, 1);
    assert_eq!(baseline, baseline_before_reopen);

    run_trade(
        &pool, &vault, &book, &pipeline, &data, &app_cfg, "sell", "100", &wallet, token,
    )
    .await;

    let closes = close_rows(&pool, user_id, strategy_id).await;
    assert_eq!(closes.len(), 2);
    assert_eq!(closes[0].1, pnl_after_c1, "cycle 0 snapshot is immutable");
    let mut tx = begin_as(&pool, Some(user_id), "user").await.unwrap();
    let cumulative: Decimal = sqlx::query_scalar(
        "SELECT realized_pnl_usd FROM positions WHERE id = $1",
    )
    .bind(position_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let cycle2_only = closes[1].1;
    assert_eq!(cycle2_only, cumulative - baseline_before_reopen);
    let sum_cycles: Decimal = closes.iter().map(|c| c.1).sum();
    assert_eq!(sum_cycles, cumulative);

    let mut tx = begin_as(&pool, Some(user_id), "user").await.unwrap();
    let closed_positions_sum: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(realized_pnl_usd), 0) FROM closed_positions WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        closed_positions_sum, sum_cycles,
        "closed_positions must store per-cycle realized, matching position_close_events"
    );

    let mut tx = begin_as(&pool, Some(user_id), "user").await.unwrap();
    let prior = prior_close_realized_sum(&mut tx, position_id, 1).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(prior, pnl_after_c1);

    let mut tx = begin_as(&pool, Some(user_id), "user").await.unwrap();
    let perf_realized: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(realized_pnl_usd),0) FROM positions WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let close_dup: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM position_close_events WHERE position_id = $1",
    )
    .bind(position_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(perf_realized, cumulative);
    assert_eq!(close_dup, 2);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn e2e_hard_close_partial_no_closed_event() {
    test_support::assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (user_id, account_id, strategy_id, wallet) = strategy_capital_seed(&migration).await;
    bootstrap_paper_capital_for_account(&worker, account_id).await;
    fund_strategy_from_bootstrap(&migration, strategy_id).await;
    let position_id: Uuid = sqlx::query_scalar(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode,
         open_shares, avg_price, cost_basis_usd, close_cycle)
         VALUES ($1,$2,$3,$4,'tok-hc-partial','paper_trading',100,0.5,50,0) RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(strategy_id)
    .bind(wallet)
    .fetch_one(&migration)
    .await
    .unwrap();
    let op = start_hard_close(&worker, user_id, account_id, None, Some("hc-e2e-partial".into()))
        .await
        .unwrap();
    process_pending_operations(&worker, &ThinSellBook, &test_public_client(), Decimal::ZERO, 3)
        .await
        .unwrap();
    let mid: Decimal = sqlx::query_scalar("SELECT open_shares FROM positions WHERE id = $1")
        .bind(position_id)
        .fetch_one(&migration)
        .await
        .unwrap();
    assert!(mid > Decimal::ZERO);
    let mut tx = begin_as(&migration, Some(user_id), "user").await.unwrap();
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM position_close_events WHERE position_id = $1",
    )
    .bind(position_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(n, 0);
    let _ = op;
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn e2e_hard_close_full_one_closed() {
    test_support::assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (user_id, account_id, strategy_id, wallet) = strategy_capital_seed(&migration).await;
    bootstrap_paper_capital_for_account(&worker, account_id).await;
    fund_strategy_from_bootstrap(&migration, strategy_id).await;
    let position_id: Uuid = sqlx::query_scalar(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode,
         open_shares, avg_price, cost_basis_usd, close_cycle)
         VALUES ($1,$2,$3,$4,'tok-hc-e2e','paper_trading',100,0.5,50,0) RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(strategy_id)
    .bind(wallet)
    .fetch_one(&migration)
    .await
    .unwrap();
    {
        let mut tx = begin_as(&migration, None, "worker").await.unwrap();
        pommarket::strategy_accounting::sync_invested_from_positions(&mut tx, strategy_id)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    let _op = start_hard_close(&worker, user_id, account_id, None, Some("hc-e2e-1".into()))
        .await
        .unwrap();
    process_pending_operations(&worker, &DeepSellBook, &test_public_client(), Decimal::ZERO, 12)
        .await
        .unwrap();
    let open: Decimal = sqlx::query_scalar("SELECT open_shares FROM positions WHERE id = $1")
        .bind(position_id)
        .fetch_one(&migration)
        .await
        .unwrap();
    assert_eq!(open, Decimal::ZERO, "deep book completes hard close");

    let mut tx = begin_as(&migration, Some(user_id), "user").await.unwrap();
    let close: Option<(String, Decimal)> = sqlx::query_as(
        "SELECT close_reason, realized_pnl_usd FROM position_close_events WHERE position_id = $1",
    )
    .bind(position_id)
    .fetch_optional(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let close = close.expect("full hard close records one CLOSED event");
    assert_eq!(close.0, "hard_close");
    let mut tx = begin_as(&migration, Some(user_id), "user").await.unwrap();
    let cumulative: Decimal = sqlx::query_scalar(
        "SELECT realized_pnl_usd FROM positions WHERE id = $1",
    )
    .bind(position_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(close.1, cumulative);
}

async fn strategy_capital_seed(pool: &sqlx::PgPool) -> (Uuid, Uuid, Uuid, String) {
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ($1,'HC-E2E','x','user','active',now()) RETURNING id",
    )
    .bind(format!("hc-e2e-{}@example.com", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .unwrap();
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signature_type, signer_address, funder_address, operating_mode, status, trading_control_state)
         VALUES ($1,'A','eoa','0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc','0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc','paper_trading','connected','normal')
         RETURNING id",
    )
    .bind(user)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_risk_limits (trading_account_id, user_id, max_order_usd, max_daily_loss_usd, max_exposure_usd, max_slippage_bps, max_orders_per_interval, interval_seconds, max_stale_data_seconds, paper_cash_usd)
         VALUES ($1,$2,500,2000,5000,500,100,60,3600,50000)",
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
        "INSERT INTO copy_strategies (user_id, trading_account_id, source_trader_id, enabled, sizing_mode, proportion, strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions, max_slippage_bps, min_order_usd, copy_buy, copy_sell)
         VALUES ($1,$2,$3,true,'proportional',1,2000,500,5000,10,500,1,true,true) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(trader)
    .fetch_one(pool)
    .await
    .unwrap();
    let wallet = "0x2222222222222222222222222222222222222222".to_string();
    (user, account, strategy, wallet)
}

fn test_public_client() -> PublicClient {
    PublicClient::new(
        "https://data-api.polymarket.com".into(),
        "https://clob.polymarket.com".into(),
        "https://gamma-api.polymarket.com".into(),
    )
}

async fn mock_resolved(server: &MockServer, condition: &str, token_win: &str, token_lose: &str) {
    let cid = condition.trim_start_matches("0x");
    let body = json!({
        "closed": true,
        "tokens": [
            {"token_id": token_lose, "outcome": "No", "winner": false, "price": 0},
            {"token_id": token_win, "outcome": "Yes", "winner": true, "price": 1}
        ]
    });
    Mock::given(method("GET"))
        .and(path(format!("/markets/0x{cid}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn e2e_resolution_after_partial_sell_includes_cycle_fills() {
    let (pool, vault, book, pipeline, data, app_cfg, user_id, strategy_id, _, wallet) =
        setup_pipeline_env().await;
    let token = "tok-e2e-res";
    run_trade(
        &pool, &vault, &book, &pipeline, &data, &app_cfg, "buy", "100", &wallet, token,
    )
    .await;
    run_trade(
        &pool, &vault, &book, &pipeline, &data, &app_cfg, "sell", "50", &wallet, token,
    )
    .await;
    let pos_id = position_id_for_token(&pool, user_id, strategy_id, token).await;
    let open_before_settle = open_shares(&pool, user_id, strategy_id, token).await;
    assert!(open_before_settle > Decimal::ZERO);

    let server = MockServer::start().await;
    let condition = "0xe2econditioneeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    mock_resolved(&server, condition, token, "tok-e2e-res-no").await;
    let client = PublicClient::new(
        "http://127.0.0.1:1".into(),
        server.uri(),
        "http://127.0.0.1:1".into(),
    );
    let r = paper_settlement::try_settle_position(&pool, &client, pos_id)
        .await
        .unwrap()
        .expect("resolution");
    assert_eq!(r.status, "resolved_win");
    assert_eq!(r.payout_usd, open_before_settle);

    let mut tx = begin_as(&pool, Some(user_id), "user").await.unwrap();
    let snap: (Decimal, Decimal, Option<Decimal>, Decimal, String) = sqlx::query_as(
        "SELECT closed_shares, total_cost_basis_usd, avg_exit_price, realized_pnl_usd, close_reason
         FROM position_close_events WHERE position_id = $1",
    )
    .bind(pos_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(snap.4, "resolution_win");
    assert!(
        snap.2.map(|p| p > Decimal::ZERO).unwrap_or(false)
            || snap.2.is_none(),
        "exit price must come from sell fills only, not resolution"
    );
    assert!(
        snap.0 >= open_before_settle,
        "cycle closed shares includes buys/fills in cycle (got {} open before settle {})",
        snap.0,
        open_before_settle
    );
    assert!(snap.1 > Decimal::ZERO);
    let cumulative: Decimal = sqlx::query_scalar(
        "SELECT realized_pnl_usd FROM positions WHERE id = $1",
    )
    .bind(pos_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(snap.3, cumulative);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn e2e_reprocessing_leader_sell_does_not_duplicate_close() {
    let (pool, vault, book, pipeline, data, app_cfg, user_id, strategy_id, _, wallet) =
        setup_pipeline_env().await;
    let token = "tok-e2e-dedup";
    run_trade(
        &pool, &vault, &book, &pipeline, &data, &app_cfg, "buy", "100", &wallet, token,
    )
    .await;
    let sell = trade("sell", "100", &wallet, token);
    let out = ingest(&pool, &sell).await.unwrap();
    process_event(&pool, &vault, &book, &pipeline, &data, &app_cfg, out.event_id)
        .await
        .unwrap();
    let out2 = ingest(&pool, &sell).await.unwrap();
    assert!(!out2.created || out2.duplicate_fragment);
    if out2.created {
        let second = process_event(
            &pool,
            &vault,
            &book,
            &pipeline,
            &data,
            &app_cfg,
            out2.event_id,
        )
        .await;
        assert!(second.is_ok());
    }
    let mut tx = begin_as(&pool, Some(user_id), "user").await.unwrap();
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM position_close_events WHERE position_id IN (
            SELECT id FROM positions WHERE strategy_id = $1 AND token_id = $2)",
    )
    .bind(strategy_id)
    .bind(token)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(n, 1);
}
