use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;

use crate::security::redact;

use super::{MailError, Mailer};

const DEFAULT_API_BASE: &str = "https://api.brevo.com";

#[derive(Clone)]
pub struct BrevoMailer {
    http: reqwest::Client,
    api_base: String,
    api_key: SecretString,
}

#[derive(Serialize)]
struct SendEmailRequest<'a> {
    sender: Sender<'a>,
    to: Vec<Recipient<'a>>,
    subject: &'a str,
    #[serde(rename = "htmlContent")]
    html_content: &'a str,
}

#[derive(Serialize)]
struct Sender<'a> {
    name: &'a str,
    email: &'a str,
}

#[derive(Serialize)]
struct Recipient<'a> {
    email: &'a str,
    name: &'a str,
}

impl BrevoMailer {
    pub fn new(
        api_key: SecretString,
        api_base: Option<String>,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent("pommarket-copy-platform")
            .build()?;
        Ok(Self {
            http,
            api_base: api_base
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_API_BASE.to_string()),
            api_key,
        })
    }

    pub fn arc(api_key: SecretString, api_base: Option<String>) -> anyhow::Result<Arc<dyn Mailer>> {
        Ok(Arc::new(Self::new(
            api_key,
            api_base,
            Duration::from_secs(20),
        )?))
    }

    async fn post_email(&self, body: &SendEmailRequest<'_>) -> Result<(), MailError> {
        let url = format!("{}/v3/smtp/email", self.api_base.trim_end_matches('/'));
        let mut headers = HeaderMap::new();
        headers.insert(
            "api-key",
            self.api_key
                .expose_secret()
                .parse()
                .map_err(|_| MailError::Transport("invalid Brevo api-key header".into()))?,
        );
        let response = self
            .http
            .post(url)
            .headers(headers)
            .json(body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    MailError::Timeout
                } else {
                    MailError::Transport(redact(&e.to_string()))
                }
            })?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let detail = response.text().await.unwrap_or_default();
        Err(MailError::Api {
            status: status.as_u16(),
            detail: redact(&detail),
        })
    }
}

#[async_trait]
impl Mailer for BrevoMailer {
    async fn send(
        &self,
        sender_name: &str,
        sender_email: &str,
        to_email: &str,
        to_name: &str,
        subject: &str,
        html: &str,
    ) -> Result<(), MailError> {
        let request = SendEmailRequest {
            sender: Sender {
                name: sender_name,
                email: sender_email,
            },
            to: vec![Recipient {
                email: to_email,
                name: to_name,
            }],
            subject,
            html_content: html,
        };
        self.post_email(&request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(server: &MockServer) -> BrevoMailer {
        BrevoMailer::new(
            SecretString::from("test-api-key"),
            Some(server.uri()),
            Duration::from_secs(2),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn successful_delivery() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v3/smtp/email"))
            .and(header("api-key", "test-api-key"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(serde_json::json!({"messageId": "abc"})),
            )
            .mount(&server)
            .await;
        let brevo = client_for(&server);
        brevo
            .send(
                "POM",
                "noreply@example.com",
                "user@example.com",
                "User",
                "Hello",
                "<p>Hi</p>",
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn api_failure_is_reported() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v3/smtp/email"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;
        let brevo = client_for(&server);
        let err = brevo
            .send(
                "POM",
                "noreply@example.com",
                "user@example.com",
                "User",
                "Hello",
                "<p>Hi</p>",
            )
            .await
            .unwrap_err();
        match err {
            MailError::Api { status, .. } => assert_eq!(status, 401),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn timeout_is_reported() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v3/smtp/email"))
            .respond_with(ResponseTemplate::new(201).set_delay(Duration::from_secs(3)))
            .mount(&server)
            .await;
        let brevo = client_for(&server);
        let err = brevo
            .send(
                "POM",
                "noreply@example.com",
                "user@example.com",
                "User",
                "Hello",
                "<p>Hi</p>",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, MailError::Timeout));
    }
}
