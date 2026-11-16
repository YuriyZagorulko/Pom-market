//! Paper Trading Account vs Polymarket Wallet Account (`account_type`).

mod common;
mod test_support;

use common::TestApi;
use pommarket::trading_account::{ACCOUNT_TYPE_PAPER, ACCOUNT_TYPE_WALLET, PAPER_ACCOUNT_NO_LIVE_CODE};
use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde_json::json;
use test_support::assert_all_test_database_urls;
use uuid::Uuid;

const VALID_KEY: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
const SIGNER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

async fn user_token(api: &TestApi) -> String {
    let admin = api
        .login_user("admin@example.com", "admin-password-1")
        .await;
    let email = format!("paper-wallet-{}@example.com", Uuid::new_v4());
    let pass = api.invite_user(&admin, &email, "PaperWalletUser").await;
    api.login_user(&email, &pass).await
}

async fn create_paper(api: &TestApi, token: &str, name: &str) -> String {
    let created = api
        .send(
            "POST",
            "/api/v1/accounts",
            token,
            json!({ "name": name, "account_type": "paper" }),
        )
        .await;
    created["id"].as_str().unwrap().to_string()
}

async fn create_wallet_eoa(api: &TestApi, token: &str, name: &str) -> String {
    let created = api
        .send(
            "POST",
            "/api/v1/accounts",
            token,
            json!({
                "name": name,
                "account_type": "wallet",
                "signature_type": "eoa",
                "signer_address": SIGNER,
            }),
        )
        .await;
    created["id"].as_str().unwrap().to_string()
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn paper_account_create_name_only() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let id = create_paper(&api, &token, "Paper desk").await;
    let (_, detail) = api.get_raw(&format!("/api/v1/accounts/{id}"), &token).await;
    assert_eq!(detail["account"]["account_type"], ACCOUNT_TYPE_PAPER);
    assert_eq!(detail["account"]["name"], "Paper desk");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn paper_account_has_no_wallet_fields() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let id = create_paper(&api, &token, "No wallet").await;
    let (_, detail) = api.get_raw(&format!("/api/v1/accounts/{id}"), &token).await;
    let account = &detail["account"];
    assert!(account["signer_address"].is_null());
    assert!(account["funder_address"].is_null());
    assert!(account["signature_type"].is_null());
    assert_eq!(detail["credentials_stored"], false);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn paper_account_rejects_wallet_fields_on_create() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let (status, body) = api
        .request_json(
            "POST",
            "/api/v1/accounts",
            Some(&token),
            Some(json!({
                "name": "Bad",
                "account_type": "paper",
                "signer_address": SIGNER,
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains("cannot include wallet credentials"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn paper_account_available_for_paper_strategy() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let account_id = create_paper(&api, &token, "Strat paper").await;
    api.fund_paper(&token, &account_id, "500").await;
    let trader = api
        .send(
            "POST",
            "/api/v1/source-traders",
            &token,
            json!({
                "alias": "Lead",
                "wallet_address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }),
        )
        .await;
    let strategy = api
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
        .await;
    let strategy_id = strategy["id"].as_str().unwrap();
    let (_, detail) = api
        .get_raw(&format!("/api/v1/strategies/{strategy_id}"), &token)
        .await;
    assert_eq!(detail["operating_mode"], "paper_trading");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn paper_account_account_live_mode_rejected() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let account_id = create_paper(&api, &token, "No live").await;
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
    assert!(body.to_string().contains(PAPER_ACCOUNT_NO_LIVE_CODE));
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn paper_account_strategy_live_mode_rejected_via_api() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let account_id = create_paper(&api, &token, "Strat no live").await;
    api.fund_paper(&token, &account_id, "500").await;
    let trader = api
        .send(
            "POST",
            "/api/v1/source-traders",
            &token,
            json!({
                "alias": "T",
                "wallet_address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
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
    assert!(body.to_string().contains(PAPER_ACCOUNT_NO_LIVE_CODE));
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn wallet_account_create_with_valid_credentials() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let id = create_wallet_eoa(&api, &token, "Wallet EOA").await;
    let (_, detail) = api.get_raw(&format!("/api/v1/accounts/{id}"), &token).await;
    assert_eq!(detail["account"]["account_type"], ACCOUNT_TYPE_WALLET);
    assert_eq!(
        detail["account"]["signer_address"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase(),
        SIGNER.to_ascii_lowercase()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn wallet_account_rejects_invalid_signer() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let (status, _) = api
        .request_json(
            "POST",
            "/api/v1/accounts",
            Some(&token),
            Some(json!({
                "name": "Bad signer",
                "account_type": "wallet",
                "signature_type": "eoa",
                "signer_address": "not-an-address",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn default_account_type_is_wallet_for_legacy_body() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let created = api
        .send(
            "POST",
            "/api/v1/accounts",
            &token,
            json!({
                "name": "Legacy shape",
                "signature_type": "eoa",
                "signer_address": SIGNER,
                "private_key": VALID_KEY,
            }),
        )
        .await;
    let id = created["id"].as_str().unwrap();
    let (_, detail) = api.get_raw(&format!("/api/v1/accounts/{id}"), &token).await;
    assert_eq!(detail["account"]["account_type"], ACCOUNT_TYPE_WALLET);
    assert_eq!(detail["credentials_stored"], true);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn paper_account_test_skips_wallet_checks() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let account_id = create_paper(&api, &token, "Test skip").await;
    let report = api
        .send(
            "POST",
            &format!("/api/v1/accounts/{account_id}/test"),
            &token,
            json!({}),
        )
        .await;
    assert_eq!(report["wallet_checks_skipped"], true);
    assert_eq!(report["paper_account"], true);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn strategies_on_same_paper_account_keep_independent_capital() {
    assert_all_test_database_urls();
    let api = TestApi::boot().await;
    let token = user_token(&api).await;
    let account_id = create_paper(&api, &token, "Capital iso").await;
    api.fund_paper(&token, &account_id, "5000").await;
    let t1 = api
        .send(
            "POST",
            "/api/v1/source-traders",
            &token,
            json!({
                "alias": "C1",
                "wallet_address": "0xcccccccccccccccccccccccccccccccccccccccc"
            }),
        )
        .await;
    let t2 = api
        .send(
            "POST",
            "/api/v1/source-traders",
            &token,
            json!({
                "alias": "C2",
                "wallet_address": "0xdddddddddddddddddddddddddddddddddddddddd"
            }),
        )
        .await;
    let s1 = api
        .send(
            "POST",
            "/api/v1/strategies",
            &token,
            json!({
                "trading_account_id": account_id,
                "source_trader_id": t1["id"],
                "sizing_mode": "fixed_amount",
                "fixed_amount_usd": "5",
                "strategy_budget_usd": "100"
            }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let s2 = api
        .send(
            "POST",
            "/api/v1/strategies",
            &token,
            json!({
                "trading_account_id": account_id,
                "source_trader_id": t2["id"],
                "sizing_mode": "fixed_amount",
                "fixed_amount_usd": "5",
                "strategy_budget_usd": "250"
            }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let en1 = api
        .send(
            "POST",
            &format!("/api/v1/strategies/{s1}/enable"),
            &token,
            json!({ "enabled": true }),
        )
        .await;
    let en2 = api
        .send(
            "POST",
            &format!("/api/v1/strategies/{s2}/enable"),
            &token,
            json!({ "enabled": true }),
        )
        .await;
    let f1: Decimal = en1["capital_sync"]["funded_capital_usd"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let f2: Decimal = en2["capital_sync"]["funded_capital_usd"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(f1, Decimal::from(100));
    assert_eq!(f2, Decimal::from(250));
    assert_ne!(f1, f2);
}