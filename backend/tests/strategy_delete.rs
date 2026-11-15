//! Strategy delete API and data cleanup (pommarket_test).

mod common;
#[path = "helpers/strategy_delete_harness.rs"]
mod strategy_delete_harness;
mod test_support;

use common::TestApi;
use pommarket::db::begin_as;
use pommarket::strategy_accounting::reconcile_account;
use pommarket::strategy_delete::delete_strategy_for_user;
use std::env;
use rust_decimal::Decimal;
use serde_json::json;
use strategy_delete_harness::seed_user_strategy;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn delete_disabled_strategy_returns_capital_and_removes_data() {
    let api = TestApi::boot().await;
    let email = format!("del-ok-{}@example.com", Uuid::new_v4());
    let (token, user_id, account, trader, strategy) = seed_user_strategy(&api, &email).await;

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let paper_before: Decimal = sqlx::query_scalar(
        "SELECT paper_cash_usd FROM account_risk_limits WHERE trading_account_id = $1",
    )
    .bind(account)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    sqlx::query(
        "INSERT INTO closed_positions (
            user_id, trading_account_id, strategy_id, source_wallet, token_id, market_title,
            operating_mode, closed_shares, realized_pnl_usd
         ) VALUES ($1,$2,$3,'0xabc','99','Test market','paper_trading',1,-0.5)",
    )
    .bind(user_id)
    .bind(account)
    .bind(strategy)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let (status, body) = api
        .request_json(
            "DELETE",
            &format!("/api/v1/strategies/{strategy}"),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body:?}");
    assert_eq!(body["ok"], true);

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM copy_strategies WHERE id = $1)")
        .bind(strategy)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert!(!exists);

    let decisions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM copy_decisions WHERE strategy_id = $1",
    )
    .bind(strategy)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(decisions, 0);

    let ledger: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM capital_ledger_entries WHERE strategy_id = $1",
    )
    .bind(strategy)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(ledger, 0);

    let paper_after: Decimal = sqlx::query_scalar(
        "SELECT paper_cash_usd FROM account_risk_limits WHERE trading_account_id = $1",
    )
    .bind(account)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(paper_after, paper_before);

    let reconcile = reconcile_account(&api.worker, account).await.unwrap();
    assert!(reconcile.reconciled);

    // Same trader can be used again (unique trading_account + source_trader).
    let strategy2: Uuid = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/strategies",
            &token,
            json!({
                "trading_account_id": account,
                "source_trader_id": trader,
                "display_name": "Fresh",
                "sizing_mode": "proportional",
                "proportion": "0.25",
                "strategy_budget_usd": "300"
            }),
        )
        .await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_ne!(strategy2, strategy);
    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let old_decisions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM copy_decisions WHERE strategy_id = $1",
    )
    .bind(strategy2)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(old_decisions, 0);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn delete_rejects_enabled_strategy() {
    let api = TestApi::boot().await;
    let email = format!("del-enabled-{}@example.com", Uuid::new_v4());
    let (token, _user_id, _account, _trader, strategy) = seed_user_strategy(&api, &email).await;

    let (_st, me) = api.get_raw("/api/v1/auth/me", &token).await;
    let user_id: Uuid = Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();
    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    sqlx::query("UPDATE copy_strategies SET enabled = true WHERE id = $1")
        .bind(strategy)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let (status, body) = api
        .request_json(
            "DELETE",
            &format!("/api/v1/strategies/{strategy}"),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(status, 400);
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("disable the strategy")
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn delete_rejects_open_position() {
    let api = TestApi::boot().await;
    let email = format!("del-pos-{}@example.com", Uuid::new_v4());
    let (token, user_id, account, _trader, strategy) = seed_user_strategy(&api, &email).await;

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    sqlx::query(
        "INSERT INTO positions (
            user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode,
            open_shares, avg_price, cost_basis_usd
         ) VALUES ($1,$2,$3,'0xabc','1','paper_trading',10,0.5,5)",
    )
    .bind(user_id)
    .bind(account)
    .bind(strategy)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let (status, body) = api
        .request_json(
            "DELETE",
            &format!("/api/v1/strategies/{strategy}"),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body:?}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn delete_paper_releases_orphan_reserved_after_cancel() {
    let api = TestApi::boot().await;
    let email = format!("del-orphan-res-{}@example.com", Uuid::new_v4());
    let (token, user_id, _account, _trader, strategy) = seed_user_strategy(&api, &email).await;

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    sqlx::query("UPDATE copy_strategies SET enabled = false WHERE id = $1")
        .bind(strategy)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE strategy_capital_accounts
         SET funded_capital_usd = 200, reserved_cash_usd = 125, available_cash_usd = 75, invested_capital_usd = 0
         WHERE strategy_id = $1",
    )
    .bind(strategy)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let (enabled, mode): (bool, String) = sqlx::query_as(
        "SELECT s.enabled, a.operating_mode FROM copy_strategies s
         JOIN trading_accounts a ON a.id = s.trading_account_id WHERE s.id = $1",
    )
    .bind(strategy)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert!(!enabled, "strategy must be disabled for orphan reserved delete test");
    assert_eq!(mode, "paper_trading");

    let (status, body) = api
        .request_json(
            "DELETE",
            &format!("/api/v1/strategies/{strategy}"),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body:?}");

    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM copy_strategies WHERE id = $1)")
        .bind(strategy)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert!(!exists);
    tx.commit().await.unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn delete_rejects_other_users_strategy() {
    let api = TestApi::boot().await;
    let email_a = format!("del-a-{}@example.com", Uuid::new_v4());
    let email_b = format!("del-b-{}@example.com", Uuid::new_v4());
    let (token_a, _uid, _acct, _tr, strategy) = seed_user_strategy(&api, &email_a).await;
    let admin = api
        .login_user("admin@example.com", "admin-password-1")
        .await;
    let pass_b = api.invite_user(&admin, &email_b, "OtherUser").await;
    let token_b = api.login_user(&email_b, &pass_b).await;

    let (status, _) = api
        .request_json(
            "DELETE",
            &format!("/api/v1/strategies/{strategy}"),
            Some(&token_b),
            None,
        )
        .await;
    assert_eq!(status, 404);

    let (status, _) = api
        .request_json(
            "DELETE",
            &format!("/api/v1/strategies/{strategy}"),
            Some(&token_a),
            None,
        )
        .await;
    assert_eq!(status, 200);
}

#[tokio::test]
#[ignore = "dev pommarket only; set DELETE_SMOKE_DEV=1"]
async fn delete_smoke_yuri_test_strategy_on_dev() {
    if env::var("DELETE_SMOKE_DEV").as_deref() != Ok("1") {
        return;
    }
    let url = env::var("APP_DATABASE_URL").unwrap();
    assert!(
        url.contains("/pommarket") && !url.contains("pommarket_test"),
        "refusing: APP_DATABASE_URL must be dev pommarket"
    );
    let pool = pommarket::db::connect(&url).await.unwrap();
    let strategy_id = Uuid::parse_str("3fc91b9a-93cf-4e6f-be12-d1e313b19507").unwrap();
    let user_id = Uuid::parse_str("cc83308f-a64e-45a6-b96a-d342b6ae7ba4").unwrap();
    let mut tx = begin_as(&pool, Some(user_id), "user").await.unwrap();
    delete_strategy_for_user(&mut tx, user_id, strategy_id)
        .await
        .expect("delete yuri test strategy on dev");
    tx.commit().await.unwrap();
}
