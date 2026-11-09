pub mod brevo;

use std::sync::Arc;

use async_trait::async_trait;
use secrecy::SecretString;
use thiserror::Error;

use crate::config::Config;
use crate::security::redact;

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
}

impl Email {
    pub fn from_config(cfg: &Config) -> anyhow::Result<Self> {
        let (mailer, enabled) = match cfg.brevo_api_key.clone() {
            Some(key) => (
                brevo::BrevoMailer::arc(key, cfg.brevo_api_base.clone())?,
                true,
            ),
            None => (Arc::new(NoopMailer) as Arc<dyn Mailer>, false),
        };
        Ok(Self {
            mailer,
            sender_email: cfg.brevo_sender_email.clone(),
            sender_name: cfg.brevo_sender_name.clone(),
            app_base_url: cfg.app_base_url.clone(),
            enabled,
        })
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn setup_url(&self, token: &str) -> String {
        format!("{}/setup?token={token}", self.app_base_url.trim_end_matches('/'))
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

    pub async fn send_invitation(&self, email: &str, display_name: &str, token: &str) -> Result<(), MailError> {
        let url = self.setup_url(token);
        let html = format!(
            "<p>Hello {display_name},</p><p>You have been invited to POM Market. \
             <a href=\"{url}\">Accept your invitation</a> and set your password. \
             This link expires in a few days and works only once.</p>"
        );
        self.deliver(email, display_name, "Your POM Market invitation", &html)
            .await
    }

    pub async fn send_password_recovery(&self, email: &str, display_name: &str, token: &str) -> Result<(), MailError> {
        let url = self.reset_password_url(token);
        let html = password_reset_email_html(display_name, &url);
        self.deliver(email, display_name, "Reset your POM Market password", &html)
            .await
    }

    pub async fn send_email_verification(&self, email: &str, display_name: &str, token: &str) -> Result<(), MailError> {
        let url = self.verify_url(token);
        let html = format!(
            "<p>Hello {display_name},</p><p>Please <a href=\"{url}\">verify your email address</a> for POM Market.</p>"
        );
        self.deliver(email, display_name, "Verify your POM Market email", &html)
            .await
    }

    pub async fn send_password_changed(&self, email: &str, display_name: &str) -> Result<(), MailError> {
        let html = format!(
            "<p>Hello {display_name},</p><p>Your POM Market password was changed. \
             If this was not you, contact an administrator immediately.</p>"
        );
        self.deliver(email, display_name, "Your POM Market password was changed", &html)
            .await
    }

    pub async fn send_security_alert(&self, email: &str, display_name: &str, message: &str) -> Result<(), MailError> {
        let html = format!(
            "<p>Hello {display_name},</p><p>Security notice for your POM Market account: {message}</p>"
        );
        self.deliver(email, display_name, "POM Market security alert", &html)
            .await
    }

    async fn deliver(&self, to_email: &str, to_name: &str, subject: &str, html: &str) -> Result<(), MailError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn reads_brevo_and_legacy_env_names() {
        std::env::set_var("BREEVO_API_KEY", "");
        std::env::remove_var("BREVO_API_KEY");
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
    }
}
