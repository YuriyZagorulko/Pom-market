//! PAPER resolution settlement integration (pommarket_test + wiremock CLOB).

mod test_support;

use pommarket::db::{begin_as, connect, migrate};
use pommarket::paper_settlement::{self, SettlementResult};
use pommarket::polymarket::http::PublicClient;
use pommarket::strategy_accounting::{allocate_to_strategy, ensure_strategy_capital_row};
use rust_decimal::Decimal;
use test_support::assert_all_test_database_urls;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn seed(min_funded: Decimal) -> (sqlx::PgPool, Uuid, Uuid, Uuid) {
    let url = std::env::var("MIGRATION_DATABASE_URL").unwrap();
    let pool = connect(&url).await.unwrap();
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ($1,'Settle','x','user','active',now()) RETURNING id",
    )
    .bind(format!("settle-{}@example.com", Uuid::new_v4()))
    .fetch_one(&pool)
    .await
    .unwrap();
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signature_type, signer_address, funder_address, operating_mode, status)
         VALUES ($1,'S','eoa','0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc','0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc','paper_trading','connected')
         RETURNING id",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_risk_limits (trading_account_id, user_id, max_order_usd, max_daily_loss_usd, max_exposure_usd, max_slippage_bps, max_orders_per_interval, interval_seconds, max_stale_data_seconds, paper_cash_usd)
         VALUES ($1,$2,500,1000,5000,150,20,60,120,10000)",
    )
    .bind(account)
    .bind(user)
    .execute(&pool)
    .await
    .unwrap();
    let trader: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias) VALUES ($1,$2,'T') RETURNING id",
    )
    .bind(user)
    .bind("0xcccccccccccccccccccccccccccccccccccccccc")
    .fetch_one(&pool)
    .await
    .unwrap();
    let strategy: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (user_id, trading_account_id, source_trader_id, enabled, sizing_mode, proportion, strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions, max_slippage_bps, min_order_usd, copy_buy, copy_sell)
         VALUES ($1,$2,$3,false,'proportional',0.1,500,200,1000,10,150,1,true,true) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(trader)
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    ensure_strategy_capital_row(&mut tx, user, account, strategy)
        .await
        .unwrap();
    allocate_to_strategy(
        &mut tx,
        user,
        account,
        strategy,
        min_funded,
        &format!("seed-{}", Uuid::new_v4()),
    )
    .await
    .unwrap();
    sqlx::query(
        "UPDATE strategy_capital_accounts SET invested_capital_usd = $2 WHERE strategy_id = $1",
    )
    .bind(strategy)
    .bind(Decimal::ZERO)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    (pool, user, account, strategy)
}

async fn insert_position(
    pool: &sqlx::PgPool,
    user: Uuid,
    account: Uuid,
    strategy: Uuid,
    token_id: &str,
    condition_id: &str,
    outcome: &str,
    shares: Decimal,
    cost: Decimal,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode,
         condition_id, outcome, market_title, open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,$3,'0xwallet',$4,'paper_trading',$5,$6,'Test market',$7,$8,$9) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(strategy)
    .bind(token_id)
    .bind(condition_id)
    .bind(outcome)
    .bind(shares)
    .bind(cost / shares)
    .bind(cost)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn client_for(clob_base: &str) -> PublicClient {
    PublicClient::new(
        "http://127.0.0.1:1".into(),
        clob_base.into(),
        "http://127.0.0.1:1".into(),
    )
}

