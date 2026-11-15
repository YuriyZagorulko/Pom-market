//! Race tests: strategy delete vs capital sync / fund / withdraw (pommarket_test).

mod common;
#[path = "helpers/strategy_delete_harness.rs"]
mod strategy_delete_harness;
mod test_support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::TestApi;
use pommarket::db::begin_as;
use pommarket::strategy_accounting::{
    lock_owned_copy_strategy, reconcile_account, sync_strategy_capital_to_configured_cap,
    unallocated_cash_in_tx, StrategyCapitalError,
};
use pommarket::strategy_delete::delete_strategy_for_user;
use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde_json::json;
use strategy_delete_harness::{delete_http, seed_user_strategy};
use uuid::Uuid;

async fn ensure_paper_account(api: &TestApi, user_id: Uuid, account: Uuid) {
    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    sqlx::query(
        "UPDATE trading_accounts SET operating_mode = 'paper_trading' WHERE id = $1 AND user_id = $2",
    )
    .bind(account)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

async fn disable_strategy(api: &TestApi, token: &str, strategy_id: Uuid) {
    api.send(
        "POST",
        &format!("/api/v1/strategies/{strategy_id}/enable"),
        token,
        json!({ "enabled": false }),
    )
    .await;
}

async fn enable_strategy(api: &TestApi, token: &str, user_id: Uuid, account: Uuid, strategy_id: Uuid) {
    ensure_paper_account(api, user_id, account).await;
    api.send(
        "POST",
        &format!("/api/v1/strategies/{strategy_id}/enable"),
        token,
        json!({ "enabled": true }),
    )
    .await;
}

async fn capital_sync_http(api: &TestApi, token: &str, strategy_id: Uuid) -> StatusCode {
    let (status, _) = api
        .request_json(
            "POST",
            &format!("/api/v1/strategies/{strategy_id}/capital/sync-configured"),
            Some(token),
            Some(json!({})),
        )
        .await;
    status
}

async fn unalloc(api: &TestApi, user_id: Uuid, account: Uuid) -> Decimal {
    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let u = unallocated_cash_in_tx(&mut tx, account).await.unwrap();
    tx.commit().await.unwrap();
    u
}

async fn plant_decision_bulk(
    worker: &sqlx::PgPool,
    user_id: Uuid,
    account: Uuid,
    strategy_id: Uuid,
    count: usize,
) {
    let mut tx = begin_as(worker, Some(user_id), "worker").await.unwrap();
    let (trader_id, wallet): (Uuid, String) = sqlx::query_as(
        "SELECT st.id, st.wallet_address FROM copy_strategies s
         JOIN source_traders st ON st.id = s.source_trader_id WHERE s.id = $1",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    for i in 0..count {
        let dedup = format!("race-bulk-{strategy_id}-{i}");
        let event_id: Uuid = sqlx::query_scalar(
            "INSERT INTO source_events (dedup_key, source_wallet, token_id, side, price, size_shares, notional_usd, event_ts, origin, status, raw)
             VALUES ($1, $2, 'tok-bulk', 'buy', 0.5, 1, 0.5, now(), 'test', 'processed', '{}'::jsonb)
             RETURNING id",
        )
        .bind(&dedup)
        .bind(&wallet)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO copy_decisions (user_id, trading_account_id, source_trader_id, source_event_id, strategy_id, status, reason, operating_mode, side, strategy_snapshot, checks, detected_at)
             VALUES ($1,$2,$3,$4,$5,'skipped','bulk','paper_trading','buy','{}','{}', now())",
        )
        .bind(user_id)
        .bind(account)
        .bind(trader_id)
        .bind(event_id)
        .bind(strategy_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn delete_vs_capital_sync_in_process_no_500() {
    let api = Arc::new(TestApi::boot().await);
    let email = format!("race-sync-{}@example.com", Uuid::new_v4());
    let (token, user_id, account, _, strategy) = seed_user_strategy(&api, &email).await;
    enable_strategy(&api, &token, user_id, account, strategy).await;

    let worker = api.worker.clone();
    let strategy_c = strategy;
    let user_c = user_id;
    let hold = tokio::spawn(async move {
        let mut tx = begin_as(&worker, Some(user_c), "user").await.unwrap();
        lock_owned_copy_strategy(&mut tx, user_c, strategy_c, false)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        tx.rollback().await.ok();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    let status = capital_sync_http(&api, &token, strategy).await;
    let _ = hold.await;
    assert_ne!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "capital sync must not 500 when strategy lock contended"
    );

    disable_strategy(&api, &token, strategy).await;
    let unalloc_before = unalloc(&api, user_id, account).await;
    let (del_status, _) = delete_http(&api, &token, strategy).await;
    assert_eq!(del_status, StatusCode::OK);
    let unalloc_after = unalloc(&api, user_id, account).await;
    assert!(unalloc_after >= unalloc_before);
    let reconcile = reconcile_account(&api.worker, account).await.unwrap();
    assert!(reconcile.reconciled);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn capital_sync_after_delete_returns_not_found() {
    let api = TestApi::boot().await;
    let email = format!("race-after-{}@example.com", Uuid::new_v4());
    let (token, user_id, account, _, strategy) = seed_user_strategy(&api, &email).await;
    disable_strategy(&api, &token, strategy).await;

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let avail: Decimal = sqlx::query_scalar(
        "SELECT available_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
  let unalloc_before = unallocated_cash_in_tx(&mut tx, account).await.unwrap();
    tx.commit().await.unwrap();

    let (del_status, _) = delete_http(&api, &token, strategy).await;
    assert_eq!(del_status, StatusCode::OK);

    let status = capital_sync_http(&api, &token, strategy).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let unalloc_after = unalloc(&api, user_id, account).await;
    assert_eq!(
        unalloc_after,
        unalloc_before + avail,
        "delete returns cash once; sync after delete must not move more"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn double_delete_second_is_conflict_or_not_found() {
    let api = Arc::new(TestApi::boot().await);
    let email = format!("race-2del-{}@example.com", Uuid::new_v4());
    let (token, user_id, account, _, strategy) = seed_user_strategy(&api, &email).await;
    disable_strategy(&api, &token, strategy).await;

    let unalloc_before = unalloc(&api, user_id, account).await;
    let path = format!("/api/v1/strategies/{strategy}");
    let client_a = api.http.clone();
    let client_b = api.http.clone();
    let base_a = api.base.clone();
    let base_b = api.base.clone();
    let path_a = path.clone();
    let path_b = path;
    let t1 = token.clone();
    let t2 = token;

    let (r1, r2) = tokio::join!(
        async move {
            client_a
                .delete(format!("{base_a}{path_a}"))
                .header("authorization", format!("Bearer {t1}"))
                .send()
                .await
                .unwrap()
                .status()
        },
        async move {
            client_b
                .delete(format!("{base_b}{path_b}"))
                .header("authorization", format!("Bearer {t2}"))
                .send()
                .await
                .unwrap()
                .status()
        },
    );

    let ok = |s: StatusCode| s == StatusCode::OK;
    let second_ok = |s: StatusCode| {
        s == StatusCode::CONFLICT || s == StatusCode::NOT_FOUND
    };
    assert!(
        (ok(r1) && second_ok(r2)) || (ok(r2) && second_ok(r1)),
        "r1={r1} r2={r2}"
    );
    let unalloc_after = unalloc(&api, user_id, account).await;
    assert!(unalloc_after >= unalloc_before);
    let reconcile = reconcile_account(&api.worker, account).await.unwrap();
    assert!(reconcile.reconciled);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn delete_vs_allocate_is_consistent() {
    let api = Arc::new(TestApi::boot().await);
    let email = format!("race-alloc-{}@example.com", Uuid::new_v4());
    let (token, user_id, account, _, strategy) = seed_user_strategy(&api, &email).await;
    disable_strategy(&api, &token, strategy).await;

    let client = api.http.clone();
    let base = api.base.clone();
    let token_d = token.clone();
    let token_a = token.clone();
    let path_del = format!("/api/v1/strategies/{strategy}");
    let path_alloc = format!("/api/v1/strategies/{strategy}/capital/allocate");
    let api_alloc = Arc::clone(&api);

    let (del_s, alloc_s) = tokio::join!(
        async move {
            client
                .delete(format!("{base}{path_del}"))
                .header("authorization", format!("Bearer {token_d}"))
                .send()
                .await
                .unwrap()
                .status()
        },
        async move {
            api_alloc
                .request_json(
                    "POST",
                    &path_alloc,
                    Some(&token_a),
                    Some(json!({ "amount_usd": "10" })),
                )
                .await
                .0
        },
    );

    assert_ne!(del_s, StatusCode::INTERNAL_SERVER_ERROR);
    assert_ne!(alloc_s, StatusCode::INTERNAL_SERVER_ERROR);
    let reconcile = reconcile_account(&api.worker, account).await.unwrap();
    assert!(reconcile.reconciled);
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM copy_strategies WHERE id = $1)")
        .bind(strategy)
        .fetch_one(&api.worker)
        .await
        .unwrap();
    if del_s.is_success() {
        assert!(!exists);
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn delete_vs_withdraw_is_consistent() {
    let api = Arc::new(TestApi::boot().await);
    let email = format!("race-wd-{}@example.com", Uuid::new_v4());
    let (token, user_id, account, _, strategy) = seed_user_strategy(&api, &email).await;
    disable_strategy(&api, &token, strategy).await;

    let client = api.http.clone();
    let base = api.base.clone();
    let token_d = token.clone();
    let token_w = token.clone();
    let path_del = format!("/api/v1/strategies/{strategy}");
    let path_w = format!("/api/v1/strategies/{strategy}/capital/withdraw");
    let api_wd = Arc::clone(&api);

    let (del_s, wd_s) = tokio::join!(
        async move {
            client
                .delete(format!("{base}{path_del}"))
                .header("authorization", format!("Bearer {token_d}"))
                .send()
                .await
                .unwrap()
                .status()
        },
        async move {
            api_wd
                .request_json(
                    "POST",
                    &path_w,
                    Some(&token_w),
                    Some(json!({ "amount_usd": "5" })),
                )
                .await
                .0
        },
    );

    assert_ne!(del_s, StatusCode::INTERNAL_SERVER_ERROR);
    assert_ne!(wd_s, StatusCode::INTERNAL_SERVER_ERROR);
    let reconcile = reconcile_account(&api.worker, account).await.unwrap();
    assert!(reconcile.reconciled);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn delete_does_not_affect_sibling_strategy_capital() {
    let api = TestApi::boot().await;
    let email = format!("race-sib-{}@example.com", Uuid::new_v4());
    let (token, user_id, account, _, strategy_a) = seed_user_strategy(&api, &email).await;
    let strategy_b = strategy_delete_extended_seed_b(&api, &token, user_id, account).await;
    ensure_paper_account(&api, user_id, account).await;
    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let trader_b: Uuid = sqlx::query_scalar(
        "SELECT source_trader_id FROM copy_strategies WHERE id = $1",
    )
    .bind(strategy_b)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    api.send(
        "PATCH",
        &format!("/api/v1/source-traders/{trader_b}"),
        &token,
        json!({ "monitoring_enabled": true }),
    )
    .await;
    enable_strategy(&api, &token, user_id, account, strategy_b).await;
    disable_strategy(&api, &token, strategy_a).await;

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let funded_b: Decimal = sqlx::query_scalar(
        "SELECT funded_capital_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_b)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let status = capital_sync_http(&api, &token, strategy_b).await;
    assert!(status.is_success() || status == StatusCode::BAD_REQUEST);

    let (del_status, _) = delete_http(&api, &token, strategy_a).await;
    assert_eq!(del_status, StatusCode::OK);

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let funded_after: Decimal = sqlx::query_scalar(
        "SELECT funded_capital_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_b)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(funded_b, funded_after);

    let status_b = capital_sync_http(&api, &token, strategy_b).await;
    assert_ne!(status_b, StatusCode::INTERNAL_SERVER_ERROR);
    let reconcile = reconcile_account(&api.worker, account).await.unwrap();
    assert!(reconcile.reconciled);
}

async fn strategy_delete_extended_seed_b(
    api: &TestApi,
    token: &str,
    user_id: Uuid,
    account: Uuid,
) -> Uuid {
    use pommarket::strategy_accounting::allocate_to_strategy;
    let trader_b: Uuid = Uuid::parse_str(
        api
            .send(
                "POST",
                "/api/v1/source-traders",
                token,
                json!({
                    "alias": "LeaderRaceB",
                    "wallet_address": format!("0x{:040x}", Uuid::new_v4().as_u128())
                }),
            )
            .await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let strategy_b: Uuid = Uuid::parse_str(
        api
            .send(
                "POST",
                "/api/v1/strategies",
                token,
                json!({
                    "trading_account_id": account,
                    "source_trader_id": trader_b,
                    "display_name": "SiblingB",
                    "sizing_mode": "proportional",
                    "proportion": "0.1",
                    "strategy_budget_usd": "300"
                }),
            )
            .await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    pommarket::strategy_accounting::ensure_strategy_capital_row(&mut tx, user_id, account, strategy_b)
        .await
        .unwrap();
    allocate_to_strategy(
        &mut tx,
        user_id,
        account,
        strategy_b,
        Decimal::from(80),
        &format!("race-b-alloc-{strategy_b}"),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    strategy_b
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn concurrent_delete_and_sync_lib_no_missing_row_panic() {
    let api = Arc::new(TestApi::boot().await);
    let email = format!("race-lib-{}@example.com", Uuid::new_v4());
    let (token, _user_id, _account, _, strategy) = seed_user_strategy(&api, &email).await;
    disable_strategy(&api, &token, strategy).await;

    let worker = api.worker.clone();
    let worker2 = api.worker.clone();
    let strategy_c = strategy;
    let user_del = _user_id;
    let user_sync = _user_id;

    let (del_res, sync_res) = tokio::join!(
        async move {
            let mut tx = begin_as(&worker, Some(user_del), "user").await.unwrap();
            let r = delete_strategy_for_user(&mut tx, user_del, strategy_c).await;
            if r.is_ok() {
                tx.commit().await.unwrap();
            } else {
                tx.rollback().await.ok();
            }
            r
        },
        async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let mut tx = begin_as(&worker2, Some(user_sync), "user").await.unwrap();
            let r =
                sync_strategy_capital_to_configured_cap(&mut tx, user_sync, strategy_c, "race-lib")
                    .await;
            if r.is_ok() {
                tx.commit().await.ok();
            } else {
                tx.rollback().await.ok();
            }
            r
        },
    );

    assert!(del_res.is_ok() || matches!(sync_res, Err(StrategyCapitalError::StrategyNotFound)));
    if let Err(e) = sync_res {
        assert!(
            matches!(
                e,
                StrategyCapitalError::StrategyNotFound
                    | StrategyCapitalError::CapitalAccountMissing
                    | StrategyCapitalError::StrategyBusy
                    | StrategyCapitalError::Business(_)
            ),
            "unexpected sync error: {e:?}"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn delete_large_history_timing_smoke() {
    let api = TestApi::boot().await;
    let email = format!("race-perf-{}@example.com", Uuid::new_v4());
    let (token, user_id, account, _, strategy) = seed_user_strategy(&api, &email).await;
    plant_decision_bulk(&api.worker, user_id, account, strategy, 400).await;
    disable_strategy(&api, &token, strategy).await;

    let start = Instant::now();
    let (status, _) = delete_http(&api, &token, strategy).await;
    let elapsed = start.elapsed();
    assert_eq!(status, StatusCode::OK);
    eprintln!(
        "delete_large_history_timing_smoke: {} decisions, delete took {:?}",
        400,
        elapsed
    );
    assert!(elapsed < Duration::from_secs(120));
    let reconcile = reconcile_account(&api.worker, account).await.unwrap();
    assert!(reconcile.reconciled);
}
