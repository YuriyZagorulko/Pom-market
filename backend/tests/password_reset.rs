mod test_support;

use std::env;
use std::net::SocketAddr;
use test_support::{assert_test_database_url, truncate_users};

use chrono::{Duration as ChronoDuration, Utc};
use pommarket::api::{new_app_state, router, FORGOT_PASSWORD_MESSAGE};
use pommarket::auth;
use pommarket::config::Config;
use pommarket::db::{connect, migrate};
use pommarket::email::Email;
use pommarket::polymarket::http::PublicClient;
use pommarket::security::{hash_password, random_token, sha256_hex, verify_password, Vault};
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at a database whose name contains pommarket_test"]
async fn password_reset_flow_security_matrix() {
    let migration_url = env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&migration_url);

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
    env::set_var("PASSWORD_RESET_TTL_MINUTES", "30");

    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    pommarket::ensure_login_roles(
        &migration,
        &env::var("APP_DB_PASSWORD").unwrap(),
        &env::var("WORKER_DB_PASSWORD").unwrap(),
    )
    .await
    .unwrap();
    truncate_users(&migration, &migration_url).await;
    let cfg = Config::from_env().unwrap();
    pommarket::bootstrap_admin(&migration, &cfg).await.unwrap();

    let pool = connect(&cfg.app_database_url).await.unwrap();
    let hash = hash_password("ada-password-1").unwrap();
    sqlx::query(
        "INSERT INTO users (email, display_name, password_hash, role, status) VALUES ('ada@example.com', 'Ada', $1, 'user', 'active')",
    )
    .bind(hash)
    .execute(&migration)
    .await
    .unwrap();

    let mail = Email::from_config(&cfg).unwrap();
    let (activity, _) = broadcast::channel(8);
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

    let missing = post_json(
        &http,
        &base,
        "/api/auth/forgot-password",
        json!({"email":"missing@example.com"}),
    )
    .await;
    let existing = post_json(
        &http,
        &base,
        "/api/auth/forgot-password",
        json!({"email":"ada@example.com"}),
    )
    .await;
    assert_eq!(missing["message"], existing["message"]);
    assert_eq!(missing["message"], FORGOT_PASSWORD_MESSAGE);

    let invalid = http
        .post(format!("{base}/api/auth/forgot-password"))
        .json(&json!({"email":"not-an-email"}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), 400);

    let token = latest_reset_token(&mock).await;

    let login = post_json(
        &http,
        &base,
        "/api/v1/auth/login",
        json!({"email":"ada@example.com","password":"ada-password-1"}),
    )
    .await;
    assert!(login["access_token"].is_string());

    post_json(
        &http,
        &base,
        "/api/auth/reset-password",
        json!({"token": token, "new_password": "ada-password-2"}),
    )
    .await;

    let stored: String =
        sqlx::query_scalar("SELECT password_hash FROM users WHERE email = 'ada@example.com'")
            .fetch_one(&migration)
            .await
            .unwrap();
    assert!(verify_password("ada-password-2", &stored));

    let active_sessions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sessions WHERE user_id = (SELECT id FROM users WHERE email = 'ada@example.com') AND revoked_at IS NULL",
    )
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(active_sessions, 0);

    assert_status(
        &http,
        &base,
        "/api/auth/reset-password",
        json!({"token": token, "new_password": "ada-password-3"}),
        false,
    )
    .await;

    let short = http
        .post(format!("{base}/api/auth/reset-password"))
        .json(&json!({"token": random_token(), "new_password": "short"}))
        .send()
        .await
        .unwrap();
    assert_eq!(short.status(), 400);

    let expired = random_token();
    sqlx::query(
        "INSERT INTO invitation_tokens (user_id, token_hash, purpose, expires_at)
         SELECT id, $1, 'reset', $2 FROM users WHERE email = 'ada@example.com'",
    )
    .bind(sha256_hex(&expired))
    .bind(Utc::now() - ChronoDuration::minutes(1))
    .execute(&migration)
    .await
    .unwrap();
    assert_status(
        &http,
        &base,
        "/api/auth/reset-password",
        json!({"token": expired, "new_password": "ada-password-4"}),
        false,
    )
    .await;

    let race_token = random_token();
    sqlx::query(
        "INSERT INTO invitation_tokens (user_id, token_hash, purpose, expires_at)
         SELECT id, $1, 'reset', now() + interval '30 minutes' FROM users WHERE email = 'ada@example.com'",
    )
    .bind(sha256_hex(&race_token))
    .execute(&migration)
    .await
    .unwrap();
    let (first, second) = tokio::join!(
        http.post(format!("{base}/api/auth/reset-password"))
            .json(&json!({"token": race_token, "new_password": "ada-password-5"}))
            .send(),
        http.post(format!("{base}/api/auth/reset-password"))
            .json(&json!({"token": race_token, "new_password": "ada-password-6"}))
            .send(),
    );
    let ok_count = [
        first.unwrap().status().is_success(),
        second.unwrap().status().is_success(),
    ]
    .into_iter()
    .filter(|v| *v)
    .count();
    assert_eq!(ok_count, 1);

    let fail_mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v3/smtp/email"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&fail_mock)
        .await;
    env::set_var("BREVO_API_BASE", fail_mock.uri());
    let fail_cfg = Config::from_env().unwrap();
    let fail_mail = Email::from_config(&fail_cfg).unwrap();
    assert!(fail_mail
        .send_password_recovery("ada@example.com", "Ada", "token-value")
        .await
        .is_err());
    let still_ok = post_json(
        &http,
        &base,
        "/api/auth/forgot-password",
        json!({"email":"ada@example.com"}),
    )
    .await;
    assert_eq!(still_ok["message"], FORGOT_PASSWORD_MESSAGE);

    let _ = auth::consume_password_reset(&pool, &random_token(), "another-password-1").await;
}

async fn latest_reset_token(mock: &MockServer) -> String {
    let requests = mock.received_requests().await.unwrap_or_default();
    let raw = requests.last().expect("brevo call").body.clone();
    let body: Value = serde_json::from_slice(&raw).unwrap();
    let html = body["htmlContent"].as_str().expect("html");
    assert!(html.contains("/reset-password?token="));
    html.split("token=")
        .nth(1)
        .expect("token")
        .split(&['"', '<', ' '][..])
        .next()
        .unwrap()
        .to_string()
}

async fn post_json(http: &Client, base: &str, path: &str, body: Value) -> Value {
    let response = http
        .post(format!("{base}{path}"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "{path} -> {}",
        response.status()
    );
    serde_json::from_str(&response.text().await.unwrap()).unwrap()
}

async fn assert_status(http: &Client, base: &str, path: &str, body: Value, should_succeed: bool) {
    let response = http
        .post(format!("{base}{path}"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().is_success(), should_succeed, "{path}");
}
