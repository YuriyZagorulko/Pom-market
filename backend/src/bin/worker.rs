use std::time::Duration;

use chrono::{Timelike, Utc};
use serde_json::json;
use sqlx::PgPool;
use tokio::time::sleep;

use pommarket::config::Config;
use pommarket::data_api_ingest::{
    decide_pagination_stop, finish_stream_checkpoint, load_checkpoint_store, persist_ingest_diagnostics,
    save_checkpoint_store, stream_checkpoint_mut, IngestPollDiagnostics, IngestStream,
    PaginationStopReason, StreamTickCounters, WalletIngestDiagnostics, DATA_API_PAGE_SIZE,
    DEFAULT_MAX_PAGES_PER_TICK,
};
use pommarket::db::{begin_as, connect, migrate};
use pommarket::pipeline::{ingest, process_event, Books, PipelineConfig};
use pommarket::polymarket::http::PublicClient;
use pommarket::polymarket::rtds;
use pommarket::polymarket::{decode_order_filled, decoded_to_trade, order_filled_topic};
use pommarket::security::Vault;

#[derive(Clone)]
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
    if std::env::var("POM_SKIP_STARTUP_MIGRATION").unwrap_or_default() != "true" {
        migrate(&cfg.migration_database_url).await?;
    }
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
        paper_allow_synthetic_fills: cfg.paper_allow_synthetic_fills,
    };
    let vault = Vault::new(
        &cfg.encryption_master_key,
        &cfg.encryption_key_id,
        cfg.previous_encryption_key.as_deref(),
        cfg.previous_encryption_key_id.as_deref(),
    )?;
    let rtds_pool = pool.clone();
    let rtds_books = HttpBooks(client.clone());
    let rtds_pipeline = pipeline.clone();
    let rtds_vault = vault.clone();
    let rtds_url = cfg.rtds_url.clone();
    let rtds_client = client.clone();
    let rtds_app_cfg = cfg.clone();
    tokio::spawn(async move {
        rtds::run_rtds(rtds_url, move |trades| {
            let pool = rtds_pool.clone();
            let books = rtds_books.clone();
            let vault = rtds_vault.clone();
            let rtds_client = rtds_client.clone();
            let rtds_app_cfg = rtds_app_cfg.clone();
            let pipe = rtds_pipeline.clone();
            async move {
                for trade in trades {
                    if let Err(err) = handle_trade(
                        &pool,
                        &vault,
                        &books,
                        &pipe,
                        &rtds_client,
                        &rtds_app_cfg,
                        trade,
                    )
                    .await
                    {
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
        if let Err(err) = pommarket::source_trader_analytics::run_pending_syncs(&pool, &client).await {
            tracing::warn!(error = %pommarket::security::redact(&err.to_string()), "source trader analytics sync failed");
        }
        match poll_data_api(&pool, &client, &vault, &books, &pipeline, &cfg).await {
            Ok(()) => set_collector(&pool, "data_api", "ok").await,
            Err(err) => {
                tracing::warn!(error = %pommarket::security::redact(&err.to_string()), "data api poll failed");
                set_collector(&pool, "data_api", "error").await;
            }
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
            &client,
            pipeline.default_fee_rate,
            pipeline.default_fee_exponent,
        )
        .await
        {
            tracing::warn!(error = %pommarket::security::redact(&err.to_string()), "hard close tick failed");
        }
        if let Err(err) =
            pommarket::paper_settlement::process_open_positions(&pool, &client, 40).await
        {
            tracing::warn!(error = %pommarket::security::redact(&err.to_string()), "paper settlement tick failed");
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
        if let Err(err) =
            pommarket::strategy_analytics::record_snapshots_for_enabled_strategies(&pool, &cfg)
                .await
        {
            tracing::warn!(error = %pommarket::security::redact(&err.to_string()), "strategy analytics snapshot failed");
        }
        sleep(Duration::from_secs(cfg.worker_poll_seconds)).await;
    }
}

async fn handle_trade(
    pool: &PgPool,
    vault: &Vault,
    books: &impl Books,
    pipeline: &PipelineConfig,
    data: &PublicClient,
    app_cfg: &Config,
    trade: pommarket::polymarket::RawTrade,
) -> anyhow::Result<()> {
    let watched = wallet_watched(pool, &trade.source_wallet).await?;
    if !watched {
        return Ok(());
    }
    let outcome = ingest(pool, &trade).await?;
    if outcome.created || !outcome.duplicate_fragment {
        process_event(
            pool,
            vault,
            books,
            pipeline,
            data,
            app_cfg,
            outcome.event_id,
        )
        .await?;
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
    pipeline: &PipelineConfig,
    app_cfg: &Config,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let wallets: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT lower(wallet_address) FROM source_traders WHERE monitoring_enabled",
    )
    .fetch_all(&mut *tx)
    .await?;
    drop(tx);
    let max_pages = std::env::var("WORKER_INGEST_MAX_PAGES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_PAGES_PER_TICK);
    let mut checkpoints = load_checkpoint_store(pool).await.unwrap_or_default();
    let mut diagnostics = IngestPollDiagnostics::default();
    let mut first_err: Option<anyhow::Error> = None;
    for wallet in wallets {
        match ingest_wallet_pages(
            pool,
            vault,
            books,
            pipeline,
            client,
            app_cfg,
            &wallet,
            max_pages,
            &mut checkpoints,
        )
        .await
        {
            Ok(wallet_diag) => diagnostics.merge_wallet(&wallet, wallet_diag),
            Err(err) => {
                let redacted = pommarket::security::redact(&err.to_string());
                let mut wallet_diag = WalletIngestDiagnostics::default();
                wallet_diag.error = Some(redacted.clone());
                diagnostics.merge_wallet(&wallet, wallet_diag);
                if first_err.is_none() {
                    first_err = Some(err);
                }
            }
        }
    }
    if let Err(err) = save_checkpoint_store(pool, &checkpoints).await {
        tracing::warn!(error = %pommarket::security::redact(&err.to_string()), "checkpoint save failed");
    }
    if first_err.is_some() {
        diagnostics.mark_error(&pommarket::security::redact(
            &first_err.as_ref().unwrap().to_string(),
        ));
    } else {
        diagnostics.mark_success();
    }
    if let Err(err) = persist_ingest_diagnostics(pool, &diagnostics).await {
        tracing::warn!(error = %pommarket::security::redact(&err.to_string()), "ingest diagnostics persist failed");
    }
    match first_err {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
async fn ingest_wallet_pages(
    pool: &PgPool,
    vault: &Vault,
    books: &impl Books,
    pipeline: &PipelineConfig,
    client: &PublicClient,
    app_cfg: &Config,
    wallet: &str,
    max_pages: usize,
    checkpoints: &mut pommarket::data_api_ingest::IngestCheckpointStore,
) -> anyhow::Result<WalletIngestDiagnostics> {
    let mut wallet_diag = WalletIngestDiagnostics::default();
    let mut first_err: Option<anyhow::Error> = None;
    for stream in [IngestStream::Activity, IngestStream::Trades] {
        match ingest_wallet_stream(
            pool,
            vault,
            books,
            pipeline,
            client,
            app_cfg,
            wallet,
            stream,
            max_pages,
            checkpoints,
        )
        .await
        {
            Ok(stream_diag) => match stream {
                IngestStream::Activity => wallet_diag.activity = stream_diag,
                IngestStream::Trades => wallet_diag.trades = stream_diag,
            },
            Err(err) => {
                let msg = pommarket::security::redact(&err.to_string());
                wallet_diag.error = Some(msg);
                if first_err.is_none() {
                    first_err = Some(err);
                }
            }
        }
    }
    match first_err {
        Some(err) => Err(err),
        None => Ok(wallet_diag),
    }
}

#[allow(clippy::too_many_arguments)]
async fn ingest_wallet_stream(
    pool: &PgPool,
    vault: &Vault,
    books: &impl Books,
    pipeline: &PipelineConfig,
    client: &PublicClient,
    app_cfg: &Config,
    wallet: &str,
    stream: IngestStream,
    max_pages: usize,
    checkpoints: &mut pommarket::data_api_ingest::IngestCheckpointStore,
) -> anyhow::Result<StreamTickCounters> {
    let cp = stream_checkpoint_mut(checkpoints, wallet, stream);
    let mut counters = StreamTickCounters {
        resume_cursor_before: cp.resume_cursor.clone(),
        ..Default::default()
    };
    let mut cursor = cp.resume_cursor.clone();
    let mut seen_cursors = std::collections::HashSet::new();
    let mut stop_reason = PaginationStopReason::EndOfFeed;
    let mut resume_after: Option<String> = None;

    for page_index in 0..max_pages {
        let (trades, next) = match stream {
            IngestStream::Activity => client.activity_page(wallet, cursor.as_deref()).await,
            IngestStream::Trades => client.trades_page(wallet, cursor.as_deref()).await,
        }
        .map_err(anyhow::Error::msg)?;

        counters.pages_fetched += 1;
        let page_len = trades.len();
        counters.rows_fetched += page_len as u64;

        let mut all_rows_were_duplicates = page_len > 0;
        for trade in trades {
            let watched = wallet_watched(pool, &trade.source_wallet).await?;
            if !watched {
                all_rows_were_duplicates = false;
                continue;
            }
            let outcome = ingest(pool, &trade).await?;
            if outcome.duplicate_fragment {
                counters.duplicate_fragments += 1;
            } else {
                all_rows_were_duplicates = false;
            }
            if trade.side == "sell" {
                counters.persisted_sell += 1;
            } else {
                counters.persisted_buy += 1;
            }
            if outcome.created || !outcome.duplicate_fragment {
                process_event(
                    pool,
                    vault,
                    books,
                    pipeline,
                    client,
                    app_cfg,
                    outcome.event_id,
                )
                .await?;
            }
        }

        let has_next = next.as_ref().is_some_and(|s| !s.is_empty());
        if let Some(stop) = decide_pagination_stop(
            page_len,
            DATA_API_PAGE_SIZE,
            page_index,
            max_pages,
            all_rows_were_duplicates,
            has_next,
        ) {
            stop_reason = stop;
            resume_after = next;
            break;
        }

        let Some(next_cursor) = next else {
            stop_reason = PaginationStopReason::EndOfFeed;
            break;
        };
        if !seen_cursors.insert(next_cursor.clone()) {
            stop_reason = PaginationStopReason::RepeatedCursor;
            break;
        }
        cursor = Some(next_cursor);
    }

    let cp = stream_checkpoint_mut(checkpoints, wallet, stream);
    finish_stream_checkpoint(cp, stop_reason, resume_after);
    counters.stop_reason = Some(format!("{stop_reason:?}"));
    counters.resume_cursor_after = cp.resume_cursor.clone();
    Ok(counters)
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
                handle_trade(pool, vault, books, pipeline, client, cfg, trade).await?;
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
