use std::time::Duration;

use chrono::{Timelike, Utc};
use serde_json::json;
use sqlx::PgPool;
use tokio::time::sleep;

use pommarket::config::Config;
use pommarket::db::{begin_as, connect, migrate};
use pommarket::pipeline::{ingest, process_event, Books, PipelineConfig};
use pommarket::polymarket::http::PublicClient;
use pommarket::polymarket::rtds;
use pommarket::polymarket::{decode_order_filled, decoded_to_trade, order_filled_topic};
use pommarket::security::Vault;

struct HttpBooks(PublicClient);

#[async_trait::async_trait]
impl Books for HttpBooks {
    async fn book(&self, token_id: &str) -> Result<pommarket::engine::OrderBook, String> {
        self.0.order_book(token_id).await
    }
    async fn fee(&self, token_id: &str) -> Option<pommarket::engine::FeeSchedule> {
        self.0.fee_schedule(token_id).await
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = Config::from_env()?;
    pommarket::init_tracing(&cfg.log_format);
    migrate(&cfg.migration_database_url).await?;
    let migration = connect(&cfg.migration_database_url).await?;
    let app_password = std::env::var("APP_DB_PASSWORD")
        .map_err(|_| anyhow::anyhow!("missing required environment variable APP_DB_PASSWORD"))?;
    let worker_password = std::env::var("WORKER_DB_PASSWORD")
        .map_err(|_| anyhow::anyhow!("missing required environment variable WORKER_DB_PASSWORD"))?;
    pommarket::ensure_login_roles(&migration, &app_password, &worker_password).await?;
    drop(migration);
    let pool = connect(&cfg.worker_database_url).await?;
    let client = PublicClient::new(
        cfg.data_api_host.clone(),
        cfg.clob_host.clone(),
        cfg.gamma_host.clone(),
    );
    let books = HttpBooks(client.clone());
    let pipeline = PipelineConfig {
        clob_host: cfg.clob_host.clone(),
        confirm_live: cfg.confirm_live_trading,
        default_fee_rate: cfg.fee_rate,
        default_fee_exponent: cfg.fee_exponent,
    };
    let vault = Vault::new(
        &cfg.encryption_master_key,
        &cfg.encryption_key_id,
        cfg.previous_encryption_key.as_deref(),
        cfg.previous_encryption_key_id.as_deref(),
    )?;
    let rtds_pool = pool.clone();
    let rtds_books = HttpBooks(client.clone());
    let rtds_cfg = PipelineConfig {
        clob_host: pipeline.clob_host.clone(),
        confirm_live: pipeline.confirm_live,
        default_fee_rate: pipeline.default_fee_rate,
        default_fee_exponent: pipeline.default_fee_exponent,
    };
    let rtds_vault = vault.clone();
    let rtds_url = cfg.rtds_url.clone();
    tokio::spawn(async move {
        rtds::run_rtds(rtds_url, |trades| {
            let pool = rtds_pool.clone();
            let books = &rtds_books;
            let cfg = &rtds_cfg;
            let vault = rtds_vault.clone();
            async move {
                for trade in trades {
                    if let Err(err) = handle_trade(&pool, &vault, books, cfg, trade).await {
                        tracing::warn!(error = %pommarket::security::redact(&err.to_string()), "rtds trade failed");
                    }
                }
            }
        })
        .await;
    });
    let mut polygon_cursor = latest_block(&client, cfg.polygon_rpc_url.as_deref())
        .await
        .unwrap_or(0);
    loop {
        heartbeat(&pool).await;
        if let Err(err) = poll_data_api(&pool, &client, &vault, &books, &pipeline).await {
            tracing::warn!(error = %pommarket::security::redact(&err.to_string()), "data api poll failed");
            set_collector(&pool, "data_api", "error").await;
        } else {
            set_collector(&pool, "data_api", "ok").await;
        }
        if cfg.polygon_logs_enabled {
            if let Some(rpc) = &cfg.polygon_rpc_url {
                match poll_polygon(
                    &pool,
                    &client,
                    &vault,
                    &books,
                    &pipeline,
                    rpc,
                    &cfg,
                    &mut polygon_cursor,
                )
                .await
                {
                    Ok(()) => set_collector(&pool, "polygon", "ok").await,
                    Err(err) => {
                        tracing::warn!(error = %pommarket::security::redact(&err.to_string()), "polygon poll failed");
                        set_collector(&pool, "polygon", "error").await;
                    }
                }
            }
        }
        if let Err(err) = pommarket::hard_close::process_pending_operations(
            &pool,
            &books,
            pipeline.default_fee_rate,
            pipeline.default_fee_exponent,
        )
        .await
        {
            tracing::warn!(error = %pommarket::security::redact(&err.to_string()), "hard close tick failed");
        }
        if let Err(err) = pommarket::pipeline::reconcile_live_orders(
            &pool,
            &vault,
            &cfg.clob_host,
            cfg.reconciliation_seconds as i64,
        )
        .await
        {
            tracing::warn!(error = %pommarket::security::redact(&err.to_string()), "reconciliation failed");
        }
        sleep(Duration::from_secs(cfg.worker_poll_seconds)).await;
    }
}

async fn handle_trade(
    pool: &PgPool,
    vault: &Vault,
    books: &impl Books,
    cfg: &PipelineConfig,
    trade: pommarket::polymarket::RawTrade,
) -> anyhow::Result<()> {
    let watched = wallet_watched(pool, &trade.source_wallet).await?;
    if !watched {
        return Ok(());
    }
    let outcome = ingest(pool, &trade).await?;
    if outcome.created || !outcome.duplicate_fragment {
        process_event(pool, vault, books, cfg, outcome.event_id).await?;
    }
    Ok(())
}

async fn wallet_watched(pool: &PgPool, wallet: &str) -> anyhow::Result<bool> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let watched: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM source_traders WHERE lower(wallet_address) = $1 AND monitoring_enabled)",
    )
    .bind(wallet)
    .fetch_one(&mut *tx)
    .await?;
    Ok(watched)
}

