#![allow(clippy::disallowed_methods)]

use std::env;

use pommarket::config::Config;
use pommarket::email::{clear_mock_inbox, take_mock_emails, Email, EmailMode};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn set_test_db_env() {
    env::set_var(
        "MIGRATION_DATABASE_URL",
        "postgres://pommarket:p@localhost:5440/pommarket_test",
    );
    env::set_var(
        "APP_DATABASE_URL",
        "postgres://pommarket_app:p@localhost:5440/pommarket_test",
    );
    env::set_var(
        "WORKER_DATABASE_URL",
        "postgres://pommarket_worker:p@localhost:5440/pommarket_test",
    );
    env::set_var("JWT_SECRET", "integration-test-jwt-secret-0123456789abcdef");
    env::set_var(
        "ENCRYPTION_MASTER_KEY",
        "integration-test-master-key-32-chars-min",
    );
    env::set_var("ENVIRONMENT", "test");
}

#[tokio::test]
async fn mock_transport_never_calls_brevo_http() {
    set_test_db_env();
    env::remove_var("EMAIL_MODE");

    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v3/smtp/email"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&mock)
        .await;

    env::set_var("BREVO_API_KEY", "leaked-real-looking-key");
    env::set_var("BREVO_API_BASE", mock.uri());

    let cfg = Config::from_env().unwrap();
    assert!(
        cfg.brevo_api_key.is_none(),
        "test env must ignore BREVO_API_KEY"
    );

    let mail = Email::from_config(&cfg).unwrap();
    assert_eq!(mail.mode(), EmailMode::Mock);
    assert!(mail.enabled());

    clear_mock_inbox();
    mail.send_invitation("invite@test.example", "Invitee", "tok-invite")
        .await
        .unwrap();
    mail.send_password_recovery("reset@test.example", "Reset", "tok-reset")
        .await
        .unwrap();

    let recorded = take_mock_emails();
    assert_eq!(recorded.len(), 2);
    assert!(recorded
        .iter()
        .any(|m| m.to_email == "invite@test.example" && m.subject.contains("invitation")));
    assert!(recorded
        .iter()
        .any(|m| m.to_email == "reset@test.example" && m.subject.contains("password")));

    assert!(
        mock.received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "mock transport must not call Brevo HTTP"
    );

    env::set_var("EMAIL_MODE", "brevo");
    assert!(
        Email::from_config(&cfg).is_err(),
        "EMAIL_MODE=brevo must be rejected in test context"
    );

    env::remove_var("EMAIL_MODE");
    env::remove_var("BREVO_API_KEY");
    env::remove_var("BREVO_API_BASE");
}