async fn mock_resolved_market(
    server: &MockServer,
    condition_id: &str,
    token_win: &str,
    token_lose: &str,
    win_outcome: &str,
    lose_outcome: &str,
) {
    let cid = condition_id.trim_start_matches("0x");
    let body = serde_json::json!({
        "closed": true,
        "tokens": [
            {"token_id": token_lose, "outcome": lose_outcome, "winner": false, "price": 0},
            {"token_id": token_win, "outcome": win_outcome, "winner": true, "price": 1}
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
async fn resolution_win_credits_available_and_closes_position() {
    assert_all_test_database_urls();
    migrate(&std::env::var("MIGRATION_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let server = MockServer::start().await;
    let condition = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    mock_resolved_market(
        &server,
        condition,
        "tok-yes",
        "tok-no",
        "Yes",
        "No",
    )
    .await;
    let client = client_for(&server.uri());
    let (pool, user, account, strategy) = seed(Decimal::from(500)).await;
    let pos = insert_position(
        &pool,
        user,
        account,
        strategy,
        "tok-yes",
        condition,
        "Yes",
        Decimal::from(10),
        Decimal::from(6),
    )
    .await;
    sqlx::query("UPDATE strategy_capital_accounts SET invested_capital_usd = 6 WHERE strategy_id = $1")
        .bind(strategy)
        .execute(&pool)
        .await
        .unwrap();
    let avail_before: Decimal = sqlx::query_scalar(
        "SELECT available_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy)
    .fetch_one(&pool)
    .await
    .unwrap();

    let r = paper_settlement::try_settle_position(&pool, &client, pos)
        .await
        .unwrap()
        .expect("settlement");
    assert_eq!(r.status, "resolved_win");
    assert_eq!(r.payout_usd, Decimal::from(10));

    let open: Decimal = sqlx::query_scalar("SELECT open_shares FROM positions WHERE id = $1")
        .bind(pos)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(open, Decimal::ZERO);
    let avail: Decimal = sqlx::query_scalar(
        "SELECT available_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(avail, avail_before + Decimal::from(10));
    let invested: Decimal = sqlx::query_scalar(
        "SELECT invested_capital_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(invested, Decimal::ZERO);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn resolution_loss_zero_payout_releases_invested() {
    assert_all_test_database_urls();
    migrate(&std::env::var("MIGRATION_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let server = MockServer::start().await;
    let condition = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    mock_resolved_market(
        &server,
        condition,
        "tok-yes",
        "tok-no",
        "Yes",
        "No",
    )
    .await;
    let client = client_for(&server.uri());
    let (pool, user, account, strategy) = seed(Decimal::from(500)).await;
    let pos = insert_position(
        &pool,
        user,
        account,
        strategy,
        "tok-no",
        condition,
        "No",
        Decimal::from(5),
        Decimal::new(25, 1),
    )
    .await;
    sqlx::query("UPDATE strategy_capital_accounts SET invested_capital_usd = 2.5 WHERE strategy_id = $1")
        .bind(strategy)
        .execute(&pool)
        .await
        .unwrap();
    let avail_before: Decimal = sqlx::query_scalar(
        "SELECT available_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy)
    .fetch_one(&pool)
    .await
    .unwrap();

    let r = paper_settlement::try_settle_position(&pool, &client, pos)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.status, "resolved_loss");
    assert_eq!(r.payout_usd, Decimal::ZERO);
    let avail: Decimal = sqlx::query_scalar(
        "SELECT available_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(avail, avail_before);
    let ledger: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM capital_ledger_entries WHERE position_id = $1 AND entry_type = 'resolution_loss'",
    )
    .bind(pos)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ledger, 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn active_market_skips_settlement() {
    assert_all_test_database_urls();
    migrate(&std::env::var("MIGRATION_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let server = MockServer::start().await;
    let condition = "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    let body = serde_json::json!({
        "closed": false,
        "tokens": [{"token_id": "tok-a", "outcome": "Yes", "winner": false, "price": 0.5}]
    });
    Mock::given(method("GET"))
        .and(path("/markets/0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = client_for(&server.uri());
    let (pool, user, account, strategy) = seed(Decimal::from(200)).await;
    let pos = insert_position(
        &pool,
        user,
        account,
        strategy,
        "tok-a",
        condition,
        "Yes",
        Decimal::ONE,
        Decimal::ONE,
    )
    .await;
    let r = paper_settlement::try_settle_position(&pool, &client, pos)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.status, "active_market");
    let open: Decimal = sqlx::query_scalar("SELECT open_shares FROM positions WHERE id = $1")
        .bind(pos)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(open, Decimal::ONE);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn partial_sell_then_resolution_pays_remaining_shares_only() {
    assert_all_test_database_urls();
    migrate(&std::env::var("MIGRATION_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let server = MockServer::start().await;
    let condition = "0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    mock_resolved_market(
        &server,
        condition,
        "tok-yes",
        "tok-no",
        "Yes",
        "No",
    )
    .await;
    let client = client_for(&server.uri());
    let (pool, user, account, strategy) = seed(Decimal::from(500)).await;
    let pos = insert_position(
        &pool,
        user,
        account,
        strategy,
        "tok-yes",
        condition,
        "Yes",
        Decimal::from(10),
        Decimal::from(8),
    )
    .await;
    sqlx::query(
        "UPDATE positions SET open_shares = 4, cost_basis_usd = 3.2 WHERE id = $1",
    )
    .bind(pos)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE strategy_capital_accounts SET invested_capital_usd = 3.2 WHERE strategy_id = $1")
        .bind(strategy)
        .execute(&pool)
        .await
        .unwrap();
    // Remaining 4 shares @ $0.80 cost simulates an earlier partial exit (6 shares sold elsewhere).

    let r = paper_settlement::try_settle_position(&pool, &client, pos)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.status, "resolved_win");
    assert_eq!(r.payout_usd, Decimal::from(4));
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn settlement_idempotent_on_worker_rerun() {
    assert_all_test_database_urls();
    migrate(&std::env::var("MIGRATION_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let server = MockServer::start().await;
    let condition = "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    mock_resolved_market(
        &server,
        condition,
        "win-tok",
        "lose-tok",
        "Yes",
        "No",
    )
    .await;
    let client = client_for(&server.uri());
    let (pool, user, account, strategy) = seed(Decimal::from(300)).await;
    let pos = insert_position(
        &pool,
        user,
        account,
        strategy,
        "win-tok",
        condition,
        "Yes",
        Decimal::from(2),
        Decimal::from(2),
    )
    .await;
    sqlx::query("UPDATE strategy_capital_accounts SET invested_capital_usd = 2 WHERE strategy_id = $1")
        .bind(strategy)
        .execute(&pool)
        .await
        .unwrap();
    paper_settlement::process_open_positions(&pool, &client, 10)
        .await
        .unwrap();
    let avail1: Decimal = sqlx::query_scalar(
        "SELECT available_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy)
    .fetch_one(&pool)
    .await
    .unwrap();
    let ledgers1: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM capital_ledger_entries WHERE position_id = $1",
    )
    .bind(pos)
    .fetch_one(&pool)
    .await
    .unwrap();
    paper_settlement::process_open_positions(&pool, &client, 10)
        .await
        .unwrap();
    let avail2: Decimal = sqlx::query_scalar(
        "SELECT available_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy)
    .fetch_one(&pool)
    .await
    .unwrap();
    let ledgers2: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM capital_ledger_entries WHERE position_id = $1",
    )
    .bind(pos)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(avail1, avail2);
    assert_eq!(ledgers1, ledgers2);
    assert_eq!(ledgers2, 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn two_strategies_settle_independently() {
    assert_all_test_database_urls();
    migrate(&std::env::var("MIGRATION_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let server = MockServer::start().await;
    let cond_a = "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
    let cond_b = "0x1111111111111111111111111111111111111111111111111111111111111111";
    mock_resolved_market(&server, cond_a, "a-win", "a-lose", "Yes", "No").await;
    mock_resolved_market(&server, cond_b, "b-win", "b-lose", "Up", "Down").await;
    let client = client_for(&server.uri());
    let (pool, user, account, strategy_a) = seed(Decimal::from(400)).await;
    let trader_b: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias) VALUES ($1,$2,'T2') RETURNING id",
    )
    .bind(user)
    .bind("0xddddddddddddddddddddddddddddddddddddddde")
    .fetch_one(&pool)
    .await
    .unwrap();
    let strategy_b: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (user_id, trading_account_id, source_trader_id, enabled, sizing_mode, proportion, strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions, max_slippage_bps, min_order_usd, copy_buy, copy_sell)
         VALUES ($1,$2,$3,false,'proportional',0.1,400,200,1000,10,150,1,true,true) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(trader_b)
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut tx = begin_as(&pool, Some(user), "user").await.unwrap();
    ensure_strategy_capital_row(&mut tx, user, account, strategy_b)
        .await
        .unwrap();
    allocate_to_strategy(
        &mut tx,
        user,
        account,
        strategy_b,
        Decimal::from(400),
        "alloc-b",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let pos_a = insert_position(
        &pool,
        user,
        account,
        strategy_a,
        "a-win",
        cond_a,
        "Yes",
        Decimal::from(3),
        Decimal::from(3),
    )
    .await;
    let pos_b = insert_position(
        &pool,
        user,
        account,
        strategy_b,
        "b-lose",
        cond_b,
        "Down",
        Decimal::from(5),
        Decimal::from(4),
    )
    .await;
    sqlx::query("UPDATE strategy_capital_accounts SET invested_capital_usd = 3 WHERE strategy_id = $1")
        .bind(strategy_a)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE strategy_capital_accounts SET invested_capital_usd = 4 WHERE strategy_id = $1")
        .bind(strategy_b)
        .execute(&pool)
        .await
        .unwrap();

    paper_settlement::try_settle_position(&pool, &client, pos_a)
        .await
        .unwrap();
    paper_settlement::try_settle_position(&pool, &client, pos_b)
        .await
        .unwrap();

    let inv_a: Decimal = sqlx::query_scalar(
        "SELECT invested_capital_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_a)
    .fetch_one(&pool)
    .await
    .unwrap();
    let inv_b: Decimal = sqlx::query_scalar(
        "SELECT invested_capital_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_b)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(inv_a, Decimal::ZERO);
    assert_eq!(inv_b, Decimal::ZERO);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn polymarket_api_error_does_not_close_position() {
    assert_all_test_database_urls();
    migrate(&std::env::var("MIGRATION_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let server = MockServer::start().await;
    let condition = "0x2222222222222222222222222222222222222222222222222222222222222222";
    Mock::given(method("GET"))
        .and(path("/markets/0x2222222222222222222222222222222222222222222222222222222222222222"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let client = client_for(&server.uri());
    let (pool, user, account, strategy) = seed(Decimal::from(100)).await;
    let pos = insert_position(
        &pool,
        user,
        account,
        strategy,
        "tok-x",
        condition,
        "Yes",
        Decimal::from(7),
        Decimal::from(7),
    )
    .await;
    let err = paper_settlement::try_settle_position(&pool, &client, pos)
        .await
        .unwrap_err();
    assert!(!err.to_string().is_empty());
    let open: Decimal = sqlx::query_scalar("SELECT open_shares FROM positions WHERE id = $1")
        .bind(pos)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(open, Decimal::from(7));
    let ledger: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM capital_ledger_entries WHERE position_id = $1",
    )
    .bind(pos)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ledger, 0);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn refund_half_market_settles_at_fifty_cents() {
    assert_all_test_database_urls();
    migrate(&std::env::var("MIGRATION_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let server = MockServer::start().await;
    let condition = "0x3333333333333333333333333333333333333333333333333333333333333333";
    let body = serde_json::json!({
        "closed": true,
        "tokens": [
            {"token_id": "t1", "outcome": "A", "winner": false, "price": 0.5},
            {"token_id": "t2", "outcome": "B", "winner": false, "price": 0.5}
        ]
    });
    Mock::given(method("GET"))
        .and(path("/markets/0x3333333333333333333333333333333333333333333333333333333333333333"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = client_for(&server.uri());
    let (pool, user, account, strategy) = seed(Decimal::from(200)).await;
    let pos = insert_position(
        &pool,
        user,
        account,
        strategy,
        "t1",
        condition,
        "A",
        Decimal::from(10),
        Decimal::from(5),
    )
    .await;
    sqlx::query("UPDATE strategy_capital_accounts SET invested_capital_usd = 5 WHERE strategy_id = $1")
        .bind(strategy)
        .execute(&pool)
        .await
        .unwrap();
    let r: SettlementResult = paper_settlement::try_settle_position(&pool, &client, pos)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.status, "resolved_refund");
    assert_eq!(r.payout_usd, Decimal::from(5));
    let typ: String = sqlx::query_scalar(
        "SELECT entry_type FROM capital_ledger_entries WHERE position_id = $1",
    )
    .bind(pos)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(typ, "resolution_refund");
}
