pub mod brevo;
pub mod console;
pub mod mock;
mod mode;

use std::sync::Arc;

use async_trait::async_trait;
use secrecy::SecretString;
use thiserror::Error;

use crate::config::Config;
use crate::security::redact;

pub use mock::{clear_mock_inbox, mock_inbox, take_mock_emails, RecordedEmail};
pub use mode::{automated_test_context, email_mode_from_env, resolve_email_mode, EmailMode};

#[derive(Debug, Error)]
pub enum MailError {
    #[error("email delivery timed out")]
    Timeout,
    #[error("email transport failed: {0}")]
    Transport(String),
    #[error("email API returned HTTP {status}")]
    Api { status: u16, detail: String },
    #[error("email is not configured")]
    NotConfigured,
}

impl MailError {
    pub fn public_message(&self) -> String {
        match self {
            MailError::NotConfigured => {
                "Email is not configured on the server (set BREVO_API_KEY). Use the invitation link below.".into()
            }
            MailError::Timeout => {
                "The email provider timed out. Check spam or use the invitation link below.".into()
            }
            MailError::Transport(_) => {
                "Could not reach the email provider. Use the invitation link below.".into()
            }
            MailError::Api { status, detail } => {
                let lower = detail.to_lowercase();
                if lower.contains("sender") {
                    "Brevo rejected the sender address. Verify BREVO_SENDER_EMAIL in Brevo.".into()
                } else {
                    format!("Email provider error (HTTP {status}). Use the invitation link below.")
                }
            }
        }
    }
}

#[async_trait]
pub trait Mailer: Send + Sync {
    async fn send(
        &self,
        sender_name: &str,
        sender_email: &str,
        to_email: &str,
        to_name: &str,
        subject: &str,
        html: &str,
    ) -> Result<(), MailError>;
}

struct NoopMailer;

#[async_trait]
impl Mailer for NoopMailer {
    async fn send(
        &self,
        _sender_name: &str,
        _sender_email: &str,
        _to_email: &str,
        _to_name: &str,
        _subject: &str,
        _html: &str,
    ) -> Result<(), MailError> {
        Err(MailError::NotConfigured)
    }
}

#[derive(Clone)]
pub struct Email {
    mailer: Arc<dyn Mailer>,
    sender_email: String,
    sender_name: String,
    app_base_url: String,
    enabled: bool,
    mode: EmailMode,
}

impl Email {
    pub fn from_config(cfg: &Config) -> anyhow::Result<Self> {
        let explicit = email_mode_from_env();
        let resolved = match resolve_email_mode(cfg, explicit) {
            Ok(mode) => mode,
            Err(mode::ResolveEmailModeError::BrevoForbiddenInTests) => {
                return Err(anyhow::anyhow!(
                    "EMAIL_MODE=brevo is forbidden in automated test environments"
                ));
            }
            Err(mode::ResolveEmailModeError::Unconfigured) => {
                return Ok(Self {
                    mailer: Arc::new(NoopMailer),
                    sender_email: cfg.brevo_sender_email.clone(),
                    sender_name: cfg.brevo_sender_name.clone(),
                    app_base_url: cfg.app_base_url.clone(),
                    enabled: false,
                    mode: EmailMode::Console,
                });
            }
        };

        let (mailer, enabled) = match resolved {
            EmailMode::Mock => (mock::MockMailer::arc(), true),
            EmailMode::Console => (Arc::new(console::ConsoleMailer) as Arc<dyn Mailer>, true),
            EmailMode::Brevo => {
                let key = cfg
                    .brevo_api_key
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("EMAIL_MODE=brevo requires BREVO_API_KEY"))?;
                (
                    brevo::BrevoMailer::arc(key, cfg.brevo_api_base.clone())?,
                    true,
                )
            }
        };

        Ok(Self {
            mailer,
            sender_email: cfg.brevo_sender_email.clone(),
            sender_name: cfg.brevo_sender_name.clone(),
            app_base_url: cfg.app_base_url.clone(),
            enabled,
            mode: resolved,
        })
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn mode(&self) -> EmailMode {
        self.mode
    }

    pub fn setup_url(&self, token: &str) -> String {
        format!(
            "{}/setup?token={token}",
            self.app_base_url.trim_end_matches('/')
        )
    }

    pub fn verify_url(&self, token: &str) -> String {
        format!(
            "{}/verify-email?token={token}",
            self.app_base_url.trim_end_matches('/')
        )
    }

    pub fn reset_password_url(&self, token: &str) -> String {
        format!(
            "{}/reset-password?token={token}",
            self.app_base_url.trim_end_matches('/')
        )
    }

