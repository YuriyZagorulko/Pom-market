//! Extended strategy-delete checks (active orders, finance, isolation, concurrency).

mod common;
#[path = "helpers/strategy_delete_harness.rs"]
mod strategy_delete_harness;
mod test_support;

use std::sync::Arc;

use common::TestApi;
use pommarket::db::begin_as;
use pommarket::strategy_accounting::{
    allocate_to_strategy, ensure_strategy_capital_row, reconcile_account, unallocated_cash_in_tx,
};
use reqwest::Client;
use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde_json::json;
use uuid::Uuid;

use strategy_delete_harness::{delete_http, seed_user_strategy};

const OPEN_ORDER_STATUSES: &[&str] = &[
    "pending",
    "submitting",
    "submitted",
    "partially_filled",
    "unknown",
];

async fn plant_copy_order(
    api: &TestApi,
    user_id: Uuid,
    account: Uuid,
    strategy_id: Uuid,
    order_status: &str,
    with_fill: bool,
) -> Uuid {
    // source_events writes require worker session (RLS); orders/decisions allow worker + app.user_id.
    let mut tx = begin_as(&api.worker, Some(user_id), "worker").await.unwrap();
    let (trader_id, wallet): (Uuid, String) = sqlx::query_as(
        "SELECT st.id, st.wallet_address FROM copy_strategies s
         JOIN source_traders st ON st.id = s.source_trader_id WHERE s.id = $1",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    let dedup = format!("del-test-order-{}-{}", order_status, Uuid::new_v4());
    let event_id: Uuid = sqlx::query_scalar(
        "INSERT INTO source_events (dedup_key, source_wallet, token_id, side, price, size_shares, notional_usd, event_ts, origin, status, raw)
         VALUES ($1, $2, 'tok-del', 'buy', 0.5, 10, 5, now(), 'test', 'processed', '{}'::jsonb)
         RETURNING id",
    )
    .bind(&dedup)
    .bind(&wallet)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    let decision_id: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_decisions (user_id, trading_account_id, source_trader_id, source_event_id, strategy_id, status, reason, operating_mode, side, strategy_snapshot, checks, detected_at)
         VALUES ($1,$2,$3,$4,$5,'approved','approved','paper_trading','buy','{}','{}', now()) RETURNING id",
    )
    .bind(user_id)
    .bind(account)
    .bind(trader_id)
    .bind(event_id)
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    let order_id: Uuid = sqlx::query_scalar(
        "INSERT INTO orders (user_id, trading_account_id, decision_id, source_event_id, client_order_id, operating_mode, order_type, side,
         token_id, limit_price, size_shares, status, origin)
         VALUES ($1,$2,$3,$4,gen_random_uuid(),'paper_trading','GTC','buy','tok-del',0.5,10,$5,'copy')
         RETURNING id",
    )
    .bind(user_id)
    .bind(account)
    .bind(decision_id)
    .bind(event_id)
    .bind(order_status)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    if with_fill {
        sqlx::query(
            "INSERT INTO fills (user_id, trading_account_id, order_id, price, shares, notional_usd, fee_usd, fee_status)
             VALUES ($1,$2,$3,0.5,4,2,0.05,'known_zero')",
        )
        .bind(user_id)
        .bind(account)
        .bind(order_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    }

    tx.commit().await.unwrap();
    order_id
}

