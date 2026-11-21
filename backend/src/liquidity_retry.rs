//! Bounded BUY liquidity retries when the first CLOB walk finds no fillable depth.

use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;
use crate::db::begin_as;
use crate::engine::{
    paper_clob_diagnostic, simulate_copy_fill, slippage_bps, truncate_lot, RejectReason,
};
use crate::pipeline::{fulfill_approved_decision, Books, PipelineConfig};
use crate::polymarket::http::PublicClient;
use crate::security::Vault;

pub const RETRY_WINDOW_SECS: i64 = 10;
pub const RETRY_INTERVAL_SECS: i64 = 1;

pub fn initial_retry_blob(reason: &RejectReason) -> Value {
    let now = Utc::now();
    json!({
        "started_at": now.to_rfc3339(),
        "next_attempt_at": (now + Duration::seconds(RETRY_INTERVAL_SECS)).to_rfc3339(),
        "attempt": 0,
        "last_reason": reason.as_str(),
        "attempts": [],
    })
}

pub fn merge_retry_checks(checks: &mut Value, retry: Value) {
    if let Some(obj) = checks.as_object_mut() {
        obj.insert("liquidity_retry".into(), retry);
    }
}

#[derive(sqlx::FromRow)]
struct RetryRow {
    decision_id: Uuid,
    strategy_id: Uuid,
    source_event_id: Uuid,
    user_id: Uuid,
    checks: Value,
    target_shares: Option<Decimal>,
    limit_price: Option<Decimal>,
    reason: String,
}

#[derive(sqlx::FromRow)]
struct EventRow {
    id: Uuid,
    source_wallet: String,
    token_id: String,
    condition_id: String,
    side: String,
    price: Decimal,
    size_shares: Decimal,
    notional_usd: Option<Decimal>,
    usdc_size: Option<Decimal>,
    event_ts: DateTime<Utc>,
    received_at: DateTime<Utc>,
}

pub async fn process_liquidity_retries(
    pool: &PgPool,
    vault: &Vault,
    books: &dyn Books,
    pipeline: &PipelineConfig,
    data: &PublicClient,
    app_cfg: &Config,
) -> anyhow::Result<()> {
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM copy_decisions
         WHERE status = 'awaiting_liquidity'
           AND side = 'buy'
           AND (checks->'liquidity_retry'->>'next_attempt_at')::timestamptz <= now()
         ORDER BY decided_at
         LIMIT 40",
    )
    .fetch_all(pool)
    .await?;

    for id in ids {
        if let Err(err) = tick_one(pool, vault, books, pipeline, data, app_cfg, id).await {
            tracing::warn!(
                decision_id = %id,
                error = %crate::security::redact(&err.to_string()),
                "liquidity retry tick failed"
            );
        }
    }
    Ok(())
}

