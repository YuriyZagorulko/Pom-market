use std::collections::HashMap;
use std::env;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use pommarket::api::{router, AppState};
use pommarket::config::Config;
use pommarket::db::{connect, migrate};
use pommarket::email::Email;
use pommarket::polymarket::http::PublicClient;
use pommarket::security::{random_token, sha256_hex, Vault};
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at a database whose name contains pommarket_test"]
async fn invitation_recovery_verification_and_token_rules() {
    let migration_url = env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert!(migration_url.contains("pommarket_test"));

    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v3/smtp/email"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"messageId": "ok"})))
        .mount(&mock)
        .await;

    env::set_var("BREVO_API_KEY", "test-brevo-key");
    env::set_var("BREVO_API_BASE", mock.uri());
    env::set_var("BREVO_SENDER_EMAIL", "noreply@example.com");
    env::set_var("BREVO_SENDER_NAME", "POM Market");
    env::set_var("APP_BASE_URL", "http://localhost:4323");
    env::set_var("BOOTSTRAP_ADMIN_EMAIL", "admin@example.com");
    env::set_var("BOOTSTRAP_ADMIN_PASSWORD", "admin-password-1");

    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    pommarket::ensure_login_roles(
        &migration,
        &env::var("APP_DB_PASSWORD").unwrap(),
        &env::var("WORKER_DB_PASSWORD").unwrap(),
    )
    .await
    .unwrap();
    sqlx::query("TRUNCATE users RESTART IDENTITY CASCADE")
        .execute(&migration)
        .await
        .unwrap();
    let cfg = Config::from_env().unwrap();
    pommarket::bootstrap_admin(&migration, &cfg).await.unwrap();

    let pool = connect(&cfg.app_database_url).await.unwrap();
    let mail = Email::from_config(&cfg).unwrap();
    assert!(mail.enabled());
    let (activity, _) = broadcast::channel(8);
    let state = AppState {
        pool: pool.clone(),
        cfg: cfg.clone(),
        vault: Vault::new(&cfg.encryption_master_key, &cfg.encryption_key_id, None, None).unwrap(),
        polymarket: PublicClient::new(cfg.data_api_host.clone(), cfg.clob_host.clone(), cfg.gamma_host.clone()),
        activity,
        login_attempts: Arc::new(Mutex::new(HashMap::new())),
        recovery_attempts: Arc::new(Mutex::new(HashMap::new())),
        verify_attempts: Arc::new(Mutex::new(HashMap::new())),
        mail,
    };
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let http = Client::builder().build().unwrap();
    let base = format!("http://{addr}");

    let admin = post(&http, &base, "/api/v1/auth/login", json!({"email":"admin@example.com","password":"admin-password-1"})).await;
    let admin_token = admin["access_token"].as_str().unwrap();
    let created = send(
        &http,
        &base,
        "POST",
        "/api/v1/admin/users",
        admin_token,
        json!({"email":"ada@example.com","display_name":"Ada"}),
    )
    .await;
    let invite_token = created["setup_url"].as_str().unwrap().split("token=").nth(1).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(mock.received_requests().await.unwrap_or_default().len() >= 1);

    post(
        &http,
        &base,
        "/api/v1/auth/password/setup",
        json!({"token": invite_token, "password": "ada-password-1"}),
    )
    .await;
    assert_status(
        &http,
        &base,
        "/api/v1/auth/password/setup",
        json!({"token": invite_token, "password": "ada-password-2"}),
        false,
    )
    .await;

    post(
        &http,
        &base,
        "/api/auth/forgot-password",
        json!({"email":"ada@example.com"}),
    )
    .await;
    post(
        &http,
        &base,
        "/api/auth/forgot-password",
        json!({"email":"missing@example.com"}),
    )
    .await;

    let expired = random_token();
    sqlx::query(
        "INSERT INTO invitation_tokens (user_id, token_hash, purpose, expires_at)
         SELECT id, $1, 'reset', $2 FROM users WHERE email = 'ada@example.com'",
    )
    .bind(sha256_hex(&expired))
    .bind(Utc::now() - ChronoDuration::hours(1))
    .execute(&pool)
    .await
    .unwrap();
    assert_status(
        &http,
        &base,
        "/api/v1/auth/password/setup",
        json!({"token": expired, "password": "ada-password-3"}),
        false,
    )
    .await;

    let login = post(
        &http,
        &base,
        "/api/v1/auth/login",
        json!({"email":"ada@example.com","password":"ada-password-1"}),
    )
    .await;
    let token = login["access_token"].as_str().unwrap();
    send(&http, &base, "POST", "/api/v1/auth/email/verify/request", token, Value::Null).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let verify_token = latest_token_from_mock(&mock).await;
    post(
        &http,
        &base,
        "/api/v1/auth/email/verify/confirm",
        json!({"token": verify_token}),
    )
    .await;
    let verified: Option<chrono::DateTime<Utc>> =
        sqlx::query_scalar("SELECT email_verified_at FROM users WHERE email = 'ada@example.com'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(verified.is_some());
}

async fn latest_token_from_mock(mock: &MockServer) -> String {
    let requests = mock.received_requests().await.unwrap_or_default();
    let raw = requests.last().expect("brevo request").body.clone();
    let body: Value = serde_json::from_slice(&raw).unwrap();
    let html = body["htmlContent"].as_str().expect("htmlContent");
    html.split("token=")
        .nth(1)
        .expect("verify link")
        .split(&['"', '<', ' '][..])
        .next()
        .unwrap()
        .to_string()
}

async fn post(http: &Client, base: &str, path: &str, body: Value) -> Value {
    let response = http.post(format!("{base}{path}")).json(&body).send().await.unwrap();
    assert!(response.status().is_success(), "{path} -> {}", response.status());
    serde_json::from_str(&response.text().await.unwrap()).unwrap()
}

async fn assert_status(http: &Client, base: &str, path: &str, body: Value, should_succeed: bool) {
    let response = http.post(format!("{base}{path}")).json(&body).send().await.unwrap();
    assert_eq!(response.status().is_success(), should_succeed, "{path}");
}

async fn send(http: &Client, base: &str, method: &str, path: &str, token: &str, body: Value) -> Value {
    let response = http
        .request(method.parse().unwrap(), format!("{base}{path}"))
        .header("authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "{method} {path}");
    let text = response.text().await.unwrap();
    if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap()
    }
}
