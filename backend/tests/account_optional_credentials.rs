//! Account private key optional for paper; required for live activation.

mod common;
mod test_support;

use common::TestApi;
use pommarket::strategy_operating_mode::{
    ACCOUNT_CREDENTIALS_REQUIRED_CODE, LIVE_COPY_PRODUCT_READY, LIVE_TRADING_NOT_READY_CODE,
};
use reqwest::StatusCode;
use serde_json::json;
use test_support::assert_all_test_database_urls;
use uuid::Uuid;

const VALID_KEY: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
const SIGNER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

async fn user_token(api: &TestApi) -> String {
    let admin = api
        .login_user("admin@example.com", "admin-password-1")
        .await;
    let email = format!("cred-{}@example.com", Uuid::new_v4());
    let pass = api.invite_user(&admin, &email, "CredUser").await;
    api.login_user(&email, &pass).await
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn create_account_without_private_key_succeeds() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let created = api
        .send(
            "POST",
            "/api/v1/accounts",
            &token,
            json!({
                "name": "Paper only",
                "signature_type": "eoa",
                "signer_address": SIGNER,
            }),
        )
        .await;
    let id = created["id"].as_str().unwrap();
    let (_, detail) = api.get_raw(&format!("/api/v1/accounts/{id}"), &token).await;
    assert_eq!(detail["credentials_stored"], false);
    assert!(created.get("private_key").is_none());
    let body = created.to_string();
    assert!(!body.contains(VALID_KEY.trim_start_matches("0x")));
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn create_account_with_private_key_succeeds() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let created = api
        .send(
            "POST",
            "/api/v1/accounts",
            &token,
            json!({
                "name": "With key",
                "signature_type": "eoa",
                "signer_address": SIGNER,
                "private_key": VALID_KEY,
            }),
        )
        .await;
    let id = created["id"].as_str().unwrap();
    let (_, detail) = api.get_raw(&format!("/api/v1/accounts/{id}"), &token).await;
    assert_eq!(detail["credentials_stored"], true);
    assert!(created.get("private_key").is_none());
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn create_account_rejects_invalid_private_key() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let (status, body) = api
        .request_json(
            "POST",
            "/api/v1/accounts",
            Some(&token),
            Some(json!({
                "name": "Bad key",
                "signature_type": "eoa",
                "signer_address": SIGNER,
                "private_key": "0xdead",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let text = body.to_string();
    assert!(!text.contains(VALID_KEY));
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn paper_mode_without_credentials() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let created = api
        .send(
            "POST",
            "/api/v1/accounts",
            &token,
            json!({
                "name": "Paper",
                "signature_type": "eoa",
                "signer_address": SIGNER,
            }),
        )
        .await;
    let account_id = created["id"].as_str().unwrap();
    api.send(
        "POST",
        &format!("/api/v1/accounts/{account_id}/mode"),
        &token,
        json!({ "mode": "paper_trading" }),
    )
    .await;
    api.fund_paper(&token, account_id, "1000").await;
    let trader = api
        .send(
            "POST",
            "/api/v1/source-traders",
            &token,
            json!({
                "alias": "Lead",
                "wallet_address": "0xcccccccccccccccccccccccccccccccccccccccc"
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
                "fixed_amount_usd": "5",
                "strategy_budget_usd": "50"
            }),
        )
        .await;
    assert_eq!(strategy["operating_mode"], "paper_trading");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn strategy_live_without_credentials_rejected() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let account = api
        .send(
            "POST",
            "/api/v1/accounts",
            &token,
            json!({
                "name": "No creds",
                "signature_type": "eoa",
                "signer_address": SIGNER,
            }),
        )
        .await;
    let account_id = account["id"].as_str().unwrap();
    api.send(
        "POST",
        &format!("/api/v1/accounts/{account_id}/mode"),
        &token,
        json!({ "mode": "paper_trading" }),
    )
    .await;
    let trader = api
        .send(
            "POST",
            "/api/v1/source-traders",
            &token,
            json!({
                "alias": "L",
                "wallet_address": "0xdddddddddddddddddddddddddddddddddddddddd"
            }),
        )
        .await;
    let strategy_id = api
        .send(
            "POST",
            "/api/v1/strategies",
            &token,
            json!({
                "trading_account_id": account_id,
                "source_trader_id": trader["id"],
                "sizing_mode": "fixed_amount",
                "fixed_amount_usd": "5",
                "strategy_budget_usd": "50"
            }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, body) = api
        .request_json(
            "POST",
            &format!("/api/v1/strategies/{strategy_id}/operating-mode"),
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
    let (_, snap) = api
        .get_raw(&format!("/api/v1/strategies/{strategy_id}"), &token)
        .await;
    assert_eq!(snap["operating_mode"], "paper_trading");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn account_live_mode_without_credentials_rejected() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let account = api
        .send(
            "POST",
            "/api/v1/accounts",
            &token,
            json!({
                "name": "No key",
                "signature_type": "eoa",
                "signer_address": SIGNER,
            }),
        )
        .await;
    let account_id = account["id"].as_str().unwrap();
    let (status, body) = api
        .request_json(
            "POST",
            &format!("/api/v1/accounts/{account_id}/mode"),
            Some(&token),
            Some(json!({
                "mode": "live_trading",
                "confirm_live": true,
                "confirmation": "ENABLE LIVE TRADING"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains(ACCOUNT_CREDENTIALS_REQUIRED_CODE));
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn add_credentials_later_via_rotate() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let account = api
        .send(
            "POST",
            "/api/v1/accounts",
            &token,
            json!({
                "name": "Late key",
                "signature_type": "eoa",
                "signer_address": SIGNER,
            }),
        )
        .await;
    let account_id = account["id"].as_str().unwrap();
    api.send(
        "POST",
        &format!("/api/v1/accounts/{account_id}/credentials/rotate"),
        &token,
        json!({
            "name": "Late key",
            "signature_type": "eoa",
            "signer_address": SIGNER,
            "private_key": VALID_KEY,
        }),
    )
    .await;
    let (_, detail) = api
        .get_raw(&format!("/api/v1/accounts/{account_id}"), &token)
        .await;
    assert_eq!(detail["credentials_stored"], true);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn strategy_live_with_credentials_still_product_gated() {
    assert_all_test_database_urls();
    assert!(!LIVE_COPY_PRODUCT_READY);
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let account = api
        .send(
            "POST",
            "/api/v1/accounts",
            &token,
            json!({
                "name": "Keyed",
                "signature_type": "eoa",
                "signer_address": SIGNER,
                "private_key": VALID_KEY,
            }),
        )
        .await;
    let account_id = account["id"].as_str().unwrap();
    api.send(
        "POST",
        &format!("/api/v1/accounts/{account_id}/mode"),
        &token,
        json!({ "mode": "paper_trading" }),
    )
    .await;
    let trader = api
        .send(
            "POST",
            "/api/v1/source-traders",
            &token,
            json!({
                "alias": "T",
                "wallet_address": "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
            }),
        )
        .await;
    let strategy_id = api
        .send(
            "POST",
            "/api/v1/strategies",
            &token,
            json!({
                "trading_account_id": account_id,
                "source_trader_id": trader["id"],
                "sizing_mode": "fixed_amount",
                "fixed_amount_usd": "5",
                "strategy_budget_usd": "50"
            }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, body) = api
        .request_json(
            "POST",
            &format!("/api/v1/strategies/{strategy_id}/operating-mode"),
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
}