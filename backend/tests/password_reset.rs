mod test_support;

use std::env;
use std::net::SocketAddr;
use std::time::Duration;
use test_support::{assert_test_database_url, truncate_users};

use chrono::{Duration as ChronoDuration, Utc};
use pommarket::api::{new_app_state, router, FORGOT_PASSWORD_MESSAGE};
use pommarket::auth;
use pommarket::config::Config;
use pommarket::db::{connect, migrate};
use pommarket::email::{clear_mock_inbox, mock_inbox, Email, EmailMode};
use pommarket::polymarket::http::PublicClient;
use pommarket::security::{hash_password, random_token, sha256_hex, verify_password, Vault};
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::broadcast;

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at a database whose name contains pommarket_test"]
async fn password_reset_flow_security_matrix() {
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
    env::set_var("PASSWORD_RESET_TTL_MINUTES", "30");
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
    let hash = hash_password("ada-password-1").unwrap();
    sqlx::query(
        "INSERT INTO users (email, display_name, password_hash, role, status) VALUES ('ada@example.com', 'Ada', $1, 'user', 'active')",
    )
    .bind(hash)
    .execute(&migration)
    .await
    .unwrap();

    let mail = Email::from_config(&cfg).unwrap();
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

    tokio::time::sleep(Duration::from_millis(150)).await;
    let reset_mail = wait_for_mock_subject("Reset your POM Market password").await;
    let token = token_from_reset_html(&reset_mail.html);

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

    env::set_var("EMAIL_MODE", "brevo");
    assert!(
        Email::from_config(&cfg).is_err(),
        "real email mode must be rejected in integration test environment"
    );
    env::remove_var("EMAIL_MODE");

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

fn token_from_reset_html(html: &str) -> String {
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
