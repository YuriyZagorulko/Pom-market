//! HTTP integration: copy_small_buys_1to1 strategy settings (pommarket_test only).
mod common;
mod test_support;

use common::TestApi;
use serde_json::json;
use uuid::Uuid;

async fn patch_expect_status(
    api: &TestApi,
    path: &str,
    token: &str,
    body: serde_json::Value,
    expected: reqwest::StatusCode,
) -> String {
    let response = api
        .http
        .request(reqwest::Method::PATCH, format!("{}{path}", api.base))
        .header("authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, expected, "body: {text}");
    text
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn copy_small_buys_strategy_api() {
    test_support::assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let admin = api
        .login_user("admin@example.com", "admin-password-1")
        .await;
    let pass = api
        .invite_user(&admin, "small-buy-api@example.com", "SmallBuy")
        .await;
    let user = api.login_user("small-buy-api@example.com", &pass).await;

    let account: Uuid = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/accounts",
            &user,
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
        "PATCH",
        &format!("/api/v1/accounts/{account}"),
        &user,
        json!({"operating_mode": "paper_trading"}),
    )
    .await;
    api.fund_paper(&user, &account.to_string(), "10000").await;

    let trader: Uuid = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/source-traders",
            &user,
            json!({
                "alias": "Leader",
                "wallet_address": "0x2222222222222222222222222222222222222222"
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
        &user,
        json!({"monitoring_enabled": true}),
    )
    .await;

    let strategy: Uuid = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/strategies",
            &user,
            json!({
                "trading_account_id": account,
                "source_trader_id": trader,
                "sizing_mode": "position_value_ratio",
                "strategy_budget_usd": "500",
                "copy_small_buys_1to1": true,
                "small_buy_threshold_usd": "5"
            }),
        )
        .await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();

    let got = api
        .send(
            "GET",
            &format!("/api/v1/strategies/{strategy}"),
            &user,
            json!({}),
        )
        .await;
    assert_eq!(got["copy_small_buys_1to1"], true);
    assert!(
        got["small_buy_threshold_usd"]
            .as_str()
            .unwrap()
            .starts_with("5")
    );

    let listed = api
        .send("GET", "/api/v1/strategies", &user, json!({}))
        .await;
    let row = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == strategy.to_string())
        .expect("strategy in list");
    assert_eq!(row["copy_small_buys_1to1"], true);

    api.send(
        "PATCH",
        &format!("/api/v1/strategies/{strategy}"),
        &user,
        json!({"max_position_usd": "80"}),
    )
    .await;
    let after_omit = api
        .send(
            "GET",
            &format!("/api/v1/strategies/{strategy}"),
            &user,
            json!({}),
        )
        .await;
    assert_eq!(after_omit["copy_small_buys_1to1"], true);
    assert!(
        after_omit["small_buy_threshold_usd"]
            .as_str()
            .unwrap()
            .starts_with("5")
    );
    assert!(
        after_omit["max_position_usd"]
            .as_str()
            .unwrap()
            .starts_with("80")
    );

    api.send(
        "PATCH",
        &format!("/api/v1/strategies/{strategy}"),
        &user,
        json!({
            "copy_small_buys_1to1": false
        }),
    )
    .await;
    let disabled = api
        .send(
            "GET",
            &format!("/api/v1/strategies/{strategy}"),
            &user,
            json!({}),
        )
        .await;
    assert_eq!(disabled["copy_small_buys_1to1"], false);
    assert!(
        disabled["small_buy_threshold_usd"]
            .as_str()
            .unwrap()
            .starts_with("5"),
        "threshold preserved when feature disabled"
    );

    api.send(
        "PATCH",
        &format!("/api/v1/strategies/{strategy}"),
        &user,
        json!({
            "copy_small_buys_1to1": true,
            "small_buy_threshold_usd": "7.50"
        }),
    )
    .await;
    let reenabled = api
        .send(
            "GET",
            &format!("/api/v1/strategies/{strategy}"),
            &user,
            json!({}),
        )
        .await;
    assert_eq!(reenabled["copy_small_buys_1to1"], true);
    assert!(
        reenabled["small_buy_threshold_usd"]
            .as_str()
            .unwrap()
            .starts_with("7.5")
    );

    let _ = patch_expect_status(
        &api,
        &format!("/api/v1/strategies/{strategy}"),
        &user,
        json!({"small_buy_threshold_usd": "0"}),
        reqwest::StatusCode::BAD_REQUEST,
    )
    .await;

    let _ = patch_expect_status(
        &api,
        &format!("/api/v1/strategies/{strategy}"),
        &user,
        json!({
            "copy_small_buys_1to1": true,
            "small_buy_threshold_usd": "-1"
        }),
        reqwest::StatusCode::BAD_REQUEST,
    )
    .await;

    let reject_create = api
        .http
        .post(format!("{}/api/v1/strategies", api.base))
        .header("authorization", format!("Bearer {user}"))
        .json(&json!({
            "trading_account_id": account,
            "source_trader_id": trader,
            "sizing_mode": "fixed_amount",
            "fixed_amount_usd": "10",
            "strategy_budget_usd": "100",
            "copy_small_buys_1to1": true,
            "small_buy_threshold_usd": "5"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(reject_create.status(), reqwest::StatusCode::BAD_REQUEST);
}
