mod test_support;

use std::env;
use std::net::SocketAddr;
use std::time::Duration;
use test_support::{assert_test_database_url, truncate_users};

use chrono::{Duration as ChronoDuration, Utc};
use pommarket::api::{new_app_state, router};
use pommarket::config::Config;
use pommarket::db::{connect, migrate};
use pommarket::email::{clear_mock_inbox, mock_inbox, Email, EmailMode};
use pommarket::polymarket::http::PublicClient;
use pommarket::security::{random_token, sha256_hex, Vault};
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::broadcast;

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at a database whose name contains pommarket_test"]
async fn invitation_recovery_verification_and_token_rules() {
    let migration_url = env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&migration_url);

    env::set_var("ENVIRONMENT", "test");
    env::set_var("EMAIL_MODE", "mock");
    env::set_var("BREVO_API_KEY", "must-not-be-used");
    env::set_var("BREVO_SENDER_EMAIL", "noreply@example.com");
    env::set_var("BREVO_SENDER_NAME", "POM Market");
    env::set_var("APP_BASE_URL", "http://localhost:4323");
    env::set_var("BOOTSTRAP_ADMIN_EMAIL", "admin@example.com");
    env::set_var("BOOTSTRAP_ADMIN_PASSWORD", "admin-password-1");
    clear_mock_inbox();

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
    let mail = Email::from_config(&cfg).unwrap();
    assert!(mail.enabled());
    assert_eq!(mail.mode(), EmailMode::Mock);
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

    let admin = post(
        &http,
        &base,
        "/api/v1/auth/login",
        json!({"email":"admin@example.com","password":"admin-password-1"}),
    )
    .await;
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
    let invite_token = created["setup_url"]
        .as_str()
        .unwrap()
        .split("token=")
        .nth(1)
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let invite_mail = wait_for_mock_subject("invitation").await;
    assert_eq!(invite_mail.to_email, "ada@example.com");
    assert!(invite_mail.html.contains("/setup?token="));

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
    send(
        &http,
        &base,
        "POST",
        "/api/v1/auth/email/verify/request",
        token,
        Value::Null,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let verify_mail = wait_for_mock_subject("Verify").await;
    let verify_token = token_from_html(&verify_mail.html);
    post(
        &http,
        &base,
        "/api/v1/auth/email/verify/confirm",
        json!({"token": verify_token}),
    )
    .await;
    let verified: Option<chrono::DateTime<Utc>> =
        sqlx::query_scalar("SELECT email_verified_at FROM users WHERE email = 'ada@example.com'")
            .fetch_one(&migration)
            .await
            .unwrap();
    assert!(verified.is_some());
}

async fn wait_for_mock_subject(subject_needle: &str) -> pommarket::email::RecordedEmail {
    for _ in 0..80 {
        let inbox = mock_inbox();
        let found = {
            let guard = inbox.lock().expect("mock inbox");
            guard
                .iter()
                .rev()
                .find(|e| e.subject.contains(subject_needle))
                .cloned()
        };
        if let Some(email) = found {
            return email;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("mock inbox missing email with subject containing {subject_needle:?}");
}

fn token_from_html(html: &str) -> String {
    html.split("token=")
        .nth(1)
        .expect("verify link")
        .split(&['"', '<', ' '][..])
        .next()
        .unwrap()
        .to_string()
}

async fn post(http: &Client, base: &str, path: &str, body: Value) -> Value {
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
    assert!(response.status().is_success(), "{method} {path}");
    let text = response.text().await.unwrap();
    if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap()
    }
}
