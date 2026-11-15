//! Per-strategy operating mode (paper / live gate).

mod common;
mod test_support;

use common::TestApi;
use pommarket::strategy_operating_mode::{
    ACCOUNT_CREDENTIALS_REQUIRED_CODE, LIVE_COPY_PRODUCT_READY, LIVE_TRADING_NOT_READY_CODE,
};
use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde_json::json;
use test_support::assert_all_test_database_urls;
use uuid::Uuid;

const VALID_KEY: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";

async fn migration_pool() -> sqlx::PgPool {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    pommarket::db::connect(&url).await.expect("migration db")
}

async fn setup_user_with_two_strategies(api: &TestApi) -> (String, Uuid, Uuid) {
    let admin = api
        .login_user("admin@example.com", "admin-password-1")
        .await;
    let email = format!("mode-{}@example.com", Uuid::new_v4());
    let pass = api.invite_user(&admin, &email, "Mode").await;
    let token = api.login_user(&email, &pass).await;

    let account = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/accounts",
            &token,
            json!({
                "name": "Acct",
                "signature_type": "eoa",
                "signer_address": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
                "funder_address": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
            }),
        )
        .await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    api.send(
        "POST",
        &format!("/api/v1/accounts/{account}/mode"),
        &token,
        json!({ "mode": "paper_trading" }),
    )
    .await;
    api.fund_paper(&token, &account.to_string(), "5000").await;

    let trader1 = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/source-traders",
            &token,
            json!({
                "alias": "L1",
                "wallet_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }),
        )
        .await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    api.send(
        "PATCH",
        &format!("/api/v1/source-traders/{trader1}"),
        &token,
        json!({ "monitoring_enabled": true }),
    )
    .await;

    let trader2 = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/source-traders",
            &token,
            json!({
                "alias": "L2",
                "wallet_address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            }),
        )
        .await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    api.send(
        "PATCH",
        &format!("/api/v1/source-traders/{trader2}"),
        &token,
        json!({ "monitoring_enabled": true }),
    )
    .await;

    let s1 = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/strategies",
            &token,
            json!({
                "trading_account_id": account,
                "source_trader_id": trader1,
                "sizing_mode": "fixed_amount",
                "fixed_amount_usd": "10",
                "strategy_budget_usd": "200"
            }),
        )
        .await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let s2 = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/strategies",
            &token,
            json!({
                "trading_account_id": account,
                "source_trader_id": trader2,
                "sizing_mode": "fixed_amount",
                "fixed_amount_usd": "10",
                "strategy_budget_usd": "200"
            }),
        )
        .await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    (token, s1, s2)
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn existing_strategies_default_to_paper() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let (token, s1, _) = setup_user_with_two_strategies(&api).await;
    let (_, body) = api
        .get_raw(&format!("/api/v1/strategies/{s1}"), &token)
        .await;
    assert_eq!(body["operating_mode"], "paper_trading");
    assert!(!LIVE_COPY_PRODUCT_READY);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn mode_independent_per_strategy() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let (token, s1, s2) = setup_user_with_two_strategies(&api).await;
    let (status, body) = api
        .request_json(
            "POST",
            &format!("/api/v1/strategies/{s1}/operating-mode"),
            Some(&token),
            Some(json!({
                "operating_mode": "live_trading",
                "confirm_live": true,
                "confirmation": "ENABLE LIVE TRADING"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains(ACCOUNT_CREDENTIALS_REQUIRED_CODE));
    let (_, b1) = api.get_raw(&format!("/api/v1/strategies/{s1}"), &token).await;
    let (_, b2) = api.get_raw(&format!("/api/v1/strategies/{s2}"), &token).await;
    assert_eq!(b1["operating_mode"], "paper_trading");
    assert_eq!(b2["operating_mode"], "paper_trading");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn cross_user_cannot_change_mode() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let (token_a, s1, _) = setup_user_with_two_strategies(&api).await;
    let admin = api
        .login_user("admin@example.com", "admin-password-1")
        .await;
    let email_b = format!("other-{}@example.com", Uuid::new_v4());
    let pass_b = api.invite_user(&admin, &email_b, "TenCharsB").await;
    let token_b = api.login_user(&email_b, &pass_b).await;
    let _ = token_a;
    let (status, _) = api
        .request_json(
            "POST",
            &format!("/api/v1/strategies/{s1}/operating-mode"),
            Some(&token_b),
            Some(json!({ "operating_mode": "paper_trading" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn rejects_enabled_active_orders_positions_audit() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let (token, s1, _) = setup_user_with_two_strategies(&api).await;
    let mig = migration_pool().await;
    let user_id: Uuid = sqlx::query_scalar("SELECT user_id FROM copy_strategies WHERE id = $1")
        .bind(s1)
        .fetch_one(&mig)
        .await
        .unwrap();
    let account_id: Uuid =
        sqlx::query_scalar("SELECT trading_account_id FROM copy_strategies WHERE id = $1")
            .bind(s1)
            .fetch_one(&mig)
            .await
            .unwrap();

    api.send(
        "POST",
        &format!("/api/v1/strategies/{s1}/enable"),
        &token,
        json!({ "enabled": true }),
    )
    .await;
    let (status, _) = api
        .request_json(
            "POST",
            &format!("/api/v1/strategies/{s1}/operating-mode"),
            Some(&token),
            Some(json!({ "operating_mode": "paper_trading" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    api.send(
        "POST",
        &format!("/api/v1/strategies/{s1}/enable"),
        &token,
        json!({ "enabled": false }),
    )
    .await;

    let event: Uuid = sqlx::query_scalar(
        "INSERT INTO source_events (dedup_key, source_wallet, token_id, side, price, size_shares, notional_usd, event_ts, received_at, origin, status, raw)
         VALUES ('d-mode', '0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'tok', 'buy', 0.5, 10, 5, now(), now(), 'test', 'processed', '{}'::jsonb) RETURNING id",
    )
    .fetch_one(&mig)
    .await
    .unwrap();
    let trader_id: Uuid =
        sqlx::query_scalar("SELECT source_trader_id FROM copy_strategies WHERE id = $1")
            .bind(s1)
            .fetch_one(&mig)
            .await
            .unwrap();
    let decision: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_decisions (user_id, trading_account_id, source_event_id, source_trader_id, strategy_id, status, reason, operating_mode, side, strategy_snapshot, checks, detected_at)
         VALUES ($1,$2,$3,$4,$5,'approved','ok','paper_trading','buy','{}','{}',now()) RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(event)
    .bind(trader_id)
    .bind(s1)
    .fetch_one(&mig)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orders (user_id, trading_account_id, decision_id, source_event_id, client_order_id, operating_mode, order_type, side, token_id, limit_price, size_shares, status)
         VALUES ($1,$2,$3,$4,gen_random_uuid(),'paper_trading','limit','buy','tok',0.5,10,'submitted')",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(decision)
    .bind(event)
    .execute(&mig)
    .await
    .unwrap();
    let (status, _) = api
        .request_json(
            "POST",
            &format!("/api/v1/strategies/{s1}/operating-mode"),
            Some(&token),
            Some(json!({ "operating_mode": "paper_trading" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    sqlx::query("DELETE FROM orders WHERE decision_id = $1")
        .bind(decision)
        .execute(&mig)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, operating_mode, open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,$3,'0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','tok','paper_trading',5,0.5,2.5)",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(s1)
    .execute(&mig)
    .await
    .unwrap();
    let (status, _) = api
        .request_json(
            "POST",
            &format!("/api/v1/strategies/{s1}/operating-mode"),
            Some(&token),
            Some(json!({ "operating_mode": "paper_trading" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    sqlx::query("DELETE FROM positions WHERE strategy_id = $1")
        .bind(s1)
        .execute(&mig)
        .await
        .unwrap();

    let funded_before: Decimal = sqlx::query_scalar(
        "SELECT funded_capital_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(s1)
    .fetch_one(&mig)
    .await
    .unwrap();
    api.send(
        "POST",
        &format!("/api/v1/accounts/{account_id}/credentials/rotate"),
        &token,
        json!({
            "name": "Acct",
            "signature_type": "eoa",
            "signer_address": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
            "private_key": VALID_KEY,
        }),
    )
    .await;
    let (status, body) = api
        .request_json(
            "POST",
            &format!("/api/v1/strategies/{s1}/operating-mode"),
            Some(&token),
            Some(json!({
                "operating_mode": "live_trading",
                "confirm_live": true,
                "confirmation": "ENABLE LIVE TRADING"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.to_string().contains(LIVE_TRADING_NOT_READY_CODE));
    let funded_after: Decimal = sqlx::query_scalar(
        "SELECT funded_capital_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(s1)
    .fetch_one(&mig)
    .await
    .unwrap();
    assert_eq!(funded_before, funded_after);

    sqlx::query(
        "INSERT INTO hard_close_operations (user_id, trading_account_id, strategy_id, idempotency_key, status)
         VALUES ($1,$2,$3,'mode-hc','queued')",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(s1)
    .execute(&mig)
    .await
    .unwrap();
    let (status, _) = api
        .request_json(
            "POST",
            &format!("/api/v1/strategies/{s1}/operating-mode"),
            Some(&token),
            Some(json!({ "operating_mode": "paper_trading" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    sqlx::query("DELETE FROM hard_close_operations WHERE strategy_id = $1")
        .bind(s1)
        .execute(&mig)
        .await
        .unwrap();

    sqlx::query("UPDATE copy_strategies SET operating_mode = 'live_trading' WHERE id = $1")
        .bind(s1)
        .execute(&mig)
        .await
        .unwrap();
    let (_st, res) = api
        .request_json(
            "POST",
            &format!("/api/v1/strategies/{s1}/operating-mode"),
            Some(&token),
            Some(json!({ "operating_mode": "paper_trading" })),
        )
        .await;
    assert_eq!(res["operating_mode"], "paper_trading");
    let audit: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE action = 'strategy_operating_mode_changed' AND target = $1",
    )
    .bind(s1.to_string())
    .fetch_one(&mig)
    .await
    .unwrap();
    assert_eq!(audit, 1);
}