async fn tick_one(
    pool: &PgPool,
    vault: &Vault,
    books: &dyn Books,
    pipeline: &PipelineConfig,
    data: &PublicClient,
    app_cfg: &Config,
    decision_id: Uuid,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("liquidity-retry:{decision_id}"))
        .execute(&mut *tx)
        .await?;
    let row: RetryRow = sqlx::query_as(
        "SELECT id AS decision_id, strategy_id, source_event_id, user_id, checks,
                target_shares, limit_price, reason
         FROM copy_decisions
         WHERE id = $1 AND status = 'awaiting_liquidity'",
    )
    .bind(decision_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("decision not awaiting liquidity"))?;
    let mut checks = row.checks.clone();
    let event = sqlx::query_as::<_, EventRow>(
        "SELECT id, source_wallet, token_id, condition_id, side, price, size_shares, notional_usd, usdc_size, event_ts, received_at
         FROM source_events WHERE id = $1",
    )
    .bind(row.source_event_id)
    .fetch_one(&mut *tx)
    .await?;

    let retry = row
        .checks
        .get("liquidity_retry")
        .cloned()
        .unwrap_or_else(|| initial_retry_blob(&RejectReason::NoLiquidity));
    let started_at: DateTime<Utc> = retry
        .get("started_at")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);
    let attempt = retry.get("attempt").and_then(|v| v.as_u64()).unwrap_or(0) + 1;
    let elapsed = Utc::now().signed_duration_since(started_at).num_seconds();

    if elapsed > RETRY_WINDOW_SECS {
        finalize_rejected(&mut tx, decision_id, RejectReason::RetryTimeout, &retry, attempt)
            .await?;
        tx.commit().await?;
        return Ok(());
    }

    if leader_sold_before(&mut tx, &event).await? {
        finalize_rejected(
            &mut tx,
            decision_id,
            RejectReason::LeaderSoldBeforeExecution,
            &retry,
            attempt,
        )
        .await?;
        tx.commit().await?;
        return Ok(());
    }

    let order_state = active_order_for_decision(&mut tx, decision_id).await?;
    if order_state.blocked {
        schedule_next(&mut tx, decision_id, &mut checks, attempt, json!({
            "attempt": attempt,
            "skipped": "active_order",
            "order_id": order_state.order_id,
            "order_status": order_state.status,
        }))
        .await?;
        tx.commit().await?;
        return Ok(());
    }

    let target = row.target_shares.unwrap_or(Decimal::ZERO);
    let mut remaining = (target - order_state.filled_shares).max(Decimal::ZERO);
    remaining = truncate_lot(remaining);
    if remaining <= Decimal::ZERO {
        finalize_rejected(
            &mut tx,
            decision_id,
            RejectReason::Duplicate,
            &retry,
            attempt,
        )
        .await?;
        tx.commit().await?;
        return Ok(());
    }

    let strategy = load_strategy(&mut tx, row.strategy_id).await?;
    let slip = strategy.max_slippage_bps.min(strategy.account_max_slippage);
    let limit = row.limit_price.unwrap_or_else(|| event.price);
    let book = books.book(&event.token_id).await;
    let fee = books.fee(&event.token_id).await;
    let paper = strategy.operating_mode == "paper_trading";
    let allow_subminimum = paper && strategy.allow_subminimum_paper_trades;
    let market_min = book.as_ref().map(|b| b.min_order_size).unwrap_or(Decimal::ZERO);

    let diag = paper_clob_diagnostic(&book, true, event.price, limit);
    let sim_result = simulate_copy_fill(
        &book,
        paper,
        allow_subminimum,
        pipeline.paper_allow_synthetic_fills,
        slip,
        true,
        event.price,
        limit,
        remaining,
        remaining * limit,
        market_min,
        fee.as_ref(),
    );

    let mut attempt_log = json!({
        "attempt": attempt,
        "elapsed_ms": elapsed * 1000,
        "leader_price": event.price.to_string(),
        "copy_limit_price": limit.to_string(),
        "best_ask": diag.best_clob_price.map(|p| p.to_string()),
        "depth_within_limit_shares": diag.clob_liquidity_shares_within_limit.map(|p| p.to_string()),
        "requested_shares": remaining.to_string(),
        "prior_filled_shares": order_state.filled_shares.to_string(),
    });

    match sim_result {
        Ok(sim) => {
            attempt_log["vwap"] = json!(sim.fill.vwap.to_string());
            attempt_log["fill_shares"] = json!(sim.fill.shares.to_string());
            if slippage_bps(event.price, sim.fill.vwap) > slip {
                attempt_log["outcome"] = json!("slippage_exceeded");
                schedule_next(&mut tx, decision_id, &mut checks, attempt, attempt_log)
                .await?;
                tx.commit().await?;
                return Ok(());
            }
            attempt_log["outcome"] = json!("filled");
            append_attempt(&mut checks, attempt_log);
            sqlx::query(
                "UPDATE copy_decisions SET status = 'approved', reason = 'approved', checks = $2, updated_at = now() WHERE id = $1",
            )
            .bind(decision_id)
            .bind(&checks)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            fulfill_approved_decision(
                pool,
                vault,
                pipeline,
                data,
                app_cfg,
                decision_id,
                sim,
                fee,
            )
            .await?;
        }
        Err(reason) => {
            attempt_log["outcome"] = json!(reason.as_str());
            if reason == RejectReason::Slippage {
                finalize_rejected(&mut tx, decision_id, reason, &retry, attempt).await?;
            } else if elapsed + RETRY_INTERVAL_SECS > RETRY_WINDOW_SECS {
                finalize_rejected(&mut tx, decision_id, RejectReason::RetryTimeout, &retry, attempt)
                    .await?;
            } else {
                schedule_next(&mut tx, decision_id, &mut checks, attempt, attempt_log).await?;
            }
            tx.commit().await?;
        }
    }
    Ok(())
}