async fn strategy_capital_snapshot(
    api: &TestApi,
    user_id: Uuid,
    strategy_id: Uuid,
) -> (Decimal, Decimal, Decimal, Decimal) {
    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let row: (Decimal, Decimal, Decimal, Decimal) = sqlx::query_as(
        "SELECT funded_capital_usd, available_cash_usd, reserved_cash_usd, invested_capital_usd
         FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    row
}

async fn seed_second_strategy(
    api: &TestApi,
    token: &str,
    user_id: Uuid,
    account: Uuid,
    alloc_b: Decimal,
) -> Uuid {
    let trader_b: Uuid = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/source-traders",
            &token,
            json!({
                "alias": "LeaderB",
                "wallet_address": format!("0x{:040x}", Uuid::new_v4().as_u128())
            }),
        )
        .await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let strategy_b: Uuid = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/strategies",
            &token,
            json!({
                "trading_account_id": account,
                "source_trader_id": trader_b,
                "display_name": "Keep",
                "sizing_mode": "proportional",
                "proportion": "0.3",
                "strategy_budget_usd": "400"
            }),
        )
        .await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    ensure_strategy_capital_row(&mut tx, user_id, account, strategy_b)
        .await
        .unwrap();
    allocate_to_strategy(
        &mut tx,
        user_id,
        account,
        strategy_b,
        alloc_b,
        &format!("test-delete-alloc-b-{strategy_b}"),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    strategy_b
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn delete_rejects_each_active_order_status() {
    let api = TestApi::boot().await;
    for status in OPEN_ORDER_STATUSES {
        let email = format!("del-ord-{status}-{}@example.com", Uuid::new_v4());
        let (token, user_id, account, _trader, strategy) = seed_user_strategy(&api, &email).await;
        let with_fill = *status == "partially_filled";
        plant_copy_order(&api, user_id, account, strategy, status, with_fill).await;

        let (http_status, body) = delete_http(&api, &token, strategy).await;
        assert_eq!(
            http_status,
            StatusCode::BAD_REQUEST,
            "status {status}: {body:?}"
        );
        assert!(
            body["error"]
                .as_str()
                .unwrap_or("")
                .contains("open order"),
            "status {status}: {}",
            body["error"].as_str().unwrap_or("")
        );

        let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
        let still_there: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM copy_strategies WHERE id = $1)",
        )
        .bind(strategy)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert!(still_there, "strategy should remain for status {status}");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn delete_rejected_partial_fill_preserves_fills_and_ledger() {
    let api = TestApi::boot().await;
    let email = format!("del-partial-{}@example.com", Uuid::new_v4());
    let (token, user_id, account, _trader, strategy) = seed_user_strategy(&api, &email).await;

    let order_id = plant_copy_order(&api, user_id, account, strategy, "partially_filled", true).await;
    let (funded_before, avail_before, _, _) = strategy_capital_snapshot(&api, user_id, strategy).await;

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let fills_before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM fills WHERE order_id = $1")
            .bind(order_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    let ledger_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM capital_ledger_entries WHERE strategy_id = $1",
    )
    .bind(strategy)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let (http_status, _) = delete_http(&api, &token, strategy).await;
    assert_eq!(http_status, StatusCode::BAD_REQUEST);

    let (funded_after, avail_after, _, _) = strategy_capital_snapshot(&api, user_id, strategy).await;
    assert_eq!(funded_before, funded_after);
    assert_eq!(avail_before, avail_after);

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let fills_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM fills WHERE order_id = $1")
        .bind(order_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(fills_before, fills_after);
    assert_eq!(fills_before, 1);

    let ledger_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM capital_ledger_entries WHERE strategy_id = $1",
    )
    .bind(strategy)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(ledger_before, ledger_after);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn delete_financial_integrity_and_sibling_strategy_unchanged() {
    let api = TestApi::boot().await;
    let email = format!("del-fin-{}@example.com", Uuid::new_v4());
    let (token, user_id, account, _trader_a, strategy_a) = seed_user_strategy(&api, &email).await;
    let strategy_b = seed_second_strategy(&api, &token, user_id, account, Decimal::from(150)).await;

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let paper_before: Decimal = sqlx::query_scalar(
        "SELECT paper_cash_usd FROM account_risk_limits WHERE trading_account_id = $1",
    )
    .bind(account)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let unalloc_before = unallocated_cash_in_tx(&mut tx, account).await.unwrap();
    tx.commit().await.unwrap();

    let cap_b_before = strategy_capital_snapshot(&api, user_id, strategy_b).await;
    let (_, avail_a_before, _, _) = strategy_capital_snapshot(&api, user_id, strategy_a).await;

    let (http_status, _) = delete_http(&api, &token, strategy_a).await;
    assert_eq!(http_status, StatusCode::OK);

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let paper_after: Decimal = sqlx::query_scalar(
        "SELECT paper_cash_usd FROM account_risk_limits WHERE trading_account_id = $1",
    )
    .bind(account)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let unalloc_after = unallocated_cash_in_tx(&mut tx, account).await.unwrap();
    tx.commit().await.unwrap();

    assert_eq!(paper_before, paper_after);
    assert_eq!(
        unalloc_after,
        unalloc_before + avail_a_before,
        "freed strategy cash should land in unallocated"
    );

    let cap_b_after = strategy_capital_snapshot(&api, user_id, strategy_b).await;
    assert_eq!(cap_b_before, cap_b_after);

    let reconcile = reconcile_account(&api.worker, account).await.unwrap();
    assert!(reconcile.reconciled, "discrepancy {}", reconcile.discrepancy_usd);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn delete_isolation_source_events_and_sibling_history() {
    let api = TestApi::boot().await;
    let email = format!("del-iso-{}@example.com", Uuid::new_v4());
    let (token, user_id, account, trader_a, strategy_a) = seed_user_strategy(&api, &email).await;
    let strategy_b = seed_second_strategy(&api, &token, user_id, account, Decimal::from(100)).await;

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let wallet_a: String = sqlx::query_scalar(
        "SELECT wallet_address FROM source_traders WHERE id = $1",
    )
    .bind(trader_a)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let mut tx = begin_as(&api.worker, Some(user_id), "worker").await.unwrap();
    sqlx::query(
        "INSERT INTO source_events (dedup_key, source_wallet, token_id, side, price, size_shares, notional_usd, event_ts, origin, status, raw)
         VALUES ('iso-keep-event', $1, 'tok-iso', 'buy', 0.4, 5, 2, now(), 'test', 'processed', '{}'::jsonb)",
    )
    .bind(&wallet_a)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    plant_copy_order(&api, user_id, account, strategy_b, "filled", true).await;

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let source_events_before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM source_events WHERE source_wallet = $1")
            .bind(&wallet_a)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    let decisions_b_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM copy_decisions WHERE strategy_id = $1",
    )
    .bind(strategy_b)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let (http_status, _) = delete_http(&api, &token, strategy_a).await;
    assert_eq!(http_status, StatusCode::OK);

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let source_events_after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM source_events WHERE source_wallet = $1")
            .bind(&wallet_a)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(source_events_before, source_events_after);

    let decisions_b_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM copy_decisions WHERE strategy_id = $1",
    )
    .bind(strategy_b)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(decisions_b_before, decisions_b_after);
    tx.commit().await.unwrap();

    let strategy_new: Uuid = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/strategies",
            &token,
            json!({
                "trading_account_id": account,
                "source_trader_id": trader_a,
                "display_name": "FreshSameTrader",
                "sizing_mode": "proportional",
                "proportion": "0.1",
                "strategy_budget_usd": "200"
            }),
        )
        .await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let new_decisions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM copy_decisions WHERE strategy_id = $1",
    )
    .bind(strategy_new)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(new_decisions, 0);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn concurrent_double_delete_returns_capital_once() {
    let api = Arc::new(TestApi::boot().await);
    let email = format!("del-conc-{}@example.com", Uuid::new_v4());
    let (token, user_id, account, _trader, strategy) = seed_user_strategy(&api, &email).await;

    let (_, avail_before, _, _) = strategy_capital_snapshot(&api, user_id, strategy).await;

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let unalloc_before = unallocated_cash_in_tx(&mut tx, account).await.unwrap();
    tx.commit().await.unwrap();

    let path = format!("/api/v1/strategies/{strategy}");
    let token_a = token.clone();
    let token_b = token.clone();
    let client_a = api.http.clone();
    let client_b = api.http.clone();
    let base_a = api.base.clone();
    let base_b = api.base.clone();
    let path_a = path.clone();
    let path_b = path;

    let (r1, r2) = tokio::join!(
        async move { delete_http_client(&client_a, &base_a, &path_a, &token_a).await },
        async move { delete_http_client(&client_b, &base_b, &path_b, &token_b).await },
    );

    let successes = [r1.0.is_success(), r2.0.is_success()]
        .iter()
        .filter(|&&x| x)
        .count();
    let predictable_second = |s: StatusCode| {
        s == StatusCode::CONFLICT || s == StatusCode::NOT_FOUND || !s.is_success()
    };
    assert!(
        successes <= 1,
        "r1={:?} r2={:?}",
        r1.0,
        r2.0
    );
    if successes == 1 {
        let other = if r1.0.is_success() { r2.0 } else { r1.0 };
        assert!(
            predictable_second(other),
            "second delete should be 409/404, got {other}"
        );
    }

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let unalloc_after = unallocated_cash_in_tx(&mut tx, account).await.unwrap();
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM copy_strategies WHERE id = $1)")
        .bind(strategy)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert!(!exists);
    assert_eq!(
        unalloc_after,
        unalloc_before + avail_before,
        "capital must not be returned twice"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn concurrent_delete_vs_open_position_is_consistent() {
    let api = Arc::new(TestApi::boot().await);
    let email = format!("del-race-{}@example.com", Uuid::new_v4());
    let (token, user_id, account, _trader, strategy) = seed_user_strategy(&api, &email).await;

    let client = api.http.clone();
    let base = api.base.clone();
    let path = format!("/api/v1/strategies/{strategy}");
    let worker = api.worker.clone();
    let token_del = token.clone();

    let delete_task = tokio::spawn(async move {
        delete_http_client(&client, &base, &path, &token_del).await
    });

    let position_task = tokio::spawn(async move {
        let mut tx = begin_as(&worker, Some(user_id), "user").await.unwrap();
        let res = sqlx::query(
            "INSERT INTO positions (
                user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode,
                open_shares, avg_price, cost_basis_usd
             ) VALUES ($1,$2,$3,'0xrace','tok-race','paper_trading',1,0.5,0.5)",
        )
        .bind(user_id)
        .bind(account)
        .bind(strategy)
        .execute(&mut *tx)
        .await;
        if res.is_ok() {
            tx.commit().await.ok();
            "position_inserted"
        } else {
            tx.rollback().await.ok();
            "position_failed"
        }
    });

    let (del_res, pos_res) = tokio::join!(delete_task, position_task);
    let (del_status, _) = del_res.unwrap();

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM copy_strategies WHERE id = $1)")
        .bind(strategy)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let open_pos: i64 = if exists {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM positions WHERE strategy_id = $1 AND open_shares > 0",
        )
        .bind(strategy)
        .fetch_one(&mut *tx)
        .await
        .unwrap()
    } else {
        0
    };
    tx.commit().await.unwrap();

    if del_status.is_success() {
        assert!(!exists);
        assert_eq!(open_pos, 0);
    } else {
        assert!(exists);
        if pos_res.unwrap() == "position_inserted" {
            assert!(open_pos >= 1);
        }
    }
}

async fn delete_http_client(
    client: &Client,
    base: &str,
    path: &str,
    token: &str,
) -> (StatusCode, serde_json::Value) {
    let response = client
        .delete(format!("{base}{path}"))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    let body = if text.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }))
    };
    (status, body)
}
