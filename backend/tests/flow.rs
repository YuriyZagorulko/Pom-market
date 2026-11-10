mod test_support;

use std::env;
use std::net::SocketAddr;

use test_support::{assert_all_test_database_urls, list_items, truncate_integration_tables};

use async_trait::async_trait;
use chrono::Utc;
use pommarket::api::{new_app_state, router};
use pommarket::config::Config;
use pommarket::db::{begin_as, connect, migrate};
use pommarket::engine::{BookLevel, OrderBook};
use pommarket::pipeline::{ingest, process_event, reconcile_live_orders, Books, PipelineConfig};
use pommarket::polymarket::http::PublicClient;
use pommarket::polymarket::RawTrade;
use pommarket::security::Vault;
use reqwest::Client;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use uuid::Uuid;

struct FixedBook;

#[async_trait]
impl Books for FixedBook {
    async fn book(&self, _token_id: &str) -> Result<OrderBook, String> {
        Ok(OrderBook {
            bids: vec![BookLevel {
                price: Decimal::new(50, 2),
                size: Decimal::from(500),
            }],
            asks: vec![BookLevel {
                price: Decimal::new(50, 2),
                size: Decimal::from(500),
            }],
            tick: Decimal::new(1, 2),
            min_order_size: Decimal::ZERO,
        })
    }
    async fn fee(&self, _token_id: &str) -> Option<pommarket::engine::FeeSchedule> {
        Some(pommarket::engine::FeeSchedule {
            fees_enabled: true,
            rate: Decimal::new(7, 2),
            exponent: 1,
            taker_only: true,
            rebate_rate: Some(Decimal::new(2, 1)),
            source: "test".into(),
        })
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at a database whose name contains pommarket_test"]
async fn paper_workflow_and_tenant_isolation() {
    assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    pommarket::ensure_login_roles(
        &migration,
        &env::var("APP_DB_PASSWORD").unwrap(),
        &env::var("WORKER_DB_PASSWORD").unwrap(),
    )
    .await
    .unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    pommarket::bootstrap_admin(&migration, &Config::from_env().unwrap())
        .await
        .unwrap();

    let cfg = Config::from_env().unwrap();
    let pool = connect(&cfg.app_database_url).await.unwrap();
    let worker = connect(&cfg.worker_database_url).await.unwrap();
    let (activity, _) = broadcast::channel(16);
    let mail = pommarket::email::Email::from_config(&cfg).unwrap();
    let state = new_app_state(
        pool.clone(),
        cfg.clone(),
        Vault::new(
            &cfg.encryption_master_key,
            &cfg.encryption_key_id,
            None,
            None,
        )
        .unwrap(),
        PublicClient::new(
            cfg.data_api_host.clone(),
            cfg.clob_host.clone(),
            cfg.gamma_host.clone(),
        ),
        activity,
        mail,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let http = Client::builder().build().unwrap();
    let base = format!("http://{addr}");

    let admin = post(
        &http,
        &base,
        "/api/v1/auth/login",
        json!({"email": "admin@example.com", "password": "admin-password-1"}),
    )
    .await;
    let admin_token = admin["access_token"].as_str().unwrap().to_string();
    let created = send(
        &http,
        &base,
        "POST",
        "/api/v1/admin/users",
        &admin_token,
        json!({"email": "ada@example.com", "display_name": "Ada"}),
    )
    .await;
    assert!(created["setup_url"].as_str().unwrap().contains("token="));
    assert!(created.get("private_key").is_none());
    let token = created["setup_url"]
        .as_str()
        .unwrap()
        .split("token=")
        .nth(1)
        .unwrap();
    post(
        &http,
        &base,
        "/api/v1/auth/password/setup",
        json!({"token": token, "password": "ada-password-1"}),
    )
    .await;
    let ada = post(
        &http,
        &base,
        "/api/v1/auth/login",
        json!({"email": "ada@example.com", "password": "ada-password-1"}),
    )
    .await;
    let ada_token = ada["access_token"].as_str().unwrap().to_string();
    assert_eq!(ada["user"]["role"], "user");

    let other = send(
        &http,
        &base,
        "POST",
        "/api/v1/admin/users",
        &admin_token,
        json!({"email": "grace@example.com", "display_name": "Grace"}),
    )
    .await;
    let other_token_raw = other["setup_url"]
        .as_str()
        .unwrap()
        .split("token=")
        .nth(1)
        .unwrap();
    post(
        &http,
        &base,
        "/api/v1/auth/password/setup",
        json!({"token": other_token_raw, "password": "grace-password-1"}),
    )
    .await;
    let grace = post(
        &http,
        &base,
        "/api/v1/auth/login",
        json!({"email": "grace@example.com", "password": "grace-password-1"}),
    )
    .await;
    let grace_token = grace["access_token"].as_str().unwrap().to_string();

    let key = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
    let account = send(
        &http,
        &base,
        "POST",
        "/api/v1/accounts",
        &ada_token,
        json!({
            "name": "Ada main",
            "signature_type": "eoa",
            "signer_address": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
            "private_key": key
        }),
    )
    .await;
    let account_id = account["id"].as_str().unwrap();
    let probe = send(
        &http,
        &base,
        "POST",
        &format!("/api/v1/accounts/{account_id}/test"),
        &ada_token,
        json!({}),
    )
    .await;
    assert_eq!(probe["address_valid"], true);
    assert!(probe.get("private_key").is_none());
    let body = probe.to_string();
    assert!(!body.contains(&key[2..]));

    send(
        &http,
        &base,
        "PATCH",
        &format!("/api/v1/accounts/{account_id}/risk"),
        &ada_token,
        json!({"paper_cash_usd": "1000", "max_order_usd": "100", "max_exposure_usd": "1000"}),
    )
    .await;
    send(
        &http,
        &base,
        "POST",
        &format!("/api/v1/accounts/{account_id}/mode"),
        &ada_token,
        json!({"mode": "paper_trading"}),
    )
    .await;

    let trader = send(
        &http,
        &base,
        "POST",
        "/api/v1/source-traders",
        &ada_token,
        json!({
            "wallet_address": "0x1111111111111111111111111111111111111111",
            "alias": "Leader"
        }),
    )
    .await;
    let trader_id = trader["id"].as_str().unwrap();
    let strategy = send(
        &http,
        &base,
        "POST",
        "/api/v1/strategies",
        &ada_token,
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
    send(
        &http,
        &base,
        "POST",
        &format!("/api/v1/strategies/{strategy_id}/enable"),
        &ada_token,
        json!({"enabled": true}),
    )
    .await;

    let grace_accounts = send(
        &http,
        &base,
        "GET",
        "/api/v1/accounts",
        &grace_token,
        Value::Null,
    )
    .await;
    assert!(grace_accounts.as_array().unwrap().is_empty());
    let stolen = http
        .get(format!("{base}/api/v1/accounts/{account_id}"))
        .header("authorization", format!("Bearer {grace_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(stolen.status(), 404);
    let admin_accounts = http
        .get(format!("{base}/api/v1/accounts"))
        .header("authorization", format!("Bearer {admin_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_accounts.status(), 403);

    let trade = sample_trade("buy", "10");
    let outcome = ingest(&worker, &trade).await.unwrap();
    assert!(outcome.created);
    let pipeline = PipelineConfig {
        clob_host: cfg.clob_host.clone(),
        confirm_live: false,
        default_fee_rate: Decimal::ZERO,
        default_fee_exponent: 1,
    };
    let vault = Vault::new(
        &cfg.encryption_master_key,
        &cfg.encryption_key_id,
        None,
        None,
    )
    .unwrap();
    process_event(&worker, &vault, &FixedBook, &pipeline, outcome.event_id)
        .await
        .unwrap();
    let again = ingest(&worker, &trade).await.unwrap();
    assert!(!again.created);
    process_event(&worker, &vault, &FixedBook, &pipeline, outcome.event_id)
        .await
        .unwrap();

    let decisions = send(
        &http,
        &base,
        "GET",
        "/api/v1/decisions",
        &ada_token,
        Value::Null,
    )
    .await;
    assert_eq!(list_items(&decisions).len(), 1);
    assert_eq!(list_items(&decisions)[0]["status"], "approved");
    let orders = send(
        &http,
        &base,
        "GET",
        "/api/v1/orders",
        &ada_token,
        Value::Null,
    )
    .await;
    let order_rows = list_items(&orders);
    assert_eq!(order_rows[0]["operating_mode"], "paper_trading");
    assert_eq!(order_rows[0]["status"], "filled");
    let grace_orders = send(
        &http,
        &base,
        "GET",
        "/api/v1/orders",
        &grace_token,
        Value::Null,
    )
    .await;
    assert!(list_items(&grace_orders).is_empty());
    let detail = send(
        &http,
        &base,
        "GET",
        &format!("/api/v1/trades/{}", outcome.event_id),
        &ada_token,
        Value::Null,
    )
    .await;
    assert!(!detail["timeline"].as_array().unwrap().is_empty());
    assert!(detail["fills"].as_array().unwrap()[0]
        .get("fee_status")
        .is_some());

    let sell = sample_trade("sell", "10");
    let sell_outcome = ingest(&worker, &sell).await.unwrap();
    process_event(
        &worker,
        &vault,
        &FixedBook,
        &pipeline,
        sell_outcome.event_id,
    )
    .await
    .unwrap();
    let positions = send(
        &http,
        &base,
        "GET",
        "/api/v1/positions",
        &ada_token,
        Value::Null,
    )
    .await;
    let position_rows = list_items(&positions);
    assert!(
        position_rows.is_empty()
            || position_rows
                .iter()
                .all(|row| row["open_shares"].as_str().unwrap().starts_with('0'))
    );

    let mut tx = begin_as(&worker, None, "worker").await.unwrap();
    let order_id: Uuid = sqlx::query_scalar(
        "UPDATE orders SET operating_mode = 'live_trading', status = 'submitting', updated_at = now() - interval '10 minutes'
         WHERE id = (SELECT id FROM orders ORDER BY created_at LIMIT 1)
         RETURNING id",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    reconcile_live_orders(&worker, &vault, "https://clob.polymarket.com", 30)
        .await
        .unwrap();
    let status: String = {
        let mut tx = begin_as(&worker, None, "worker").await.unwrap();
        sqlx::query_scalar("SELECT status FROM orders WHERE id = $1")
            .bind(order_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap()
    };
    assert_eq!(status, "unknown");
    let mut tx = begin_as(&worker, None, "worker").await.unwrap();
    sqlx::query("UPDATE orders SET updated_at = now() - interval '10 minutes' WHERE id = $1")
        .bind(order_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    reconcile_live_orders(&worker, &vault, "https://clob.polymarket.com", 30)
        .await
        .unwrap();
    let status: String = {
        let mut tx = begin_as(&worker, None, "worker").await.unwrap();
        sqlx::query_scalar("SELECT status FROM orders WHERE id = $1")
            .bind(order_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap()
    };
    assert_eq!(status, "unknown");
    let _ = Utc::now();
}

fn sample_trade(side: &str, size: &str) -> RawTrade {
    let payload = json!({
        "proxy_wallet": "0x1111111111111111111111111111111111111111",
        "transaction_hash": format!("0x{}", side),
        "token_id": "55",
        "condition_id": "cond",
        "side": side,
        "price": "0.50",
        "size": size,
        "timestamp": Utc::now().timestamp(),
        "title": "Test market",
        "slug": "test-market"
    });
    pommarket::polymarket::parse_activity_payload(&payload, "test")
        .pop()
        .unwrap()
}

async fn post(http: &Client, base: &str, path: &str, body: Value) -> Value {
    let response = http
        .post(format!("{base}{path}"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert!(status.is_success(), "{path} -> {status} {text}");
    serde_json::from_str(&text).unwrap()
}

async fn send(
    http: &Client,
    base: &str,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> Value {
    let response = http
        .request(method.parse().unwrap(), format!("{base}{path}"))
        .header("authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert!(status.is_success(), "{method} {path} -> {status} {text}");
    if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap()
    }
}