struct OrderState {
    blocked: bool,
    order_id: Option<Uuid>,
    status: Option<String>,
    filled_shares: Decimal,
}

async fn active_order_for_decision(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    decision_id: Uuid,
) -> anyhow::Result<OrderState> {
    let row: Option<(Uuid, String, Decimal)> = sqlx::query_as(
        "SELECT o.id, o.status,
                COALESCE((SELECT SUM(f.shares) FROM fills f WHERE f.order_id = o.id), 0)
         FROM orders o
         WHERE o.decision_id = $1
         ORDER BY o.created_at DESC
         LIMIT 1",
    )
    .bind(decision_id)
    .fetch_optional(&mut **tx)
    .await?;

    let Some((id, status, filled)) = row else {
        return Ok(OrderState {
            blocked: false,
            order_id: None,
            status: None,
            filled_shares: Decimal::ZERO,
        });
    };
    let active = matches!(
        status.as_str(),
        "pending" | "submitting" | "submitted" | "partially_filled" | "unknown"
    );
    Ok(OrderState {
        blocked: active,
        order_id: Some(id),
        status: Some(status),
        filled_shares: filled,
    })
}

async fn leader_sold_before(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &EventRow,
) -> anyhow::Result<bool> {
    let sold: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM source_events
            WHERE lower(source_wallet) = lower($1)
              AND token_id = $2
              AND side = 'sell'
              AND event_ts > $3
              AND id <> $4
         )",
    )
    .bind(&event.source_wallet)
    .bind(&event.token_id)
    .bind(event.event_ts)
    .bind(event.id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(sold)
}

async fn schedule_next(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    decision_id: Uuid,
    checks: &mut Value,
    attempt: u64,
    attempt_log: Value,
) -> anyhow::Result<()> {
    append_attempt(checks, attempt_log);
    if let Some(retry) = checks.get_mut("liquidity_retry") {
        if let Some(obj) = retry.as_object_mut() {
            obj.insert("attempt".into(), json!(attempt));
            obj.insert(
                "next_attempt_at".into(),
                json!((Utc::now() + Duration::seconds(RETRY_INTERVAL_SECS)).to_rfc3339()),
            );
        }
    }
    sqlx::query("UPDATE copy_decisions SET checks = $2, updated_at = now() WHERE id = $1")
        .bind(decision_id)
        .bind(checks.clone())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn append_attempt(checks: &mut Value, attempt_log: Value) {
    if let Some(retry) = checks.get_mut("liquidity_retry") {
        if let Some(obj) = retry.as_object_mut() {
            let mut attempts = obj
                .get("attempts")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            attempts.push(attempt_log);
            obj.insert("attempts".into(), json!(attempts));
        }
    }
}

async fn finalize_rejected(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    decision_id: Uuid,
    reason: RejectReason,
    _retry: &Value,
    attempt: u64,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE copy_decisions SET status = 'rejected', reason = $2, updated_at = now() WHERE id = $1",
    )
    .bind(decision_id)
    .bind(reason.as_str())
    .execute(&mut **tx)
    .await?;
    tracing::info!(decision_id = %decision_id, reason = reason.as_str(), attempt, "liquidity retry finished");
    Ok(())
}

#[derive(sqlx::FromRow)]
struct StrategySnap {
    max_slippage_bps: i32,
    account_max_slippage: i32,
    operating_mode: String,
    allow_subminimum_paper_trades: bool,
}

async fn load_strategy(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    strategy_id: Uuid,
) -> anyhow::Result<StrategySnap> {
    sqlx::query_as(
        "SELECT s.max_slippage_bps, r.max_slippage_bps AS account_max_slippage,
                s.operating_mode, s.allow_subminimum_paper_trades
         FROM copy_strategies s
         JOIN account_risk_limits r ON r.trading_account_id = s.trading_account_id
         WHERE s.id = $1",
    )
    .bind(strategy_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(Into::into)
}