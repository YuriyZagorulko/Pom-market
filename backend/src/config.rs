use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub env: String,
    pub api_bind: String,
    pub migration_database_url: String,
    pub app_database_url: String,
    pub worker_database_url: String,
    pub admin_origin: String,
    pub user_origin: String,
    pub jwt_secret: String,
    pub encryption_master_key: String,
    pub encryption_key_id: String,
    pub previous_encryption_key: Option<String>,
    pub previous_encryption_key_id: Option<String>,
    pub access_ttl_seconds: i64,
    pub refresh_ttl_seconds: i64,
    pub invite_ttl_hours: i64,
    pub password_reset_ttl_minutes: i64,
    pub bootstrap_admin_email: Option<String>,
    pub bootstrap_admin_password: Option<String>,
    pub bootstrap_admin_name: String,
    pub clob_host: String,
    pub data_api_host: String,
    pub gamma_host: String,
    pub rtds_url: String,
    pub polygon_rpc_url: Option<String>,
    pub polygon_logs_enabled: bool,
    pub chain_id: u64,
    pub ctf_exchange: String,
    pub neg_risk_exchange: String,
    pub worker_poll_seconds: u64,
    pub reconciliation_seconds: u64,
    pub confirm_live_trading: bool,
    pub fee_rate: rust_decimal::Decimal,
    pub fee_exponent: u32,
    pub log_format: String,
    pub brevo_api_key: Option<secrecy::SecretString>,
    pub brevo_api_base: Option<String>,
    pub brevo_sender_email: String,
    pub brevo_sender_name: String,
    pub app_base_url: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            env: var("ENVIRONMENT", "development"),
            api_bind: var("API_BIND", "0.0.0.0:4321"),
            migration_database_url: required("MIGRATION_DATABASE_URL")?,
            app_database_url: required("APP_DATABASE_URL")?,
            worker_database_url: required("WORKER_DATABASE_URL")?,
            admin_origin: var("ADMIN_ORIGIN", "http://localhost:4322"),
            user_origin: var("USER_ORIGIN", "http://localhost:4323"),
            jwt_secret: required("JWT_SECRET")?,
            encryption_master_key: required("ENCRYPTION_MASTER_KEY")?,
            encryption_key_id: var("ENCRYPTION_KEY_ID", "k1"),
            previous_encryption_key: empty_as_none("ENCRYPTION_MASTER_KEY_PREVIOUS"),
            previous_encryption_key_id: empty_as_none("ENCRYPTION_KEY_ID_PREVIOUS"),
            access_ttl_seconds: var("ACCESS_TTL_SECONDS", "900").parse()?,
            refresh_ttl_seconds: var("REFRESH_TTL_SECONDS", "1209600").parse()?,
            invite_ttl_hours: var("INVITE_TTL_HOURS", "72").parse()?,
            password_reset_ttl_minutes: var("PASSWORD_RESET_TTL_MINUTES", "30").parse()?,
            bootstrap_admin_email: empty_as_none("BOOTSTRAP_ADMIN_EMAIL"),
            bootstrap_admin_password: empty_as_none("BOOTSTRAP_ADMIN_PASSWORD"),
            bootstrap_admin_name: var("BOOTSTRAP_ADMIN_NAME", "Administrator"),
            clob_host: var("POLYMARKET_CLOB_HOST", "https://clob.polymarket.com"),
            data_api_host: var("POLYMARKET_DATA_API_HOST", "https://data-api.polymarket.com"),
            gamma_host: var("POLYMARKET_GAMMA_HOST", "https://gamma-api.polymarket.com"),
            rtds_url: var("POLYMARKET_RTDS_URL", "wss://ws-live-data.polymarket.com"),
            polygon_rpc_url: empty_as_none("POLYGON_RPC_URL"),
            polygon_logs_enabled: var("POLYGON_LOGS_ENABLED", "false") == "true",
            chain_id: var("POLYGON_CHAIN_ID", "137").parse()?,
            ctf_exchange: var(
                "POLYGON_CTF_EXCHANGE_ADDRESS",
                "0xE111180000d2663C0091e4f400237545B87B996B",
            ),
            neg_risk_exchange: var(
                "POLYGON_NEG_RISK_EXCHANGE_ADDRESS",
                "0xe2222d279d744050d28e00520010520000310F59",
            ),
            worker_poll_seconds: var("WORKER_POLL_SECONDS", "8").parse()?,
            reconciliation_seconds: var("RECONCILIATION_SECONDS", "30").parse()?,
            confirm_live_trading: var("CONFIRM_LIVE_TRADING", "false") == "true",
            fee_rate: var("TAKER_FEE_RATE", "0").parse()?,
            fee_exponent: var("TAKER_FEE_EXPONENT", "1").parse()?,
            log_format: var("LOG_FORMAT", "json"),
            brevo_api_key: crate::email::brevo_api_key_from_env(),
            brevo_api_base: empty_as_none("BREVO_API_BASE"),
            brevo_sender_email: var("BREVO_SENDER_EMAIL", "noreply@example.com"),
            brevo_sender_name: var("BREVO_SENDER_NAME", "POM Market"),
            app_base_url: env::var("APP_BASE_URL")
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| var("USER_ORIGIN", "http://localhost:4323")),
        })
    }

    pub fn cors_origins(&self) -> Vec<String> {
        let extra = env::var("CORS_ORIGINS").unwrap_or_default();
        let mut origins = vec![self.admin_origin.clone(), self.user_origin.clone()];
        for part in extra.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if !origins.iter().any(|o| o == part) {
                origins.push(part.to_string());
            }
        }
        origins
    }
}

fn var(name: &str, default: &str) -> String {
    env::var(name).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_string())
}

fn required(name: &str) -> anyhow::Result<String> {
    env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing required environment variable {name}"))
}

fn empty_as_none(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.trim().is_empty())
}
