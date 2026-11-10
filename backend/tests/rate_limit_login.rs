mod common;
mod test_support;

use common::TestApi;
use reqwest::StatusCode;
use serde_json::json;
use std::env;

const BUCKET: &str = "192.0.2.90";

async fn login_status(api: &TestApi, xff: Option<&str>) -> StatusCode {
    let mut req = api
        .http
        .post(format!("{}/api/v1/auth/login", api.base))
        .json(&json!({"email": "nobody-rate-limit@example.com", "password": "wrong-password"}));
    if let Some(v) = xff {
        req = req.header("x-forwarded-for", v);
    }
    req.send().await.unwrap().status()
}

async fn login_ok(api: &TestApi, email: &str, password: &str) -> StatusCode {
    api.http
        .post(format!("{}/api/v1/auth/login", api.base))
        .json(&json!({"email": email, "password": password}))
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at a database whose name contains pommarket_test"]
async fn login_rate_limit_enforced_per_trusted_xff_bucket() {
    env::set_var("TRUSTED_PROXIES", "127.0.0.0/8");
    let api = TestApi::boot().await;
    for i in 0..10 {
        let status = login_status(&api, Some(BUCKET)).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "attempt {} should be unauthorized, not limited",
            i + 1
        );
    }
    assert_eq!(
        login_status(&api, Some(BUCKET)).await,
        StatusCode::TOO_MANY_REQUESTS,
        "11th failed attempt on same client IP (via XFF) must be rate limited"
    );
    env::remove_var("TRUSTED_PROXIES");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at a database whose name contains pommarket_test"]
async fn login_rate_limit_ignores_spoofed_xff_without_trusted_proxy() {
    env::remove_var("TRUSTED_PROXIES");
    let api = TestApi::boot().await;
    for _ in 0..10 {
        assert_eq!(
            login_status(&api, Some("192.0.2.91")).await,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        login_status(&api, Some("192.0.2.91")).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        login_status(&api, Some("192.0.2.92")).await,
        StatusCode::TOO_MANY_REQUESTS,
        "new X-Forwarded-For must not bypass limit when peer is not a trusted proxy"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at a database whose name contains pommarket_test"]
async fn login_without_xff_shares_peer_bucket() {
    env::remove_var("TRUSTED_PROXIES");
    let api = TestApi::boot().await;
    for _ in 0..10 {
        assert_eq!(login_status(&api, None).await, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(
        login_status(&api, None).await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at a database whose name contains pommarket_test"]
async fn successful_logins_do_not_consume_ip_failure_budget() {
    env::remove_var("TRUSTED_PROXIES");
    let api = TestApi::boot().await;
    let admin_pass =
        env::var("BOOTSTRAP_ADMIN_PASSWORD").unwrap_or_else(|_| "admin-password-1".into());
    for _ in 0..15 {
        assert_eq!(
            login_ok(&api, "admin@example.com", &admin_pass).await,
            StatusCode::OK,
            "successful logins must not trip the per-IP failure limiter"
        );
    }
    assert_eq!(
        login_status(&api, Some("192.0.2.55")).await,
        StatusCode::UNAUTHORIZED,
        "failed attempts still allowed after many successes"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at a database whose name contains pommarket_test"]
async fn different_emails_share_ip_failure_budget() {
    env::remove_var("TRUSTED_PROXIES");
    let api = TestApi::boot().await;
    for i in 0..10 {
        let email = format!("victim-{i}@example.com");
        assert_eq!(
            login_status_with_email(&api, &email).await,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        login_status_with_email(&api, "fresh-victim@example.com").await,
        StatusCode::TOO_MANY_REQUESTS,
        "changing email must not bypass shared client IP limit"
    );
}

async fn login_status_with_email(api: &TestApi, email: &str) -> StatusCode {
    api.http
        .post(format!("{}/api/v1/auth/login", api.base))
        .json(&json!({"email": email, "password": "wrong-password"}))
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at a database whose name contains pommarket_test"]
async fn forgot_password_uses_same_client_ip_policy() {
    env::set_var("TRUSTED_PROXIES", "127.0.0.0/8");
    let api = TestApi::boot().await;
    let url = format!("{}/api/auth/forgot-password", api.base);
    for _ in 0..5 {
        let status = api
            .http
            .post(&url)
            .header("x-forwarded-for", "198.51.100.80")
            .json(&json!({"email":"missing@example.com"}))
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, StatusCode::OK);
    }
    assert_eq!(
        api.http
            .post(&url)
            .header("x-forwarded-for", "198.51.100.80")
            .json(&json!({"email":"missing@example.com"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    env::remove_var("TRUSTED_PROXIES");
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at a database whose name contains pommarket_test"]
async fn refresh_rate_limit_applies_per_peer_ip() {
    env::remove_var("TRUSTED_PROXIES");
    let api = TestApi::boot().await;
    let url = format!("{}/api/v1/auth/refresh", api.base);
    for _ in 0..120 {
        let status = api.http.post(&url).send().await.unwrap().status();
        assert!(
            status == StatusCode::UNAUTHORIZED || status == StatusCode::TOO_MANY_REQUESTS,
            "unexpected status {status}"
        );
        if status == StatusCode::TOO_MANY_REQUESTS {
            return;
        }
    }
    assert_eq!(
        api.http.post(&url).send().await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}