async fn poll_data_api(
    pool: &PgPool,
    client: &PublicClient,
    vault: &Vault,
    books: &impl Books,
    cfg: &PipelineConfig,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let wallets: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT lower(wallet_address) FROM source_traders WHERE monitoring_enabled",
    )
    .fetch_all(&mut *tx)
    .await?;
    drop(tx);
    for wallet in wallets {
        let mut cursor = None;
        let mut seen_cursors = std::collections::HashSet::new();
        for _ in 0..5 {
            let (trades, next) = client
                .activity_page(&wallet, cursor.as_deref())
                .await
                .map_err(anyhow::Error::msg)?;
            for trade in trades {
                handle_trade(pool, vault, books, cfg, trade).await?;
            }
            let Some(next) = next else { break };
            if !seen_cursors.insert(next.clone()) {
                break;
            }
            cursor = Some(next);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn poll_polygon(
    pool: &PgPool,
    client: &PublicClient,
    vault: &Vault,
    books: &impl Books,
    pipeline: &PipelineConfig,
    rpc: &str,
    cfg: &Config,
    cursor: &mut u64,
) -> anyhow::Result<()> {
    let head = latest_block(client, Some(rpc)).await?.saturating_sub(2);
    if *cursor == 0 {
        *cursor = head.saturating_sub(20);
    }
    if head <= *cursor {
        return Ok(());
    }
    let from = *cursor + 1;
    let to = head.min(from + 200);
    let topic = format!("0x{}", hex::encode(order_filled_topic()));
    for address in [&cfg.ctf_exchange, &cfg.neg_risk_exchange] {
        let logs = client
            .polygon_logs(rpc, address, &topic, from, to)
            .await
            .map_err(anyhow::Error::msg)?;
        for log in logs {
            if log.get("removed").and_then(|v| v.as_bool()) == Some(true) {
                continue;
            }
            let topics = log
                .get("topics")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let data = log.get("data").and_then(|v| v.as_str()).unwrap_or("");
            let tx = log
                .get("transactionHash")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Some(decoded) = decode_order_filled(&topics, data, tx) {
                let trade = decoded_to_trade(decoded, log);
                handle_trade(pool, vault, books, pipeline, trade).await?;
            }
        }
    }
    *cursor = to;
    Ok(())
}

async fn latest_block(_client: &PublicClient, rpc: Option<&str>) -> anyhow::Result<u64> {
    let Some(rpc) = rpc else { return Ok(0) };
    let response = reqwest::Client::new()
        .post(rpc)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}))
        .send()
        .await?;
    let body: serde_json::Value = response.json().await?;
    let hex_value = body.get("result").and_then(|v| v.as_str()).unwrap_or("0x0");
    Ok(u64::from_str_radix(hex_value.trim_start_matches("0x"), 16).unwrap_or(0))
}

async fn heartbeat(pool: &PgPool) {
    let Ok(mut tx) = begin_as(pool, None, "worker").await else {
        return;
    };
    let _ = sqlx::query(
        "UPDATE system_settings SET value = $1, updated_at = now() WHERE key = 'worker_heartbeat'",
    )
    .bind(json!({"at": Utc::now().to_rfc3339(), "second": Utc::now().second()}))
    .execute(&mut *tx)
    .await;
    let _ = tx.commit().await;
}

async fn set_collector(pool: &PgPool, field: &str, status: &str) {
    let Ok(mut tx) = begin_as(pool, None, "worker").await else {
        return;
    };
    let _ = sqlx::query(
        "UPDATE system_settings SET value = value || jsonb_build_object($1::text, $2::text), updated_at = now() WHERE key = 'collector_status'",
    )
    .bind(field)
    .bind(status)
    .execute(&mut *tx)
    .await;
    let _ = tx.commit().await;
}
