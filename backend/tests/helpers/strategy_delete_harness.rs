//! Shared helpers for strategy delete integration tests (included via `#[path]`).

use crate::common::TestApi;
use pommarket::db::begin_as;
use pommarket::strategy_accounting::{allocate_to_strategy, ensure_strategy_capital_row};
use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde_json::json;
use uuid::Uuid;

pub async fn seed_user_strategy(api: &TestApi, email: &str) -> (String, Uuid, Uuid, Uuid, Uuid) {
    let admin = api
        .login_user("admin@example.com", "admin-password-1")
        .await;
    let pass = api.invite_user(&admin, email, "DelUser").await;
    let token = api.login_user(email, &pass).await;

    let account: Uuid = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/accounts",
            &token,
            json!({
                "name": "Paper",
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

    let trader: Uuid = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/source-traders",
            &token,
            json!({
                "alias": "Leader",
                "wallet_address": format!("0x{:040x}", Uuid::new_v4().as_u128())
            }),
        )
        .await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    api.send(
        "PATCH",
        &format!("/api/v1/source-traders/{trader}"),
        &token,
        json!({ "monitoring_enabled": true }),
    )
    .await;

    let strategy: Uuid = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/strategies",
            &token,
            json!({
                "trading_account_id": account,
                "source_trader_id": trader,
                "display_name": "ToDelete",
                "sizing_mode": "proportional",
                "proportion": "0.5",
                "strategy_budget_usd": "500"
            }),
        )
        .await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();

    let (_status, me) = api.get_raw("/api/v1/auth/me", &token).await;
    let user_id: Uuid = Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();
    let mut tx = begin_as(&api.worker, Some(user_id), "user").await.unwrap();
    ensure_strategy_capital_row(&mut tx, user_id, account, strategy)
        .await
        .unwrap();
    allocate_to_strategy(
        &mut tx,
        user_id,
        account,
        strategy,
        Decimal::from(200),
        &format!("test-delete-alloc-{strategy}"),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    (token, user_id, account, trader, strategy)
}

pub async fn delete_http(
    api: &TestApi,
    token: &str,
    strategy_id: Uuid,
) -> (StatusCode, serde_json::Value) {
    let (status, body) = api
        .request_json(
            "DELETE",
            &format!("/api/v1/strategies/{strategy_id}"),
            Some(token),
            None,
        )
        .await;
    (status, body)
}
