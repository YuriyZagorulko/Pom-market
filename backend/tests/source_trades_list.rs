mod common;
mod test_support;

use common::TestApi;
use serde_json::json;

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at a database whose name contains pommarket_test"]
async fn source_trades_paged_filters_and_tenant_isolation() {
    let api = TestApi::boot().await;
    let admin_login = api
        .post(
            "/api/v1/auth/login",
            json!({"email": "admin@example.com", "password": "admin-password-1"}),
        )
        .await;
    let admin_token = admin_login["access_token"].as_str().unwrap().to_string();
    let email_a = "source-list-a@example.com";
    let pass_a = api.invite_user(&admin_token, email_a, "SourceListA").await;
    let token_a = api.login_user(email_a, &pass_a).await;
    let email_b = "source-list-b@example.com";
    let pass_b = api.invite_user(&admin_token, email_b, "SourceListB").await;
    let token_b = api.login_user(email_b, &pass_b).await;

    let wallet = "0x3333333333333333333333333333333333333333";
    let trader = api
        .send(
            "POST",
            "/api/v1/source-traders",
            &token_a,
            json!({"wallet_address": wallet, "alias": "ListA"}),
        )
        .await;
    let trader_id = trader["id"].as_str().unwrap();
    let buy = TestApi::sample_trade(wallet, "buy", "5", "0xlistbuy");
    api.inject_and_process(&buy).await;

    let (_, body) = api
        .get_raw("/api/v1/source-trades?limit=10&page=1", &token_a)
        .await;
    assert_eq!(body["total"].as_i64().unwrap_or(0), 1);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);

    let (_, side) = api
        .get_raw("/api/v1/source-trades?side=sell", &token_a)
        .await;
    assert_eq!(side["items"].as_array().unwrap().len(), 0);

    let (_, wallets) = api
        .get_raw(
            &format!("/api/v1/source-trades?wallets={trader_id}"),
            &token_a,
        )
        .await;
    assert_eq!(wallets["items"].as_array().unwrap().len(), 1);

    let (_, foreign) = api.get_raw("/api/v1/source-trades", &token_b).await;
    assert_eq!(foreign["items"].as_array().unwrap().len(), 0);
}
