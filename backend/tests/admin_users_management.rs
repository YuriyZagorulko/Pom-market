mod test_support;

use std::env;
use std::net::SocketAddr;

use pommarket::api::{new_app_state, router};
use pommarket::config::Config;
use pommarket::db::{connect, migrate};
use pommarket::polymarket::http::PublicClient;
use pommarket::security::Vault;
use reqwest::Client;
use serde_json::{json, Value};
use test_support::{assert_test_database_url, truncate_users};
use tokio::sync::broadcast;
use uuid::Uuid;

async fn post(http: &Client, base: &str, path: &str, body: Value) -> Value {
    let res = http
        .post(format!("{base}{path}"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success(), "{} {:?}", path, res.status());
    res.json().await.unwrap()
}

async fn send(http: &Client, base: &str, method: &str, path: &str, token: &str, body: Value) -> Value {
    let res = http
        .request(
            method.parse().unwrap(),
            format!("{base}{path}"),
        )
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    res.json().await.unwrap_or(json!({}))
}

async fn send_status(
    http: &Client,
    base: &str,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (reqwest::StatusCode, Value) {
    let mut req = http.request(method.parse().unwrap(), format!("{base}{path}"));
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }
    if let Some(body) = body {
        req = req.json(&body);
    }
    let res = req.send().await.unwrap();
    let status = res.status();
    let text = res.text().await.unwrap();
    let parsed = if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or(json!({ "raw": text }))
    };
    (status, parsed)
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at a database whose name contains pommarket_test"]
async fn admin_user_profile_password_and_rbac() {
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
    let (activity, _) = broadcast::channel(8);
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
    let admin_token = admin["access_token"].as_str().unwrap();

    let created = send(
        &http,
        &base,
        "POST",
        "/api/v1/admin/users",
        admin_token,
        json!({"email": "ada@example.com", "display_name": "Ada"}),
    )
    .await;
    let user_id = created["id"].as_str().unwrap();
    let invite_token = created["setup_url"]
        .as_str()
        .unwrap()
        .split("token=")
        .nth(1)
        .unwrap();
    post(
        &http,
        &base,
        "/api/v1/auth/password/setup",
        json!({"token": invite_token, "password": "ada-password-1"}),
    )
    .await;

    let ada_login = post(
        &http,
        &base,
        "/api/v1/auth/login",
        json!({"email": "ada@example.com", "password": "ada-password-1"}),
    )
    .await;
    let ada_token = ada_login["access_token"].as_str().unwrap();
    let target_user_id = Uuid::parse_str(user_id).unwrap();
    let active_sessions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sessions WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(target_user_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert!(active_sessions >= 1);

    let (status, _) = send_status(
        &http,
        &base,
        "GET",
        &format!("/api/v1/admin/users/{user_id}"),
        Some(ada_token),
        None,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::FORBIDDEN);

    let detail: Value = http
        .get(format!("{base}/api/v1/admin/users/{user_id}"))
        .bearer_auth(admin_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(detail["display_name"], "Ada");
    assert!(detail.get("password_hash").is_none());
    assert!(detail.get("updated_at").is_some());

    send(
        &http,
        &base,
        "PATCH",
        &format!("/api/v1/admin/users/{user_id}"),
        admin_token,
        json!({"display_name": "Ada Lovelace", "email": "ada.lovelace@example.com"}),
    )
    .await;

    let other = send(
        &http,
        &base,
        "POST",
        "/api/v1/admin/users",
        admin_token,
        json!({"email": "grace@example.com", "display_name": "Grace"}),
    )
    .await;
    let grace_id = other["id"].as_str().unwrap();

    let (status, body) = send_status(
        &http,
        &base,
        "PATCH",
        &format!("/api/v1/admin/users/{user_id}"),
        Some(admin_token),
        Some(json!({"email": "grace@example.com"})),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::CONFLICT);
    assert!(body["error"].as_str().unwrap().contains("email"));

    let (status, _) = send_status(
        &http,
        &base,
        "PATCH",
        &format!("/api/v1/admin/users/{}", Uuid::new_v4()),
        Some(admin_token),
        Some(json!({"display_name": "Ghost"})),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);

    let search: Value = http
        .get(format!("{base}/api/v1/admin/users?q=lovelace"))
        .bearer_auth(admin_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(search.as_array().unwrap().len(), 1);

    send(
        &http,
        &base,
        "POST",
        &format!("/api/v1/admin/users/{user_id}/password"),
        admin_token,
        json!({"new_password": "new-ada-pass-1"}),
    )
    .await;

    let active_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sessions WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(target_user_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    assert_eq!(active_after, 0);

    let new_login = post(
        &http,
        &base,
        "/api/v1/auth/login",
        json!({"email": "ada.lovelace@example.com", "password": "new-ada-pass-1"}),
    )
    .await;
    assert!(new_login["access_token"].is_string());

    let _ = grace_id;
    let raw: Value = http
        .get(format!("{base}/api/v1/admin/users/{user_id}"))
        .bearer_auth(admin_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(raw.as_object().unwrap().keys().all(|k| k != "password_hash"));
}