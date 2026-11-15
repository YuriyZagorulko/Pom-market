mod common;
mod test_support;

use common::TestApi;
use pommarket::trading_control::{pipeline_blocked_combined, STATE_KILL_SWITCH, STATE_NORMAL, STATE_SOFT_CLOSE};
use serde_json::json;
use uuid::Uuid;

#[test]
fn strategy_kill_combined_with_account_normal() {
    assert_eq!(
        pipeline_blocked_combined(STATE_NORMAL, STATE_KILL_SWITCH, true, false, "paper_trading"),
        Some("emergency_stop")
    );
    assert_eq!(
        pipeline_blocked_combined(STATE_NORMAL, STATE_KILL_SWITCH, false, false, "paper_trading"),
        Some("emergency_stop")
    );
}

#[test]
fn account_soft_close_blocks_strategy_normal_buy() {
    assert_eq!(
        pipeline_blocked_combined(STATE_SOFT_CLOSE, STATE_NORMAL, true, false, "paper_trading"),
        Some("soft_close_no_buy")
    );
    assert!(pipeline_blocked_combined(STATE_SOFT_CLOSE, STATE_NORMAL, false, false, "paper_trading").is_none());
}

#[test]
fn strategy_soft_close_allows_sell() {
    assert_eq!(
        pipeline_blocked_combined(STATE_NORMAL, STATE_SOFT_CLOSE, true, false, "paper_trading"),
        Some("soft_close_no_buy")
    );
    assert!(pipeline_blocked_combined(STATE_NORMAL, STATE_SOFT_CLOSE, false, false, "paper_trading").is_none());
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn strategy_trading_control_api_smoke() {
    let api = TestApi::boot().await;
    let admin = api
        .login_user("admin@example.com", "admin-password-1")
        .await;
    let pass = api
        .invite_user(&admin, "str-ctl@example.com", "StrCtl")
        .await;
    let user = api.login_user("str-ctl@example.com", &pass).await;

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

    let trader: Uuid = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/source-traders",
            &user,
            json!({
                "alias": "T",
                "wallet_address": "0x1111111111111111111111111111111111111111"
            }),
        )
        .await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();

    let strategy: Uuid = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/strategies",
            &user,
            json!({
                "trading_account_id": account,
                "source_trader_id": trader,
                "display_name": "S1",
                "sizing_mode": "fixed_amount",
                "fixed_amount_usd": "10",
                "strategy_budget_usd": "100"
            }),
        )
        .await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();

    api.send(
        "POST",
        &format!("/api/v1/strategies/{strategy}/trading-control/kill-switch/activate"),
        &user,
        json!({"confirm": true}),
    )
    .await;

    let mut wtx = pommarket::db::begin_as(&api.worker, None, "worker")
        .await
        .unwrap();
    let st: String = sqlx::query_scalar(
        "SELECT trading_control_state FROM copy_strategies WHERE id = $1",
    )
    .bind(strategy)
    .fetch_one(&mut *wtx)
    .await
    .unwrap();
    wtx.commit().await.unwrap();
    assert_eq!(st, "kill_switch");

    let (_, acct_ctl) = api
        .get_raw(
            &format!("/api/v1/accounts/{account}/trading-control"),
            &user,
        )
        .await;
    assert_eq!(acct_ctl["trading_control_state"], "normal");
}
