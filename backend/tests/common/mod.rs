//! Shared harness for HTTP integration tests (test DB only).
#![allow(dead_code)]

use std::env;
use std::net::SocketAddr;

use crate::test_support::{assert_all_test_database_urls, truncate_integration_tables};
use async_trait::async_trait;
use pommarket::api::{new_app_state, router};
use pommarket::config::Config;
use pommarket::db::{connect, migrate};
use pommarket::engine::{BookLevel, OrderBook};
use pommarket::pipeline::{ingest, process_event, Books, PipelineConfig};
use pommarket::polymarket::http::PublicClient;
use pommarket::polymarket::RawTrade;
use pommarket::security::Vault;
use reqwest::{Client, StatusCode};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use tokio::sync::broadcast;

pub struct FixedBook;

#[async_trait]
impl Books for FixedBook {
    async fn book(&self, _token_id: &str) -> Result<OrderBook, String> {
        Ok(OrderBook {
            bids: vec![BookLevel {
                price: Decimal::new(50, 2),
                size: Decimal::from(500),
            }],
            asks: vec![BookLevel {
                price: Decimal::new(50, 2),
                size: Decimal::from(500),
            }],
            tick: Decimal::new(1, 2),
            min_order_size: Decimal::ZERO,
        })
    }

    async fn fee(&self, _token_id: &str) -> Option<pommarket::engine::FeeSchedule> {
        Some(pommarket::engine::FeeSchedule {
            fees_enabled: true,
            rate: Decimal::new(7, 2),
            exponent: 1,
            taker_only: true,
            rebate_rate: Some(Decimal::new(2, 1)),
            source: "test".into(),
        })
    }
}

pub struct TestApi {
    pub http: Client,
    pub base: String,
    pub worker: sqlx::PgPool,
    pub vault: Vault,
    pub books: FixedBook,
    pub pipeline: PipelineConfig,
    pub data: PublicClient,
    pub app_cfg: Config,
}

impl TestApi {
    pub async fn boot() -> Self {
        assert_all_test_database_urls();
        let migration_url = env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
        migrate(&migration_url).await.unwrap();
        let migration = connect(&migration_url).await.unwrap();
        pommarket::ensure_login_roles(
            &migration,
            &env::var("APP_DB_PASSWORD").unwrap(),
            &env::var("WORKER_DB_PASSWORD").unwrap(),
        )
        .await
        .unwrap();
        truncate_integration_tables(&migration, &migration_url).await;
        pommarket::bootstrap_admin(&migration, &Config::from_env().unwrap())
            .await
            .unwrap();

        let cfg = Config::from_env().unwrap();
        let pool = connect(&cfg.app_database_url).await.unwrap();
        let worker = connect(&cfg.worker_database_url).await.unwrap();
        let (activity, _) = broadcast::channel(16);
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

        let vault = Vault::new(
            &cfg.encryption_master_key,
            &cfg.encryption_key_id,
            None,
            None,
        )
        .unwrap();
        let pipeline = PipelineConfig {
            clob_host: cfg.clob_host.clone(),
            confirm_live: false,
            default_fee_rate: Decimal::ZERO,
            default_fee_exponent: 1,
        };
        let data = PublicClient::new(
            cfg.data_api_host.clone(),
            cfg.clob_host.clone(),
            cfg.gamma_host.clone(),
        );

        Self {
            http: Client::builder().build().unwrap(),
            base: format!("http://{addr}"),
            worker,
            vault,
            books: FixedBook,
            pipeline,
            data,
            app_cfg: cfg,
        }
    }

    pub async fn post(&self, path: &str, body: Value) -> Value {
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert!(status.is_success(), "{path} -> {status} {text}");
        serde_json::from_str(&text).unwrap()
    }

    pub async fn send(&self, method: &str, path: &str, token: &str, body: Value) -> Value {
        let response = self
            .http
            .request(method.parse().unwrap(), format!("{}{path}", self.base))
            .header("authorization", format!("Bearer {token}"))
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert!(status.is_success(), "{method} {path} -> {status} {text}");
        if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap()
        }
    }

    pub async fn get_raw(&self, path: &str, token: &str) -> (StatusCode, Value) {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .header("authorization", format!("Bearer {token}"))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        let body = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }))
        };
        (status, body)
    }

    pub async fn get_status(&self, path: &str, token: &str) -> StatusCode {
        self.http
            .get(format!("{}{path}", self.base))
            .header("authorization", format!("Bearer {token}"))
            .send()
            .await
            .unwrap()
            .status()
    }

    pub async fn fund_paper(&self, token: &str, account_id: &str, delta_usd: &str) {
        self.send(
            "POST",
            &format!("/api/v1/accounts/{account_id}/capital/paper-funding"),
            token,
            json!({
                "delta_usd": delta_usd,
                "reason": "integration_test",
                "idempotency_key": format!("test-fund-{account_id}-{delta_usd}")
            }),
        )
        .await;
    }

    pub async fn bootstrap_capital(&self, token: &str, account_id: &str) {
        self.send(
            "POST",
            &format!("/api/v1/accounts/{account_id}/capital/bootstrap"),
            token,
            json!({ "apply": true, }),
        )
        .await;
    }

    pub async fn login_user(&self, email: &str, password: &str) -> String {
        let body = self
            .post(
                "/api/v1/auth/login",
                json!({ "email": email, "password": password }),
            )
            .await;
        body["access_token"].as_str().unwrap().to_string()
    }

    pub async fn invite_user(&self, admin_token: &str, email: &str, name: &str) -> String {
        let created = self
            .send(
                "POST",
                "/api/v1/admin/users",
                admin_token,
                json!({ "email": email, "display_name": name }),
            )
            .await;
        let token = created["setup_url"]
            .as_str()
            .unwrap()
            .split("token=")
            .nth(1)
            .unwrap();
        self.post(
            "/api/v1/auth/password/setup",
            json!({ "token": token, "password": format!("{name}-pass-10") }),
        )
        .await;
        format!("{name}-pass-10")
    }

    pub fn sample_trade(wallet: &str, side: &str, size: &str, tx: &str) -> RawTrade {
        let payload = json!({
            "proxy_wallet": wallet,
            "transaction_hash": tx,
            "token_id": "55",
            "condition_id": "cond",
            "side": side,
            "price": "0.50",
            "size": size,
            "timestamp": chrono::Utc::now().timestamp(),
            "title": "Filter test market",
            "slug": "filter-test"
        });
        pommarket::polymarket::parse_activity_payload(&payload, "test")
            .pop()
            .unwrap()
    }

    pub async fn inject_and_process(&self, trade: &RawTrade) -> pommarket::pipeline::IngestOutcome {
        let outcome = ingest(&self.worker, trade).await.unwrap();
        if outcome.created || !outcome.duplicate_fragment {
            process_event(
                &self.worker,
                &self.vault,
                &self.books,
                &self.pipeline,
                &self.data,
                &self.app_cfg,
                outcome.event_id,
            )
            .await
            .unwrap();
        }
        outcome
    }
}