    pub async fn send_invitation(
        &self,
        email: &str,
        display_name: &str,
        token: &str,
    ) -> Result<(), MailError> {
        let url = self.setup_url(token);
        let html = format!(
            "<p>Hello {display_name},</p><p>You have been invited to POM Market. \
             <a href=\"{url}\">Accept your invitation</a> and set your password. \
             This link expires in a few days and works only once.</p>"
        );
        self.deliver(email, display_name, "Your POM Market invitation", &html)
            .await
    }

    pub async fn send_password_recovery(
        &self,
        email: &str,
        display_name: &str,
        token: &str,
    ) -> Result<(), MailError> {
        let url = self.reset_password_url(token);
        let html = password_reset_email_html(display_name, &url);
        self.deliver(email, display_name, "Reset your POM Market password", &html)
            .await
    }

    pub async fn send_email_verification(
        &self,
        email: &str,
        display_name: &str,
        token: &str,
    ) -> Result<(), MailError> {
        let url = self.verify_url(token);
        let html = format!(
            "<p>Hello {display_name},</p><p>Please <a href=\"{url}\">verify your email address</a> for POM Market.</p>"
        );
        self.deliver(email, display_name, "Verify your POM Market email", &html)
            .await
    }

    pub async fn send_password_changed(
        &self,
        email: &str,
        display_name: &str,
    ) -> Result<(), MailError> {
        let html = format!(
            "<p>Hello {display_name},</p><p>Your POM Market password was changed. \
             If this was not you, contact an administrator immediately.</p>"
        );
        self.deliver(
            email,
            display_name,
            "Your POM Market password was changed",
            &html,
        )
        .await
    }

    pub async fn send_security_alert(
        &self,
        email: &str,
        display_name: &str,
        message: &str,
    ) -> Result<(), MailError> {
        let html = format!(
            "<p>Hello {display_name},</p><p>Security notice for your POM Market account: {message}</p>"
        );
        self.deliver(email, display_name, "POM Market security alert", &html)
            .await
    }

    async fn deliver(
        &self,
        to_email: &str,
        to_name: &str,
        subject: &str,
        html: &str,
    ) -> Result<(), MailError> {
        if !self.enabled {
            return Err(MailError::NotConfigured);
        }
        self.mailer
            .send(
                &self.sender_name,
                &self.sender_email,
                to_email,
                to_name,
                subject,
                html,
            )
            .await
    }

    pub fn log_failure(err: &MailError) {
        tracing::warn!(error = %redact(&err.to_string()), "transactional email failed");
    }
}

fn password_reset_email_html(display_name: &str, reset_url: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Reset your POM Market password</title>
</head>
<body style="margin:0;padding:0;background:#f3efe6;font-family:Segoe UI,sans-serif;color:#1d1a16;">
  <table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="background:#f3efe6;padding:32px 12px;">
    <tr>
      <td align="center">
        <table role="presentation" width="100%" style="max-width:520px;background:#fffdf8;border:1px solid #ddd4c6;border-radius:16px;padding:28px;">
          <tr>
            <td>
              <p style="margin:0 0 8px;font-family:Georgia,serif;font-size:24px;color:#9a4318;">POM Market</p>
              <h1 style="margin:0 0 16px;font-family:Georgia,serif;font-size:28px;font-weight:500;">Reset your password</h1>
              <p style="margin:0 0 14px;line-height:1.5;">Hello {display_name},</p>
              <p style="margin:0 0 14px;line-height:1.5;">We received a request to reset the password for your POM Market account. Use the button below to choose a new password.</p>
              <p style="margin:24px 0;text-align:center;">
                <a href="{reset_url}" style="display:inline-block;background:#9a4318;color:#fff;text-decoration:none;padding:12px 22px;border-radius:999px;font-weight:600;">Reset Password</a>
              </p>
              <p style="margin:0 0 14px;line-height:1.5;font-size:14px;color:#6d645b;">This link expires in <strong>30 minutes</strong> and works only once.</p>
              <p style="margin:0;line-height:1.5;font-size:14px;color:#6d645b;">If you did not request a password reset, you can safely ignore this email. Your password will stay the same.</p>
            </td>
          </tr>
        </table>
      </td>
    </tr>
  </table>
</body>
</html>"#
    )
}

pub fn brevo_api_key_from_env() -> Option<SecretString> {
    if automated_test_context_from_urls_only() {
        return None;
    }
    for name in ["BREVO_API_KEY", "BREEVO_API_KEY"] {
        if let Ok(value) = std::env::var(name) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(SecretString::from(trimmed.to_string()));
            }
        }
    }
    None
}

