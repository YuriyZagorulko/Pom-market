mod common;
mod test_support;

use common::TestApi;
use pommarket::db::begin_as;
use serde_json::json;
use test_support::list_items;
#[tokio::test]
#[ignore = "requires PostgreSQL pointed at a database whose name contains pommarket_test"]
async fn pipeline_mock_scenarios() {
    let api = TestApi::boot().await;
    let admin_login = api
        .post(
            "/api/v1/auth/login",
            json!({"email": "admin@example.com", "password": "admin-password-1"}),
        )
        .await;
    let admin_token = admin_login["access_token"].as_str().unwrap().to_string();

    let email = "pipeline@example.com";
    let password = api.invite_user(&admin_token, email, "Pipeline").await;
    let token = api.login_user(email, &password).await;

    let account = api
        .send(
            "POST",
            "/api/v1/accounts",
            &token,
            json!({
                "name": "Pipeline acct",
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

    let unknown = TestApi::sample_trade(
        "0xdeaddeaddeaddeaddeaddeaddeaddeaddeaddead",
        "buy",
        "10",
        "0xunknownwallet",
    );
    let unknown_out = api.inject_and_process(&unknown).await;
    assert!(unknown_out.created, "event row may still be created");
    let (_, orders) = api.get_raw("/api/v1/orders", &token).await;
    assert!(list_items(&orders).is_empty(), "unwatched wallet");

    let trader = api
        .send(
            "POST",
            "/api/v1/source-traders",
            &token,
            json!({
                "wallet_address": "0x3333333333333333333333333333333333333333",
                "alias": "PipeLeader"
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
    assert_eq!(strategy["enabled"], false);

    let wallet = "0x3333333333333333333333333333333333333333";
    let disabled_trade = TestApi::sample_trade(wallet, "buy", "10", "0xdisabledstrat");
    api.inject_and_process(&disabled_trade).await;
    let (_, orders) = api.get_raw("/api/v1/orders", &token).await;
    assert!(list_items(&orders).is_empty(), "strategy disabled");

    let mut tx = begin_as(&api.worker, None, "worker").await.unwrap();
    let skipped: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM copy_decisions WHERE strategy_id = $1 AND status = 'skipped'",
    )
    .bind(uuid::Uuid::parse_str(strategy_id).unwrap())
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert!(skipped >= 1, "disabled strategy should be skipped");

    test_support::bootstrap_paper_capital_for_account(
        &api.worker,
        uuid::Uuid::parse_str(account_id).unwrap(),
    )
    .await;
    api.send(
        "POST",
        &format!("/api/v1/strategies/{strategy_id}/enable"),
        &token,
        json!({"enabled": true}),
    )
    .await;

    let good = TestApi::sample_trade(wallet, "buy", "10", "0xgoodbuy");
    api.inject_and_process(&good).await;
    let (_, orders) = api.get_raw("/api/v1/orders", &token).await;
    assert_eq!(list_items(&orders).len(), 1);

    let dup = api.inject_and_process(&good).await;
    assert!(!dup.created, "dedup");
    let (_, orders) = api.get_raw("/api/v1/orders", &token).await;
    assert_eq!(list_items(&orders).len(), 1, "no duplicate order");

    let sid = uuid::Uuid::parse_str(strategy_id).unwrap();
    let mut drain = begin_as(&api.worker, None, "worker").await.unwrap();
    sqlx::query(
        "UPDATE strategy_capital_accounts SET available_cash_usd = 0, reserved_cash_usd = 0 WHERE strategy_id = $1",
    )
    .bind(sid)
    .execute(&mut *drain)
    .await
    .unwrap();
    drain.commit().await.unwrap();
    let broke = TestApi::sample_trade(wallet, "buy", "10", "0xinsufficient");
    api.inject_and_process(&broke).await;
    let mut chk = begin_as(&api.worker, None, "worker").await.unwrap();
    let last: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT status, reason FROM copy_decisions WHERE strategy_id = $1 ORDER BY detected_at DESC LIMIT 1",
    )
    .bind(sid)
    .fetch_optional(&mut *chk)
    .await
    .unwrap();
    chk.commit().await.unwrap();
    assert!(
        matches!(last.as_ref(), Some((s, _)) if s == "rejected" || s == "skipped"),
        "expected broke buy rejection, got {last:?}"
    );
    assert!(
        matches!(&last, Some((_, Some(r))) if r == "insufficient_balance" || r == "strategy_budget"),
        "expected cash rejection, got {last:?}"
    );
    let (_, orders) = api.get_raw("/api/v1/orders", &token).await;
    let filled = list_items(&orders)
        .iter()
        .filter(|o| o.get("status").and_then(|s| s.as_str()) == Some("filled"))
        .count();
    assert_eq!(filled, 1, "only the funded buy should fill");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at a database whose name contains pommarket_test"]
async fn tenant_isolation_api_extended() {
    let api = TestApi::boot().await;
    let admin_login = api
        .post(
            "/api/v1/auth/login",
            json!({"email": "admin@example.com", "password": "admin-password-1"}),
        )
        .await;
    let admin_token = admin_login["access_token"].as_str().unwrap().to_string();

    let pass_a = api
        .invite_user(&admin_token, "tenant-a@example.com", "TenantA")
        .await;
    let pass_b = api
        .invite_user(&admin_token, "tenant-b@example.com", "TenantB")
        .await;
    let token_a = api.login_user("tenant-a@example.com", &pass_a).await;
    let token_b = api.login_user("tenant-b@example.com", &pass_b).await;

    let acct_b = api
        .send(
            "POST",
            "/api/v1/accounts",
            &token_b,
            json!({
                "name": "B acct",
                "signature_type": "eoa",
                "signer_address": "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"
            }),
        )
        .await;
    let acct_b_id = acct_b["id"].as_str().unwrap();
    api.send(
        "PATCH",
        &format!("/api/v1/accounts/{acct_b_id}/risk"),
        &token_b,
        json!({"max_order_usd": "100", "max_exposure_usd": "1000"}),
    )
    .await;
    api.fund_paper(&token_b, acct_b_id, "1000").await;

    let trader_b = api
        .send(
            "POST",
            "/api/v1/source-traders",
            &token_b,
            json!({
                "wallet_address": "0x4444444444444444444444444444444444444444",
                "alias": "B wallet"
            }),
        )
        .await;
    let trader_b_id = trader_b["id"].as_str().unwrap();

    let strat_b = api
        .send(
            "POST",
            "/api/v1/strategies",
            &token_b,
            json!({
                "trading_account_id": acct_b_id,
                "source_trader_id": trader_b_id,
                "sizing_mode": "fixed_amount",
                "fixed_amount_usd": "10",
                "strategy_budget_usd": "50"
            }),
        )
        .await;
    let strat_b_id = strat_b["id"].as_str().unwrap();

    assert_eq!(api.get_status("/api/v1/accounts", &token_a).await, 200);
    let list_a = api.get_raw("/api/v1/accounts", &token_a).await.1;
    assert!(list_a.as_array().unwrap().is_empty());

    assert_eq!(
        api.get_status(&format!("/api/v1/accounts/{acct_b_id}"), &token_a)
            .await,
        404
    );
    assert_eq!(
        api.get_status(&format!("/api/v1/source-traders/{trader_b_id}"), &token_a)
            .await,
        404
    );
    assert_eq!(
        api.get_status(&format!("/api/v1/strategies/{strat_b_id}"), &token_a)
            .await,
        404
    );

    let patch = api
        .http
        .request(
            reqwest::Method::PATCH,
            format!("{}/api/v1/accounts/{acct_b_id}", api.base),
        )
        .header("authorization", format!("Bearer {token_a}"))
        .json(&json!({"name": "stolen"}))
        .send()
        .await
        .unwrap();
    assert_eq!(patch.status(), 404);

    let del = api
        .http
        .request(
            reqwest::Method::DELETE,
            format!("{}/api/v1/accounts/{acct_b_id}", api.base),
        )
        .header("authorization", format!("Bearer {token_a}"))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 404);
}
