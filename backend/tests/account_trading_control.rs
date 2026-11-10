mod common;
mod test_support;

use common::TestApi;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn trading_control_api_and_pipeline_guards() {
    let api = TestApi::boot().await;
    let admin = api
        .login_user("admin@example.com", "admin-password-1")
        .await;
    let pass = api
        .invite_user(&admin, "danger-zone@example.com", "DangerZone")
        .await;
    let user = api.login_user("danger-zone@example.com", &pass).await;

    let account: Uuid = Uuid::parse_str(
        api.send(
            "POST",
            "/api/v1/accounts",
            &user,
            json!({
                "name": "Danger",
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
        &format!("/api/v1/accounts/{account}/trading-control/kill-switch/activate"),
        &user,
        json!({"confirm": true}),
    )
    .await;
    let mut wtx = pommarket::db::begin_as(&api.worker, None, "worker")
        .await
        .unwrap();
    let kill_state: String =
        sqlx::query_scalar("SELECT trading_control_state FROM trading_accounts WHERE id = $1")
            .bind(account)
            .fetch_one(&mut *wtx)
            .await
            .unwrap();
    wtx.commit().await.unwrap();
    assert_eq!(kill_state, "kill_switch");

    api.send(
        "POST",
        &format!("/api/v1/accounts/{account}/trading-control/kill-switch/deactivate"),
        &user,
        json!({"confirm": true}),
    )
    .await;

    api.send(
        "POST",
        &format!("/api/v1/accounts/{account}/trading-control/soft-close/activate"),
        &user,
        json!({"confirm": true}),
    )
    .await;
    let mut wtx = pommarket::db::begin_as(&api.worker, None, "worker")
        .await
        .unwrap();
    let soft: String =
        sqlx::query_scalar("SELECT trading_control_state FROM trading_accounts WHERE id = $1")
            .bind(account)
            .fetch_one(&mut *wtx)
            .await
            .unwrap();
    wtx.commit().await.unwrap();
    assert_eq!(soft, "soft_close");

    let (_status, control) = api
        .get_raw(
            &format!("/api/v1/accounts/{account}/trading-control"),
            &user,
        )
        .await;
    assert_eq!(control["trading_control_state"], "soft_close");
    assert_eq!(control["blocks_buy"], true);
    assert_eq!(control["allows_copy_sell"], true);

    api.send(
        "POST",
        &format!("/api/v1/accounts/{account}/trading-control/resume"),
        &user,
        json!({"confirm": true}),
    )
    .await;
    let mut wtx = pommarket::db::begin_as(&api.worker, None, "worker")
        .await
        .unwrap();
    let normal: String =
        sqlx::query_scalar("SELECT trading_control_state FROM trading_accounts WHERE id = $1")
            .bind(account)
            .fetch_one(&mut *wtx)
            .await
            .unwrap();
    wtx.commit().await.unwrap();
    assert_eq!(normal, "normal");
}
