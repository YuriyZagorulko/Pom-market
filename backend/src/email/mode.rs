use std::env;

use crate::config::Config;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmailMode {
    Brevo,
    Console,
    Mock,
}

impl EmailMode {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_lowercase().as_str() {
            "brevo" => Some(Self::Brevo),
            "console" => Some(Self::Console),
            "mock" => Some(Self::Mock),
            "" => None,
            _ => None,
        }
    }
}

pub fn automated_test_context(cfg: &Config) -> bool {
    if env::var("ENVIRONMENT").as_deref() == Ok("test") {
        return true;
    }
    if env::var("E2E_MODE").as_deref() == Ok("1") {
        return true;
    }
    for url in [
        &cfg.migration_database_url,
        &cfg.app_database_url,
        &cfg.worker_database_url,
    ] {
        if url.contains("pommarket_test") || url.contains("pommarket_e2e") {
            return true;
        }
    }
    false
}

pub fn resolve_email_mode(
    cfg: &Config,
    explicit: Option<EmailMode>,
) -> Result<EmailMode, ResolveEmailModeError> {
    if automated_test_context(cfg) {
        if explicit == Some(EmailMode::Brevo) {
            return Err(ResolveEmailModeError::BrevoForbiddenInTests);
        }
        return Ok(EmailMode::Mock);
    }
    if let Some(mode) = explicit {
        return Ok(mode);
    }
    if cfg.brevo_api_key.is_some() {
        return Ok(EmailMode::Brevo);
    }
    Err(ResolveEmailModeError::Unconfigured)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveEmailModeError {
    Unconfigured,
    BrevoForbiddenInTests,
}

impl std::fmt::Display for ResolveEmailModeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unconfigured => write!(f, "email is not configured"),
            Self::BrevoForbiddenInTests => write!(
                f,
                "EMAIL_MODE=brevo is forbidden in automated test environments (ENVIRONMENT=test, E2E_MODE=1, or pommarket_test / pommarket_e2e database URLs)"
            ),
        }
    }
}

impl std::error::Error for ResolveEmailModeError {}

pub fn email_mode_from_env() -> Option<EmailMode> {
    env::var("EMAIL_MODE")
        .ok()
        .and_then(|v| EmailMode::parse(&v))
}