fn automated_test_context_from_urls_only() -> bool {
    if std::env::var("ENVIRONMENT").as_deref() == Ok("test") {
        return true;
    }
    if std::env::var("E2E_MODE").as_deref() == Ok("1") {
        return true;
    }
    for name in [
        "MIGRATION_DATABASE_URL",
        "APP_DATABASE_URL",
        "WORKER_DATABASE_URL",
    ] {
        if let Ok(url) = std::env::var(name) {
            if url.contains("pommarket_test") || url.contains("pommarket_e2e") {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;
    use std::sync::{Mutex, OnceLock};

    fn env_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn test_cfg(url_suffix: &str) -> Config {
        let base = format!("postgres://u:p@localhost:5432/{url_suffix}");
        Config {
            env: "test".into(),
            api_bind: "0.0.0.0:4321".into(),
            migration_database_url: base.clone(),
            app_database_url: base.clone(),
            worker_database_url: base,
            admin_origin: "http://localhost:4322".into(),
            user_origin: "http://localhost:4323".into(),
            jwt_secret: "x".repeat(32),
            encryption_master_key: "x".repeat(32),
            encryption_key_id: "k1".into(),
            previous_encryption_key: None,
            previous_encryption_key_id: None,
            access_ttl_seconds: 900,
            refresh_ttl_seconds: 1209600,
            invite_ttl_hours: 72,
            password_reset_ttl_minutes: 30,
            bootstrap_admin_email: None,
            bootstrap_admin_password: None,
            bootstrap_admin_name: "Admin".into(),
            clob_host: "https://clob.polymarket.com".into(),
            data_api_host: "https://data-api.polymarket.com".into(),
            gamma_host: "https://gamma-api.polymarket.com".into(),
            rtds_url: "wss://ws-live-data.polymarket.com".into(),
            polygon_rpc_url: None,
            polygon_logs_enabled: false,
            chain_id: 137,
            ctf_exchange: "0x".into(),
            neg_risk_exchange: "0x".into(),
            worker_poll_seconds: 8,
            reconciliation_seconds: 30,
            confirm_live_trading: false,
            fee_rate: rust_decimal::Decimal::ZERO,
            fee_exponent: 1,
            log_format: "text".into(),
            brevo_api_key: Some(SecretString::from("secret-should-not-be-used".to_string())),
            brevo_api_base: None,
            brevo_sender_email: "noreply@example.com".into(),
            brevo_sender_name: "POM Market".into(),
            app_base_url: "http://localhost:4323".into(),
            trusted_proxies: String::new(),
            trusted_proxies_parsed: crate::client_ip::TrustedProxies::default(),
        }
    }

    #[test]
    fn reads_brevo_and_legacy_env_names() {
        let _guard = env_test_lock().lock().unwrap();
        for name in [
            "BREVO_API_KEY",
            "BREEVO_API_KEY",
            "ENVIRONMENT",
            "E2E_MODE",
            "MIGRATION_DATABASE_URL",
            "APP_DATABASE_URL",
            "WORKER_DATABASE_URL",
        ] {
            std::env::remove_var(name);
        }
        std::env::set_var("ENVIRONMENT", "development");
        assert!(brevo_api_key_from_env().is_none());
        std::env::set_var("BREVO_API_KEY", "key-primary");
        assert_eq!(
            brevo_api_key_from_env().unwrap().expose_secret(),
            "key-primary"
        );
        std::env::remove_var("BREVO_API_KEY");
        std::env::set_var("BREEVO_API_KEY", "key-legacy");
        assert_eq!(
            brevo_api_key_from_env().unwrap().expose_secret(),
            "key-legacy"
        );
        std::env::remove_var("BREEVO_API_KEY");
        std::env::remove_var("ENVIRONMENT");
    }

    #[test]
    fn test_database_forces_mock_mode() {
        let _guard = env_test_lock().lock().unwrap();
        let cfg = test_cfg("pommarket_test");
        std::env::remove_var("EMAIL_MODE");
        let mail = Email::from_config(&cfg).unwrap();
        assert!(mail.enabled());
        assert_eq!(mail.mode(), EmailMode::Mock);
    }

    #[test]
    fn brevo_mode_rejected_in_test_context() {
        let _guard = env_test_lock().lock().unwrap();
        let cfg = test_cfg("pommarket_test");
        std::env::set_var("EMAIL_MODE", "brevo");
        assert!(Email::from_config(&cfg).is_err());
        std::env::remove_var("EMAIL_MODE");
    }
}
