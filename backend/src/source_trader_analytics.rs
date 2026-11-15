//! Polymarket Data API analytics cache for source traders (profile page).

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{begin_as, Tx};
use crate::polymarket::http::PublicClient;
use crate::security::redact;

pub const PNL_CHART_LABEL: &str = "Economic PnL (Polymarket Data API)";
pub const MARKETS_TRADED_LABEL: &str =
    "Unique markets (`user-stats.trades`, not fill count)";

pub const REFRESH_TRADE_PAGES: u32 = 8;
pub const BACKFILL_TRADE_PAGES_PER_TICK: u32 = 25;
pub const REFRESH_ACTIVITY_PAGES: u32 = 8;
pub const BACKFILL_ACTIVITY_PAGES_PER_TICK: u32 = 25;
const ANALYTICS_SYNC_BATCH: i64 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestStopReason {
    ApiEnd,
    PageLimit,
    EmptyBatch,
}

#[derive(Clone, Debug)]
pub struct TradeIngestReport {
    pub inserted: u64,
    pub pages_fetched: u32,
    pub final_cursor: Option<String>,
    pub stop: IngestStopReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncFinishInput {
    pub backfill_complete: bool,
    pub has_cursor: bool,
    pub had_error: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncFinishOutput {
    pub status: &'static str,
    pub phase: &'static str,
}

pub fn resolve_sync_finish(input: SyncFinishInput) -> SyncFinishOutput {
    if input.had_error {
        return SyncFinishOutput {
            status: "error",
            phase: "idle",
        };
    }
    if input.backfill_complete {
        SyncFinishOutput {
            status: "ready",
            phase: "complete",
        }
    } else if input.has_cursor {
        SyncFinishOutput {
            status: "stale",
            phase: "backfill_partial",
        }
    } else {
        SyncFinishOutput {
            status: "ready",
            phase: "complete",
        }
    }
}

pub fn backfill_stop_reason(stop: IngestStopReason) -> Option<&'static str> {
    match stop {
        IngestStopReason::ApiEnd => Some("api_end"),
        IngestStopReason::PageLimit => Some("page_limit"),
        IngestStopReason::EmptyBatch => Some("api_end"),
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TradeSizeStats {
    pub count: u64,
    pub sum_usd: Decimal,
    pub min_usd: Option<Decimal>,
    pub max_usd: Option<Decimal>,
    pub mean_usd: Option<Decimal>,
    pub median_usd: Option<Decimal>,
    pub p75_usd: Option<Decimal>,
    pub p90_usd: Option<Decimal>,
    pub p95_usd: Option<Decimal>,
    pub p99_usd: Option<Decimal>,
    pub buckets: Vec<(String, u64)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PeriodMetrics {
    pub economic_pnl_delta: Option<Decimal>,
    pub trade_pnl_delta: Option<Decimal>,
    pub realized_pnl_delta: Option<Decimal>,
    pub unrealized_pnl_end: Option<Decimal>,
    pub trading_volume_usd: Decimal,
    pub total_trades: u64,
    pub buy_count: u64,
    pub sell_count: u64,
    pub buy_volume_usd: Decimal,
    pub sell_volume_usd: Decimal,
    pub buy_stats: TradeSizeStats,
    pub sell_stats: TradeSizeStats,
    pub fees_delta: Option<Decimal>,
    pub wallet_income_delta: Option<Decimal>,
    pub largest_trade_usd: Option<Decimal>,
    pub average_trade_usd: Option<Decimal>,
    pub median_trade_usd: Option<Decimal>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PnlChartPoint {
    pub at: DateTime<Utc>,
    pub economic_pnl_usd: Decimal,
    pub cumulative_delta_usd: Decimal,
}

#[derive(Clone, Debug, Serialize)]
pub struct TraderAnalyticsDashboard {
    pub wallet_address: String,
    pub alias: String,
    pub period: String,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub sync_status: String,
    pub sync_phase: String,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub profile_stats: Value,
    pub markets_traded: Option<i64>,
    pub join_date: Option<DateTime<Utc>>,
    pub all_time_volume_usd: Option<Decimal>,
    pub last_trade_at: Option<DateTime<Utc>>,
    pub metrics: PeriodMetrics,
    pub pnl_chart_label: String,
    pub pnl_points: Vec<PnlChartPoint>,
    pub positions_open: Value,
    pub positions_redeemable: Value,
    pub positions_summary: Value,
    pub extra: Value,
    pub trades_sync: Value,
}

pub fn resolve_trader_period(
    period: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> anyhow::Result<(DateTime<Utc>, DateTime<Utc>, String)> {
    let now = Utc::now();
    if let (Some(f), Some(t)) = (from, to) {
        let start = DateTime::parse_from_rfc3339(f)
            .map_err(|_| anyhow::anyhow!("invalid from timestamp"))?
            .with_timezone(&Utc);
        let end = DateTime::parse_from_rfc3339(t)
            .map_err(|_| anyhow::anyhow!("invalid to timestamp"))?
            .with_timezone(&Utc);
        return Ok((start, end.min(now), "custom".into()));
    }
    let label = period.unwrap_or("30D").to_ascii_uppercase();
    let start = match label.as_str() {
        "1D" => now - Duration::days(1),
        "7D" => now - Duration::days(7),
        "30D" => now - Duration::days(30),
        "90D" => now - Duration::days(90),
        "ALL" => DateTime::<Utc>::from_naive_utc_and_offset(
            NaiveDate::from_ymd_opt(1970, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            Utc,
        ),
        other => anyhow::bail!("unknown period {other}"),
    };
    Ok((start, now, label))
}

pub fn pnl_interval_for_period(period: &str) -> &'static str {
    match period {
        "1D" => "1d",
        "7D" => "1w",
        "30D" => "1m",
        "90D" | "ALL" | "CUSTOM" => "all",
        _ => "1m",
    }
}

pub async fn ensure_sync_row(
    pool: &PgPool,
    user_id: Uuid,
    source_trader_id: Uuid,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    sqlx::query(
        "INSERT INTO source_trader_analytics_sync (source_trader_id, user_id, status)
         VALUES ($1, $2, 'pending')
         ON CONFLICT (source_trader_id) DO NOTHING",
    )
    .bind(source_trader_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn request_refresh(
    pool: &PgPool,
    user_id: Uuid,
    source_trader_id: Uuid,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM source_traders WHERE id = $1 AND user_id = $2)",
    )
    .bind(source_trader_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !exists {
        anyhow::bail!("trader not found");
    }
    ensure_sync_row(pool, user_id, source_trader_id).await?;
    sqlx::query(
        "UPDATE source_trader_analytics_sync
         SET status = 'pending', updated_at = now()
         WHERE source_trader_id = $1 AND user_id = $2
           AND status IS DISTINCT FROM 'syncing'",
    )
    .bind(source_trader_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn run_pending_syncs(pool: &PgPool, client: &PublicClient) -> anyhow::Result<u32> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT source_trader_id FROM source_trader_analytics_sync
         WHERE status IN ('pending', 'stale')
            OR (status = 'ready' AND trades_backfill_complete = false)
            OR (status = 'ready' AND COALESCE(activity_backfill_complete, false) = false)
         ORDER BY updated_at ASC
         LIMIT $1",
    )
    .bind(ANALYTICS_SYNC_BATCH)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut done = 0u32;
    for id in ids {
        if sync_trader(pool, client, id).await.is_ok() {
            done += 1;
        }
    }
    Ok(done)
}

pub async fn sync_trader(
    pool: &PgPool,
    client: &PublicClient,
    source_trader_id: Uuid,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT t.user_id, t.wallet_address FROM source_traders t WHERE t.id = $1",
    )
    .bind(source_trader_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((user_id, wallet)) = row else {
        tx.rollback().await?;
        return Ok(());
    };
    tx.commit().await?;

    let _ = mark_syncing(pool, source_trader_id).await;

    let mut err: Option<String> = None;
    let backfill_done = load_backfill_complete(pool, source_trader_id).await?;

    if let Err(e) = sync_profile_stats(pool, client, source_trader_id, user_id, &wallet).await {
        err = Some(redact(&e.to_string()));
    }
    if let Err(e) = sync_user_volume(pool, client, source_trader_id, user_id, &wallet).await {
        err.get_or_insert(redact(&e.to_string()));
    }
    if let Err(e) = sync_pnl_series(pool, client, source_trader_id, &wallet).await {
        err.get_or_insert(redact(&e.to_string()));
    }
    if let Err(e) = sync_positions(pool, client, source_trader_id, user_id, &wallet).await {
        err.get_or_insert(redact(&e.to_string()));
    }
    if backfill_done {
        if let Err(e) =
            sync_trades_incremental(pool, client, source_trader_id, &wallet, REFRESH_TRADE_PAGES)
                .await
        {
            err.get_or_insert(redact(&e.to_string()));
        }
    } else if let Err(e) = maybe_backfill_trades(pool, client, source_trader_id, &wallet).await {
        err.get_or_insert(redact(&e.to_string()));
    }

    let activity_backfill_done = load_activity_backfill_complete(pool, source_trader_id).await?;
    if activity_backfill_done {
        if let Err(e) =
            sync_activity_incremental(pool, client, source_trader_id, &wallet, REFRESH_ACTIVITY_PAGES)
                .await
        {
            err.get_or_insert(redact(&e.to_string()));
        }
    } else if let Err(e) = maybe_backfill_activity(pool, client, source_trader_id, &wallet).await {
        err.get_or_insert(redact(&e.to_string()));
    }

    if let Some(msg) = err {
        mark_sync_error(pool, source_trader_id, &msg).await?;
    } else {
        finalize_sync_status(pool, source_trader_id).await?;
    }
    Ok(())
}

async fn mark_syncing(pool: &PgPool, id: Uuid) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query(
        "UPDATE source_trader_analytics_sync SET status = 'syncing', sync_phase = 'profile',
            last_attempt_at = now(), updated_at = now() WHERE source_trader_id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn load_backfill_complete(pool: &PgPool, id: Uuid) -> anyhow::Result<bool> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let done: bool = sqlx::query_scalar(
        "SELECT trades_backfill_complete FROM source_trader_analytics_sync WHERE source_trader_id = $1",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(done)
}

async fn finalize_sync_status(pool: &PgPool, id: Uuid) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let row: (bool, Option<String>) = sqlx::query_as(
        "SELECT trades_backfill_complete, trades_cursor FROM source_trader_analytics_sync WHERE source_trader_id = $1",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    let finish = resolve_sync_finish(SyncFinishInput {
        backfill_complete: row.0,
        has_cursor: row.1.as_ref().is_some_and(|c| !c.is_empty()),
        had_error: false,
    });
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query(
        "UPDATE source_trader_analytics_sync SET status = $2, sync_phase = $3,
            last_success_at = now(), last_error = NULL, updated_at = now() WHERE source_trader_id = $1",
    )
    .bind(id)
    .bind(finish.status)
    .bind(finish.phase)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn mark_sync_error(pool: &PgPool, id: Uuid, msg: &str) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query(
        "UPDATE source_trader_analytics_sync SET status = 'error', last_error = $2,
            last_attempt_at = now(), updated_at = now() WHERE source_trader_id = $1",
    )
    .bind(id)
    .bind(msg)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn sync_profile_stats(
    pool: &PgPool,
    client: &PublicClient,
    trader_id: Uuid,
    user_id: Uuid,
    wallet: &str,
) -> anyhow::Result<()> {
    let body = client.user_stats(wallet).await.map_err(anyhow::Error::msg)?;
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query(
        "UPDATE source_trader_analytics_sync SET profile_stats = $2, sync_phase = 'trades_incremental', updated_at = now()
         WHERE source_trader_id = $1 AND user_id = $3",
    )
    .bind(trader_id)
    .bind(body)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn sync_user_volume(
    pool: &PgPool,
    client: &PublicClient,
    trader_id: Uuid,
    user_id: Uuid,
    wallet: &str,
) -> anyhow::Result<()> {
    let body = client.user_volume(wallet).await.map_err(anyhow::Error::msg)?;
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query(
        "UPDATE source_trader_analytics_sync SET volume_stats = $2, updated_at = now()
         WHERE source_trader_id = $1 AND user_id = $3",
    )
    .bind(trader_id)
    .bind(body)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn sync_pnl_series(
    pool: &PgPool,
    client: &PublicClient,
    trader_id: Uuid,
    wallet: &str,
) -> anyhow::Result<()> {
    for interval in ["1d", "1w", "1m", "all"] {
        let body = client
            .user_pnl_series(wallet, interval)
            .await
            .map_err(anyhow::Error::msg)?;
        upsert_pnl_points(pool, trader_id, interval, &body).await?;
    }
    Ok(())
}

async fn upsert_pnl_points(
    pool: &PgPool,
    trader_id: Uuid,
    interval: &str,
    body: &Value,
) -> anyhow::Result<()> {
    let points = body
        .pointer("/data/points")
        .or_else(|| body.get("points"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut tx = begin_as(pool, None, "worker").await?;
    for p in points {
        let ts = p
            .get("timestamp")
            .and_then(|v| v.as_i64())
            .or_else(|| p.get("timestamp").and_then(|v| v.as_u64()).map(|u| u as i64));
        let Some(ts) = ts else {
            continue;
        };
        let point_ts = Utc.timestamp_opt(ts, 0).single().unwrap_or_else(Utc::now);
        let dec = |k: &str| p.get(k).and_then(json_to_decimal);
        sqlx::query(
            "INSERT INTO source_trader_pnl_points (
                source_trader_id, point_ts, pnl_interval, economic_pnl, trade_pnl, realized_pnl,
                unrealized_pnl, position_pnl, settled_pnl, fees, wallet_income, volume_usdc, raw
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
             ON CONFLICT (source_trader_id, pnl_interval, point_ts) DO UPDATE SET
                economic_pnl = EXCLUDED.economic_pnl,
                trade_pnl = EXCLUDED.trade_pnl,
                realized_pnl = EXCLUDED.realized_pnl,
                unrealized_pnl = EXCLUDED.unrealized_pnl,
                position_pnl = EXCLUDED.position_pnl,
                settled_pnl = EXCLUDED.settled_pnl,
                fees = EXCLUDED.fees,
                wallet_income = EXCLUDED.wallet_income,
                volume_usdc = EXCLUDED.volume_usdc,
                raw = EXCLUDED.raw,
                fetched_at = now()",
        )
        .bind(trader_id)
        .bind(point_ts)
        .bind(interval)
        .bind(dec("economic_pnl"))
        .bind(dec("trade_pnl"))
        .bind(dec("realized_pnl"))
        .bind(dec("unrealized_pnl"))
        .bind(dec("position_pnl"))
        .bind(dec("settled_pnl"))
        .bind(dec("fees"))
        .bind(dec("wallet_income"))
        .bind(dec("volume_usdc"))
        .bind(p)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn sync_positions(
    pool: &PgPool,
    client: &PublicClient,
    trader_id: Uuid,
    user_id: Uuid,
    wallet: &str,
) -> anyhow::Result<()> {
    let rows = client.positions_v2(wallet).await.map_err(anyhow::Error::msg)?;
    let (open, redeemable): (Vec<Value>, Vec<Value>) = rows.into_iter().partition(|r| {
        !r.get("redeemable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    });
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query(
        "UPDATE source_trader_analytics_sync SET positions_open = $2, positions_redeemable = $3, updated_at = now()
         WHERE source_trader_id = $1 AND user_id = $4",
    )
    .bind(trader_id)
    .bind(json!(open))
    .bind(json!(redeemable))
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn sync_trades_incremental(
    pool: &PgPool,
    client: &PublicClient,
    trader_id: Uuid,
    wallet: &str,
    max_pages: u32,
) -> anyhow::Result<u64> {
    Ok(
        ingest_trade_pages(pool, client, trader_id, wallet, max_pages, None, false)
            .await?
            .inserted,
    )
}

async fn load_activity_backfill_complete(pool: &PgPool, trader_id: Uuid) -> anyhow::Result<bool> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let complete: bool = sqlx::query_scalar(
        "SELECT COALESCE(activity_backfill_complete, false) FROM source_trader_analytics_sync WHERE source_trader_id = $1",
    )
    .bind(trader_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(complete)
}

async fn sync_activity_incremental(
    pool: &PgPool,
    client: &PublicClient,
    trader_id: Uuid,
    wallet: &str,
    max_pages: u32,
) -> anyhow::Result<u64> {
    Ok(
        ingest_activity_pages(pool, client, trader_id, wallet, max_pages, None, false)
            .await?
            .inserted,
    )
}

async fn maybe_backfill_activity(
    pool: &PgPool,
    client: &PublicClient,
    trader_id: Uuid,
    wallet: &str,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let complete: bool = sqlx::query_scalar(
        "SELECT COALESCE(activity_backfill_complete, false) FROM source_trader_analytics_sync WHERE source_trader_id = $1",
    )
    .bind(trader_id)
    .fetch_one(&mut *tx)
    .await?;
    let cursor: Option<String> = sqlx::query_scalar(
        "SELECT activity_cursor FROM source_trader_analytics_sync WHERE source_trader_id = $1",
    )
    .bind(trader_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    if complete {
        return Ok(());
    }
    let report = ingest_activity_pages(
        pool,
        client,
        trader_id,
        wallet,
        BACKFILL_ACTIVITY_PAGES_PER_TICK,
        cursor,
        true,
    )
    .await?;
    let mut tx = begin_as(pool, None, "worker").await?;
    let stop = backfill_stop_reason(report.stop);
    let api_end =
        report.stop == IngestStopReason::ApiEnd || report.stop == IngestStopReason::EmptyBatch;
    if api_end {
        sqlx::query(
            "UPDATE source_trader_analytics_sync SET
                activity_backfill_complete = true,
                activity_cursor = NULL,
                activity_backfill_stop_reason = $2,
                sync_phase = 'activity_backfill',
                updated_at = now()
             WHERE source_trader_id = $1",
        )
        .bind(trader_id)
        .bind(stop)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query(
            "UPDATE source_trader_analytics_sync SET
                activity_backfill_stop_reason = $2,
                sync_phase = 'activity_backfill',
                updated_at = now()
             WHERE source_trader_id = $1",
        )
        .bind(trader_id)
        .bind(stop)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn ingest_activity_pages(
    pool: &PgPool,
    client: &PublicClient,
    trader_id: Uuid,
    wallet: &str,
    max_pages: u32,
    start_cursor: Option<String>,
    persist_cursor: bool,
) -> anyhow::Result<TradeIngestReport> {
    let mut cursor = start_cursor;
    let mut inserted = 0u64;
    let mut pages_fetched = 0u32;
    let mut stop = IngestStopReason::PageLimit;
    for _ in 0..max_pages {
        let (cashflows, next_cf) = client
            .activity_cashflows_page(wallet, cursor.as_deref())
            .await
            .map_err(anyhow::Error::msg)?;
        pages_fetched += 1;
        if cashflows.is_empty() {
            cursor = next_cf;
            stop = IngestStopReason::EmptyBatch;
            break;
        }
        inserted += upsert_public_cashflows(pool, trader_id, &cashflows).await?;
        cursor = next_cf.clone();
        if cursor.is_none() {
            stop = IngestStopReason::ApiEnd;
            break;
        }
    }
    if persist_cursor {
        let mut tx = begin_as(pool, None, "worker").await?;
        sqlx::query(
            "UPDATE source_trader_analytics_sync SET activity_cursor = $2,
                activity_ingested_count = activity_ingested_count + $3,
                activity_backfill_pages_fetched = activity_backfill_pages_fetched + $4,
                updated_at = now()
             WHERE source_trader_id = $1",
        )
        .bind(trader_id)
        .bind(&cursor)
        .bind(inserted as i64)
        .bind(pages_fetched as i64)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }
    Ok(TradeIngestReport {
        inserted,
        pages_fetched,
        final_cursor: cursor,
        stop,
    })
}

async fn upsert_public_cashflows(
    pool: &PgPool,
    trader_id: Uuid,
    flows: &[crate::polymarket::RawCashflow],
) -> anyhow::Result<u64> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let mut n = 0u64;
    for f in flows {
        let res = sqlx::query(
            "INSERT INTO source_trader_analytics_cashflows (
                source_trader_id, dedup_key, event_type, resolution_class, event_ts, token_id,
                condition_id, size_shares, payout_usd, tx_hash, origin, raw
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
             ON CONFLICT (source_trader_id, dedup_key) DO NOTHING",
        )
        .bind(trader_id)
        .bind(&f.dedup_key)
        .bind(&f.event_type)
        .bind(&f.resolution_class)
        .bind(f.event_ts)
        .bind(&f.token_id)
        .bind(&f.condition_id)
        .bind(f.size_shares)
        .bind(f.payout_usd)
        .bind(&f.tx_hash)
        .bind(&f.origin)
        .bind(&f.raw)
        .execute(&mut *tx)
        .await?;
        n += res.rows_affected();
    }
    tx.commit().await?;
    Ok(n)
}

async fn maybe_backfill_trades(
    pool: &PgPool,
    client: &PublicClient,
    trader_id: Uuid,
    wallet: &str,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let complete: bool = sqlx::query_scalar(
        "SELECT trades_backfill_complete FROM source_trader_analytics_sync WHERE source_trader_id = $1",
    )
    .bind(trader_id)
    .fetch_one(&mut *tx)
    .await?;
    let cursor: Option<String> = sqlx::query_scalar(
        "SELECT trades_cursor FROM source_trader_analytics_sync WHERE source_trader_id = $1",
    )
    .bind(trader_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    if complete {
        return Ok(());
    }
    let report = ingest_trade_pages(
        pool,
        client,
        trader_id,
        wallet,
        BACKFILL_TRADE_PAGES_PER_TICK,
        cursor,
        true,
    )
    .await?;
    let mut tx = begin_as(pool, None, "worker").await?;
    let stop = backfill_stop_reason(report.stop);
    let api_end =
        report.stop == IngestStopReason::ApiEnd || report.stop == IngestStopReason::EmptyBatch;
    if api_end {
        sqlx::query(
            "UPDATE source_trader_analytics_sync SET
                trades_backfill_complete = true,
                trades_cursor = NULL,
                trades_backfill_stop_reason = $2,
                sync_phase = 'trades_backfill',
                updated_at = now()
             WHERE source_trader_id = $1",
        )
        .bind(trader_id)
        .bind(stop)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query(
            "UPDATE source_trader_analytics_sync SET
                trades_backfill_stop_reason = $2,
                sync_phase = 'trades_backfill',
                updated_at = now()
             WHERE source_trader_id = $1",
        )
        .bind(trader_id)
        .bind(stop)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    let _ = report.inserted;
    Ok(())
}

async fn ingest_trade_pages(
    pool: &PgPool,
    client: &PublicClient,
    trader_id: Uuid,
    wallet: &str,
    max_pages: u32,
    start_cursor: Option<String>,
    persist_cursor: bool,
) -> anyhow::Result<TradeIngestReport> {
    let mut cursor = start_cursor;
    let mut inserted = 0u64;
    let mut pages_fetched = 0u32;
    let mut stop = IngestStopReason::PageLimit;
    for _ in 0..max_pages {
        let (trades, next) = client
            .trades_page(wallet, cursor.as_deref())
            .await
            .map_err(anyhow::Error::msg)?;
        pages_fetched += 1;
        if trades.is_empty() {
            cursor = next;
            stop = IngestStopReason::EmptyBatch;
            break;
        }
        inserted += upsert_public_trades(pool, trader_id, &trades).await?;
        cursor = next.clone();
        if cursor.is_none() {
            stop = IngestStopReason::ApiEnd;
            break;
        }
    }
    if persist_cursor {
        let mut tx = begin_as(pool, None, "worker").await?;
        sqlx::query(
            "UPDATE source_trader_analytics_sync SET trades_cursor = $2,
                trades_ingested_count = trades_ingested_count + $3,
                trades_backfill_pages_fetched = trades_backfill_pages_fetched + $4,
                updated_at = now()
             WHERE source_trader_id = $1",
        )
        .bind(trader_id)
        .bind(&cursor)
        .bind(inserted as i64)
        .bind(pages_fetched as i64)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }
    Ok(TradeIngestReport {
        inserted,
        pages_fetched,
        final_cursor: cursor,
        stop,
    })
}

async fn upsert_public_trades(
    pool: &PgPool,
    trader_id: Uuid,
    trades: &[crate::polymarket::RawTrade],
) -> anyhow::Result<u64> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let mut n = 0u64;
    for t in trades {
        let res = sqlx::query(
            "INSERT INTO source_trader_analytics_trades (
                source_trader_id, dedup_key, event_ts, side, notional_usd, price, size_shares,
                token_id, condition_id, market_title, outcome, market_slug, tx_hash, origin, raw
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
             ON CONFLICT (source_trader_id, dedup_key) DO NOTHING",
        )
        .bind(trader_id)
        .bind(&t.dedup_key)
        .bind(t.event_ts)
        .bind(&t.side)
        .bind(t.notional_usd)
        .bind(t.price)
        .bind(t.size_shares)
        .bind(&t.token_id)
        .bind(&t.condition_id)
        .bind(&t.market_title)
        .bind(&t.outcome)
        .bind(&t.market_slug)
        .bind(&t.tx_hash)
        .bind(&t.origin)
        .bind(&t.raw)
        .execute(&mut *tx)
        .await?;
        n += res.rows_affected();
    }
    tx.commit().await?;
    Ok(n)
}

pub async fn build_dashboard(
    pool: &PgPool,
    user_id: Uuid,
    source_trader_id: Uuid,
    period: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> anyhow::Result<TraderAnalyticsDashboard> {
    let (start, end, label) = resolve_trader_period(period, from, to)?;
    ensure_sync_row(pool, user_id, source_trader_id).await?;
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let trader: Option<(String, String, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT wallet_address, alias, last_trade_at FROM source_traders WHERE id = $1 AND user_id = $2",
    )
    .bind(source_trader_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((wallet, alias, last_trade_at)) = trader else {
        anyhow::bail!("trader not found");
    };
    let sync: Option<SyncRow> = sqlx::query_as(
        "SELECT status, sync_phase, last_success_at, last_error, profile_stats, positions_open, positions_redeemable,
                trades_backfill_complete, trades_cursor, trades_backfill_stop_reason, trades_backfill_pages_fetched,
                trades_ingested_count, volume_stats
         FROM source_trader_analytics_sync WHERE source_trader_id = $1 AND user_id = $2",
    )
    .bind(source_trader_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let trade_cache: Option<(i64, Option<DateTime<Utc>>, Option<DateTime<Utc>>, Option<Decimal>)> =
        sqlx::query_as(
            "SELECT COUNT(*)::bigint, MIN(event_ts), MAX(event_ts), SUM(notional_usd)
             FROM source_trader_analytics_trades WHERE source_trader_id = $1",
        )
        .bind(source_trader_id)
        .fetch_optional(&mut *tx)
        .await?;
    let metrics = aggregate_period_metrics(&mut tx, source_trader_id, start, end).await?;
    let pnl_interval = pnl_interval_for_period(&label);
    let pnl_points = load_pnl_chart(&mut tx, source_trader_id, pnl_interval, start, end).await?;
    tx.commit().await?;

    let profile = sync.as_ref().map(|s| s.profile_stats.clone()).unwrap_or(json!({}));
    let markets = profile_stats_markets(&profile);
    let join = profile_join_date(&profile);
    let volume = profile_stats_volume(&profile);
    let (positions_open, positions_redeemable, positions_summary) =
        sync_positions_summary(sync.as_ref());

    let extra = build_extra_metrics(&metrics, sync.as_ref());
    let trades_sync = build_trades_sync(sync.as_ref(), trade_cache);

    Ok(TraderAnalyticsDashboard {
        wallet_address: wallet,
        alias,
        period: label,
        from: start,
        to: end,
        sync_status: sync
            .as_ref()
            .map(|s| s.status.clone())
            .unwrap_or_else(|| "pending".into()),
        sync_phase: sync
            .as_ref()
            .map(|s| s.sync_phase.clone())
            .unwrap_or_else(|| "idle".into()),
        last_success_at: sync.as_ref().and_then(|s| s.last_success_at),
        last_error: sync.as_ref().and_then(|s| s.last_error.clone()),
        profile_stats: profile,
        markets_traded: markets,
        join_date: join,
        all_time_volume_usd: volume,
        last_trade_at,
        metrics,
        pnl_chart_label: PNL_CHART_LABEL.into(),
        pnl_points,
        positions_open,
        positions_redeemable,
        positions_summary,
        extra,
        trades_sync,
    })
}

#[derive(sqlx::FromRow)]
struct SyncRow {
    status: String,
    sync_phase: String,
    last_success_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    profile_stats: Value,
    positions_open: Value,
    positions_redeemable: Value,
    trades_backfill_complete: bool,
    trades_cursor: Option<String>,
    trades_backfill_stop_reason: Option<String>,
    trades_backfill_pages_fetched: i64,
    trades_ingested_count: i64,
    volume_stats: Value,
}

fn build_trades_sync(
    sync: Option<&SyncRow>,
    cache: Option<(i64, Option<DateTime<Utc>>, Option<DateTime<Utc>>, Option<Decimal>)>,
) -> Value {
    let (cached_count, oldest, newest, local_notional_sum) =
        cache.unwrap_or((0, None, None, None));
    let backfill_complete = sync.map(|s| s.trades_backfill_complete).unwrap_or(false);
    let has_cursor = sync
        .as_ref()
        .and_then(|s| s.trades_cursor.as_ref())
        .is_some_and(|c| !c.is_empty());
    let mut map = serde_json::Map::new();
    map.insert("cached_trades_count".into(), json!(cached_count));
    map.insert("backfill_complete".into(), json!(backfill_complete));
    map.insert("has_more_trades".into(), json!(!backfill_complete && has_cursor));
    map.insert(
        "stop_reason".into(),
        json!(sync.and_then(|s| s.trades_backfill_stop_reason.clone())),
    );
    map.insert(
        "pages_fetched".into(),
        json!(sync.map(|s| s.trades_backfill_pages_fetched).unwrap_or(0)),
    );
    map.insert(
        "rows_inserted_total".into(),
        json!(sync.map(|s| s.trades_ingested_count).unwrap_or(0)),
    );
    map.insert(
        "pages_per_tick".into(),
        json!(BACKFILL_TRADE_PAGES_PER_TICK),
    );
    map.insert(
        "incremental_pages_on_refresh".into(),
        json!(REFRESH_TRADE_PAGES),
    );
    map.insert("oldest_cached_trade_at".into(), json!(oldest));
    map.insert("newest_cached_trade_at".into(), json!(newest));
    map.insert(
        "local_cached_notional_sum_usd".into(),
        json!(local_notional_sum),
    );
    map.insert(
        "volume_stats_api".into(),
        json!(sync.map(|s| s.volume_stats.clone()).unwrap_or(json!({}))),
    );
    map.insert(
        "note".into(),
        json!("markets_traded in profile is unique markets, not fill count"),
    );
    Value::Object(map)
}

async fn aggregate_period_metrics(
    tx: &mut Tx<'_>,
    trader_id: Uuid,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> anyhow::Result<PeriodMetrics> {
    let rows: Vec<(String, Decimal)> = sqlx::query_as(
        "SELECT side, notional_usd FROM source_trader_analytics_trades
         WHERE source_trader_id = $1 AND event_ts >= $2 AND event_ts <= $3",
    )
    .bind(trader_id)
    .bind(start)
    .bind(end)
    .fetch_all(&mut **tx)
    .await?;
    let buy: Vec<Decimal> = rows
        .iter()
        .filter(|(s, _)| s == "buy")
        .map(|(_, n)| *n)
        .collect();
    let sell: Vec<Decimal> = rows
        .iter()
        .filter(|(s, _)| s == "sell")
        .map(|(_, n)| *n)
        .collect();
    let all: Vec<Decimal> = rows.iter().map(|(_, n)| *n).collect();
    let buy_stats = trade_size_stats(&buy);
    let sell_stats = trade_size_stats(&sell);
    let buy_vol: Decimal = buy.iter().copied().sum();
    let sell_vol: Decimal = sell.iter().copied().sum();
    let (econ_d, trade_d, real_d, unreal_end, fees_d, income_d) =
        pnl_deltas_in_period(tx, trader_id, "all", start, end).await?;
    Ok(PeriodMetrics {
        economic_pnl_delta: econ_d,
        trade_pnl_delta: trade_d,
        realized_pnl_delta: real_d,
        unrealized_pnl_end: unreal_end,
        trading_volume_usd: buy_vol + sell_vol,
        total_trades: all.len() as u64,
        buy_count: buy.len() as u64,
        sell_count: sell.len() as u64,
        buy_volume_usd: buy_vol,
        sell_volume_usd: sell_vol,
        buy_stats,
        sell_stats,
        fees_delta: fees_d,
        wallet_income_delta: income_d,
        largest_trade_usd: all.iter().copied().max(),
        average_trade_usd: mean_decimal(&all),
        median_trade_usd: median_decimal(&all),
    })
}

async fn pnl_deltas_in_period(
    tx: &mut Tx<'_>,
    trader_id: Uuid,
    interval: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> anyhow::Result<(
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
)> {
    let rows: Vec<PnlRow> = sqlx::query_as(
        "SELECT point_ts, economic_pnl, trade_pnl, realized_pnl, unrealized_pnl, fees, wallet_income
         FROM source_trader_pnl_points
         WHERE source_trader_id = $1 AND pnl_interval = $2 AND point_ts >= $3 AND point_ts <= $4
         ORDER BY point_ts ASC",
    )
    .bind(trader_id)
    .bind(interval)
    .bind(start)
    .bind(end)
    .fetch_all(&mut **tx)
    .await?;
    if rows.is_empty() {
        return Ok((None, None, None, None, None, None));
    }
    let first = &rows[0];
    let last = rows.last().unwrap();
    let delta = |a: Option<Decimal>, b: Option<Decimal>| match (a, b) {
        (Some(x), Some(y)) => Some(y - x),
        _ => None,
    };
    Ok((
        delta(first.economic_pnl, last.economic_pnl),
        delta(first.trade_pnl, last.trade_pnl),
        delta(first.realized_pnl, last.realized_pnl),
        last.unrealized_pnl,
        delta(first.fees, last.fees),
        delta(first.wallet_income, last.wallet_income),
    ))
}

#[derive(sqlx::FromRow)]
struct PnlRow {
    point_ts: DateTime<Utc>,
    economic_pnl: Option<Decimal>,
    trade_pnl: Option<Decimal>,
    realized_pnl: Option<Decimal>,
    unrealized_pnl: Option<Decimal>,
    fees: Option<Decimal>,
    wallet_income: Option<Decimal>,
}

async fn load_pnl_chart(
    tx: &mut Tx<'_>,
    trader_id: Uuid,
    interval: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> anyhow::Result<Vec<PnlChartPoint>> {
    let rows: Vec<PnlRow> = sqlx::query_as(
        "SELECT point_ts, economic_pnl, trade_pnl, realized_pnl, unrealized_pnl, fees, wallet_income
         FROM source_trader_pnl_points
         WHERE source_trader_id = $1 AND pnl_interval = $2 AND point_ts >= $3 AND point_ts <= $4
         ORDER BY point_ts ASC",
    )
    .bind(trader_id)
    .bind(interval)
    .bind(start)
    .bind(end)
    .fetch_all(&mut **tx)
    .await?;
    if rows.is_empty() {
        return Ok(vec![]);
    }
    let base = rows[0].economic_pnl.unwrap_or(Decimal::ZERO);
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let econ = r.economic_pnl?;
            Some(PnlChartPoint {
                at: r.point_ts,
                economic_pnl_usd: econ,
                cumulative_delta_usd: econ - base,
            })
        })
        .collect())
}

#[derive(Debug, Deserialize)]
pub struct TradesListQuery {
    pub side: Option<String>,
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub from: Option<String>,
    pub to: Option<String>,
}

pub async fn list_trades(
    pool: &PgPool,
    user_id: Uuid,
    source_trader_id: Uuid,
    q: &TradesListQuery,
) -> anyhow::Result<(Vec<TradeListRow>, i64)> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM source_traders WHERE id = $1 AND user_id = $2)",
    )
    .bind(source_trader_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !exists {
        anyhow::bail!("trader not found");
    }
    let from_ts = q
        .from
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));
    let to_ts = q
        .to
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));
    let side = q.side.as_deref().map(|s| s.to_ascii_lowercase());
    let pattern = q
        .q
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| format!("%{}%", s.trim()));

    let items: Vec<TradeListRow> = sqlx::query_as(
        "SELECT event_ts, side, market_title, outcome, price, size_shares, notional_usd, tx_hash, market_slug, token_id
         FROM source_trader_analytics_trades
         WHERE source_trader_id = $1
           AND ($2::text IS NULL OR side = $2)
           AND ($3::timestamptz IS NULL OR event_ts >= $3)
           AND ($4::timestamptz IS NULL OR event_ts <= $4)
           AND ($5::text IS NULL OR market_title ILIKE $5 OR outcome ILIKE $5)
         ORDER BY event_ts DESC
         LIMIT $6 OFFSET $7",
    )
    .bind(source_trader_id)
    .bind(&side)
    .bind(from_ts)
    .bind(to_ts)
    .bind(&pattern)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx)
    .await?;

    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM source_trader_analytics_trades
         WHERE source_trader_id = $1
           AND ($2::text IS NULL OR side = $2)
           AND ($3::timestamptz IS NULL OR event_ts >= $3)
           AND ($4::timestamptz IS NULL OR event_ts <= $4)
           AND ($5::text IS NULL OR market_title ILIKE $5 OR outcome ILIKE $5)",
    )
    .bind(source_trader_id)
    .bind(&side)
    .bind(from_ts)
    .bind(to_ts)
    .bind(&pattern)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((items, total))
}

#[derive(sqlx::FromRow, Serialize)]
pub struct TradeListRow {
    pub event_ts: DateTime<Utc>,
    pub side: String,
    pub market_title: Option<String>,
    pub outcome: Option<String>,
    pub price: Decimal,
    pub size_shares: Decimal,
    pub notional_usd: Decimal,
    pub tx_hash: Option<String>,
    pub market_slug: Option<String>,
    pub token_id: Option<String>,
}

pub fn trade_size_stats(values: &[Decimal]) -> TradeSizeStats {
    if values.is_empty() {
        return TradeSizeStats {
            count: 0,
            sum_usd: Decimal::ZERO,
            min_usd: None,
            max_usd: None,
            mean_usd: None,
            median_usd: None,
            p75_usd: None,
            p90_usd: None,
            p95_usd: None,
            p99_usd: None,
            buckets: bucket_counts(values),
        };
    }
    TradeSizeStats {
        count: values.len() as u64,
        sum_usd: values.iter().copied().sum(),
        min_usd: values.iter().copied().min(),
        max_usd: values.iter().copied().max(),
        mean_usd: mean_decimal(values),
        median_usd: median_decimal(values),
        p75_usd: percentile_decimal(values, 0.75),
        p90_usd: percentile_decimal(values, 0.90),
        p95_usd: percentile_decimal(values, 0.95),
        p99_usd: percentile_decimal(values, 0.99),
        buckets: bucket_counts(values),
    }
}

const BUCKET_BOUNDS: &[(f64, f64, &str)] = &[
    (0.0, 10.0, "$0–10"),
    (10.0, 50.0, "$10–50"),
    (50.0, 100.0, "$50–100"),
    (100.0, 250.0, "$100–250"),
    (250.0, 500.0, "$250–500"),
    (500.0, 1000.0, "$500–1k"),
    (1000.0, 2500.0, "$1k–2.5k"),
    (2500.0, 5000.0, "$2.5k–5k"),
    (5000.0, 10000.0, "$5k–10k"),
    (10000.0, 20000.0, "$10k–20k"),
    (20000.0, 50000.0, "$20k–50k"),
    (50000.0, f64::INFINITY, "$50k+"),
];

pub fn bucket_counts(values: &[Decimal]) -> Vec<(String, u64)> {
    let mut counts = vec![0u64; BUCKET_BOUNDS.len()];
    for v in values {
        let f = v.to_f64().unwrap_or(0.0);
        for (i, (lo, hi, _)) in BUCKET_BOUNDS.iter().enumerate() {
            if f >= *lo && f < *hi {
                counts[i] += 1;
                break;
            }
        }
    }
    BUCKET_BOUNDS
        .iter()
        .zip(counts)
        .map(|((_, _, label), c)| (label.to_string(), c))
        .collect()
}

pub fn mean_decimal(values: &[Decimal]) -> Option<Decimal> {
    if values.is_empty() {
        return None;
    }
    Some(values.iter().copied().sum::<Decimal>() / Decimal::from(values.len() as u64))
}

pub fn median_decimal(values: &[Decimal]) -> Option<Decimal> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(v[v.len() / 2])
}

pub fn percentile_decimal(values: &[Decimal], p: f64) -> Option<Decimal> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
    Some(v[idx.min(v.len() - 1)])
}

fn json_to_decimal(v: &Value) -> Option<Decimal> {
    match v {
        Value::Number(n) => n.to_string().parse().ok(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn profile_stats_markets(profile: &Value) -> Option<i64> {
    profile
        .pointer("/data/trades")
        .or_else(|| profile.get("trades"))
        .and_then(|v| v.as_i64().or_else(|| v.as_u64().map(|u| u as i64)))
}

fn profile_join_date(profile: &Value) -> Option<DateTime<Utc>> {
    let ts = profile
        .pointer("/data/join_date")
        .or_else(|| profile.get("join_date"))
        .and_then(|v| v.as_i64().or_else(|| v.as_u64().map(|u| u as i64)))?;
    Utc.timestamp_opt(ts, 0).single()
}

fn profile_stats_volume(profile: &Value) -> Option<Decimal> {
    profile
        .pointer("/data/all_time_pnl/volume_usdc")
        .or_else(|| profile.pointer("/data/volume_usdc"))
        .and_then(json_to_decimal)
}

fn sync_positions_summary(sync: Option<&SyncRow>) -> (Value, Value, Value) {
    let open = sync
        .map(|s| s.positions_open.clone())
        .unwrap_or(json!([]));
    let redeemable = sync
        .map(|s| s.positions_redeemable.clone())
        .unwrap_or(json!([]));
    let mut total_value = Decimal::ZERO;
    let mut total_unreal = Decimal::ZERO;
    let mut open_count = 0u64;
    if let Some(arr) = open.as_array() {
        open_count = arr.len() as u64;
        for row in arr {
            if let Some(v) = row.get("current_value").and_then(json_to_decimal) {
                total_value += v;
            }
            if let Some(v) = row.get("unrealized_pnl").and_then(json_to_decimal) {
                total_unreal += v;
            }
        }
    }
    let redeemable_count = redeemable.as_array().map(|a| a.len()).unwrap_or(0);
    (
        open,
        redeemable,
        json!({
            "open_count": open_count,
            "redeemable_count": redeemable_count,
            "total_current_value_usd": total_value,
            "total_unrealized_pnl_usd": total_unreal,
        }),
    )
}

fn build_extra_metrics(metrics: &PeriodMetrics, sync: Option<&SyncRow>) -> Value {
    let mut top_markets: serde_json::Map<String, Value> = serde_json::Map::new();
    if metrics.total_trades > 0 {
        top_markets.insert(
            "buy_sell_ratio".into(),
            json!(if metrics.sell_count > 0 {
                metrics.buy_count as f64 / metrics.sell_count as f64
            } else {
                metrics.buy_count as f64
            }),
        );
    }
    if let Some(sync) = sync {
        let mut wins = 0u64;
        let mut losses = 0u64;
        for key in ["positions_open", "positions_redeemable"] {
            let positions = if key == "positions_open" {
                &sync.positions_open
            } else {
                &sync.positions_redeemable
            };
            if let Some(arr) = positions.as_array() {
                for row in arr {
                    let pnl = row.get("total_pnl").and_then(json_to_decimal);
                    match pnl {
                        Some(v) if v > Decimal::ZERO => wins += 1,
                        Some(v) if v < Decimal::ZERO => losses += 1,
                        _ => {}
                    }
                }
            }
        }
        top_markets.insert("positions_win_count".into(), json!(wins));
        top_markets.insert("positions_loss_count".into(), json!(losses));
        if wins + losses > 0 {
            top_markets.insert(
                "positions_win_rate".into(),
                json!(wins as f64 / (wins + losses) as f64),
            );
        }
    }
    Value::Object(top_markets)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn median_and_buckets() {
        let v = vec![Decimal::from(5), Decimal::from(15), Decimal::from(100)];
        assert_eq!(median_decimal(&v), Some(Decimal::from(15)));
        let b = bucket_counts(&v);
        assert_eq!(b[0].1, 1);
        assert_eq!(b[1].1, 1);
        assert_eq!(b[3].1, 1);
    }

    #[test]
    fn percentile_order() {
        let v: Vec<Decimal> = (1..=100).map(Decimal::from).collect();
        assert_eq!(percentile_decimal(&v, 0.99), Some(Decimal::from(99)));
    }

    #[test]
    fn period_resolver_custom() {
        let (a, _, l) = resolve_trader_period(None, None, None).unwrap();
        assert_eq!(l, "30D");
        let _ = a;
    }

    #[test]
    fn sync_finish_complete() {
        let o = resolve_sync_finish(SyncFinishInput {
            backfill_complete: true,
            has_cursor: false,
            had_error: false,
        });
        assert_eq!(o.status, "ready");
        assert_eq!(o.phase, "complete");
    }

    #[test]
    fn sync_finish_partial_has_cursor() {
        let o = resolve_sync_finish(SyncFinishInput {
            backfill_complete: false,
            has_cursor: true,
            had_error: false,
        });
        assert_eq!(o.status, "stale");
        assert_eq!(o.phase, "backfill_partial");
    }

    #[test]
    fn backfill_stop_reason_labels() {
        assert_eq!(backfill_stop_reason(IngestStopReason::PageLimit), Some("page_limit"));
        assert_eq!(backfill_stop_reason(IngestStopReason::ApiEnd), Some("api_end"));
    }
}