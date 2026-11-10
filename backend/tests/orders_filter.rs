mod common;
mod test_support;

use common::TestApi;
use serde_json::json;
use test_support::list_items;

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at a database whose name contains pommarket_test"]
async fn orders_list_respects_empty_and_explicit_filters() {
    let api = TestApi::boot().await;
    let admin_login = api
        .post(
            "/api/v1/auth/login",
            json!({"email": "admin@example.com", "password": "admin-password-1"}),
        )
        .await;
    let admin_token = admin_login["access_token"].as_str().unwrap().to_string();

    let email = "orders-filter@example.com";
    let password = api.invite_user(&admin_token, email, "OrdersFilter").await;
    let token = api.login_user(email, &password).await;

    let account = api
        .send(
            "POST",
            "/api/v1/accounts",
            &token,
            json!({
                "name": "Filter acct",
                "signature_type": "eoa",
                "signer_address": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
            }),
        )
        .await;
    let account_id = account["id"].as_str().unwrap();
    api.send(
        "PATCH",
        &format!("/api/v1/accounts/{account_id}/risk"),
        &token,
        json!({"max_order_usd": "100", "max_exposure_usd": "1000"}),
    )
    .await;
    api.fund_paper(&token, account_id, "1000").await;
    api.send(
        "POST",
        &format!("/api/v1/accounts/{account_id}/mode"),
        &token,
        json!({"mode": "paper_trading"}),
    )
    .await;

    let trader = api
        .send(
            "POST",
            "/api/v1/source-traders",
            &token,
            json!({
                "wallet_address": "0x2222222222222222222222222222222222222222",
                "alias": "FilterLeader"
            }),
        )
        .await;
    let trader_id = trader["id"].as_str().unwrap();
    let strategy = api
        .send(
            "POST",
            "/api/v1/strategies",
            &token,
            json!({
                "trading_account_id": account_id,
                "source_trader_id": trader_id,
                "sizing_mode": "fixed_amount",
                "fixed_amount_usd": "20",
                "strategy_budget_usd": "100"
            }),
        )
        .await;
    let strategy_id = strategy["id"].as_str().unwrap();
    api.bootstrap_capital(&token, account_id).await;
    api.send(
        "POST",
        &format!("/api/v1/strategies/{strategy_id}/enable"),
        &token,
        json!({"enabled": true}),
    )
    .await;

    let wallet = "0x2222222222222222222222222222222222222222";
    let buy = TestApi::sample_trade(wallet, "buy", "10", "0xfilterbuy");
    let sell = TestApi::sample_trade(wallet, "sell", "10", "0xfiltersell");
    api.inject_and_process(&buy).await;
    api.inject_and_process(&sell).await;

    let count = |orders: &serde_json::Value| {
        orders
            .get("items")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .or_else(|| orders.as_array().map(|a| a.len()))
            .unwrap_or(0)
    };

    let (_, none) = api.get_raw("/api/v1/orders", &token).await;
    assert_eq!(count(&none), 2, "no query params");

    let (_, empty) = api
        .get_raw("/api/v1/orders?limit=50&q=&side=&status=", &token)
        .await;
    assert_eq!(count(&empty), 2, "empty side/status must not filter");

    let (_, buys) = api.get_raw("/api/v1/orders?side=buy", &token).await;
    assert_eq!(count(&buys), 1);
    assert_eq!(list_items(&buys)[0]["side"], "buy");

    let (_, sells) = api.get_raw("/api/v1/orders?side=sell", &token).await;
    assert_eq!(count(&sells), 1);

    let (_, filled) = api.get_raw("/api/v1/orders?status=filled", &token).await;
    assert_eq!(count(&filled), 2);

    let (_, combo) = api
        .get_raw("/api/v1/orders?side=buy&status=filled", &token)
        .await;
    assert_eq!(count(&combo), 1);

    let (_, bogus) = api.get_raw("/api/v1/orders?side=notaside", &token).await;
    assert_eq!(count(&bogus), 0, "unknown side should yield empty list");

    let (_, bogus_status) = api
        .get_raw("/api/v1/orders?status=notastatus", &token)
        .await;
    assert_eq!(count(&bogus_status), 0);
}
