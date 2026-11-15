//! Copy-strategy analytics: strategy-scoped P&L, positions, activity, snapshots.
//!
//! # Financial model (paper copy strategies)
//!
//! - **Allocated capital**: `effective_allocation_cap_usd` from account equity × allocation mode.
//! - **Deployed / invested**: `SUM(cost_basis_usd)` for open lots with `strategy_id`.
//! - **Reserved**: open order reserves for this strategy only.
//! - **Available strategy capital**: `allocated − deployed − reserved` (not shared account cash).
//! - **Realized P&L**: `SUM(positions.realized_pnl_usd)` for the strategy (includes partial exits on open lots).
//! - **Unrealized P&L**: `Σ (mark_price − avg_price) × open_shares` per lot; mark = last strategy fill price, else `avg_price` (labeled estimated).
//! - **Strategy equity metric**: `positions_market_value + realized_pnl` — **does not** include unattributable shared `paper_cash_usd`.
//!
//! Shared account cash is never split across strategies in this release.

#![allow(clippy::type_complexity)]

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;
use crate::db::begin_as;
use crate::strategy_budget::utilization_report_for_user;

pub const SNAPSHOT_INTERVAL_MINUTES: i64 = 15;
/// Marks older than this are labeled stale (not current market).
pub const STALE_MARK_SECONDS: i64 = 86_400;
pub const PARTIAL_EQUITY_LABEL: &str = "Partial equity (position value + realized P&L)";
pub const FULL_EQUITY_LABEL: &str =
    "Strategy equity (available + reserved cash + position market value)";
pub const EQUITY_MODEL_NOTE: &str =
    "Legacy partial view before strategy capital is funded on the ledger.";
pub const PENDING_CAPITAL_NOTE: &str = "Configured allocation is not funded in the strategy ledger yet. Paper copy trades stay blocked until cash moves from the trading account (enable the strategy or allocate capital).";

/// Realized P&L in metrics uses cumulative `positions.realized_pnl_usd` (per strategy).
/// Per-close cycle history: `position_close_events` or sum of `closed_positions` (cycle amounts).
pub const REALIZED_PNL_SEMANTICS_NOTE: &str =
    "Realized P&L sums cumulative amounts on open position rows; closed-cycle history is in position_close_events.";
pub const PENDING_CAPITAL_LABEL: &str = "Configured allocation (not yet funded)";
pub const FULL_EQUITY_NOTE: &str =
    "Full strategy equity from the capital ledger: liquid cash (available + reserved) plus mark-to-model open positions.";

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapitalFundingStatus {
    NotFunded,
    PartiallyFunded,
    Funded,
}

/// Ledger `funded_capital_usd` vs configured allocation cap — never treat configured cap as funded.
pub fn capital_funding_status(configured_cap_usd: Decimal, ledger_funded_usd: Decimal) -> CapitalFundingStatus {
    if ledger_funded_usd <= Decimal::ZERO {
        CapitalFundingStatus::NotFunded
    } else if configured_cap_usd > Decimal::ZERO && ledger_funded_usd < configured_cap_usd {
        CapitalFundingStatus::PartiallyFunded
    } else {
        CapitalFundingStatus::Funded
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EquityModel {
    pub strategy_equity_usd: Decimal,
    pub equity_quality: &'static str,
    pub equity_label: &'static str,
    pub note: &'static str,
    pub cash_attribution_usd: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StrategyMetrics {
    pub allocated_capital_usd: Decimal,
    pub deployed_capital_usd: Decimal,
    pub reserved_capital_usd: Decimal,
    pub available_capital_usd: Decimal,
    pub invested_capital_usd: Decimal,
    pub positions_market_value_usd: Decimal,
    pub realized_pnl_usd: Decimal,
    pub unrealized_pnl_usd: Decimal,
    pub total_pnl_usd: Decimal,
    pub fees_usd: Decimal,
    pub roi_percent: Option<Decimal>,
    pub open_positions_count: i64,
    pub closed_positions_count: i64,
    pub total_trades_count: i64,
    pub equity: EquityModel,
    pub capital_bootstrap_quality: Option<String>,
    pub configured_allocation_cap_usd: Decimal,
    pub funded_capital_usd: Decimal,
    pub capital_funding_status: CapitalFundingStatus,
    pub mark_quality: &'static str,
    pub mark_quality_note: Option<String>,
    /// Primary ROI/PnL view uses CLOB fills only; legacy position lots may pre-date labeling.
    pub performance_fill_scope: &'static str,
    pub clob_fill_count: i64,
    pub legacy_unlabeled_fill_count: i64,
    pub synthetic_experimental_fill_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StrategyOverview {
    pub strategy_id: Uuid,
    pub alias: String,
    pub enabled: bool,
    pub operating_mode: String,
    pub sizing_mode: String,
    pub metrics: StrategyMetrics,
    pub recent_activity: Vec<ActivityRow>,
    pub top_positions: Vec<PositionRow>,
    pub bottom_positions: Vec<PositionRow>,
    pub as_of: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PositionRow {
    pub id: Uuid,
    pub token_id: String,
    pub market_title: Option<String>,
    pub outcome: Option<String>,
    pub status: &'static str,
    pub opened_at: Option<DateTime<Utc>>,
    pub open_shares: Decimal,
    pub avg_price: Decimal,
    pub cost_basis_usd: Decimal,
    pub mark_price: Decimal,
    pub mark_source: &'static str,
    pub market_value_usd: Decimal,
    pub realized_pnl_usd: Decimal,
    pub unrealized_pnl_usd: Decimal,
    pub return_percent: Option<Decimal>,
    pub mark_observed_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub opened_at_fill: Option<DateTime<Utc>>,
    pub avg_exit_price: Option<Decimal>,
    pub proceeds_usd: Option<Decimal>,
    pub position_fees_usd: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityFillLeg {
    pub fill_id: Uuid,
    pub filled_at: DateTime<Utc>,
    pub price: Decimal,
    pub shares: Decimal,
    pub notional_usd: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_usd: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActivityRow {
    pub decision_id: Uuid,
    pub decision_status: String,
    pub side: String,
    pub reason: Option<String>,
    pub reason_title: Option<String>,
    pub reason_detail: Option<String>,
    pub source_event_id: Uuid,
    pub source_event_at: Option<DateTime<Utc>>,
    pub detected_at: Option<DateTime<Utc>>,
    pub decided_at: DateTime<Utc>,
    pub source_notional_usd: Option<Decimal>,
    pub source_price: Option<Decimal>,
    pub source_shares: Option<Decimal>,
    pub market_title: Option<String>,
    pub outcome: Option<String>,
    pub target_notional_usd: Option<Decimal>,
    /// PVR trade multiplier when sizing audit recorded it (`NULL` / 1 = omit in UI).
    pub pvr_trade_multiplier: Option<Decimal>,
    pub sizing_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_target_notional_usd: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capped_target_notional_usd: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sizing_explanation: Option<String>,
    pub order_id: Option<Uuid>,
    pub order_status: Option<String>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub fill_id: Option<Uuid>,
    pub filled_at: Option<DateTime<Utc>>,
    pub fill_price: Option<Decimal>,
    pub fill_shares: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_notional_usd: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_legs: Option<Vec<ActivityFillLeg>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_target_shares: Option<Decimal>,
    pub fee_usd: Option<Decimal>,
    pub slippage_vs_source_bps: Option<i32>,
    pub slippage_vs_limit_bps: Option<i32>,
    pub paper_execution_mode: Option<String>,
    pub paper_execution_context: Option<serde_json::Value>,
    pub execution_status: String,
    /// `decision` (copy pipeline) or `settlement` (capital ledger).
    #[serde(default = "default_activity_kind")]
    pub activity_kind: String,
    /// High-level row type for filters: `buy`, `sell`, `settlement` (and `fill` when filled).
    #[serde(default = "default_feed_type")]
    pub feed_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger_entry_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settlement_payout_usd: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_open_shares: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_closed_shares: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_avg_entry_price: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_avg_exit_price: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_cost_basis_usd: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_proceeds_usd: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_fees_usd: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_realized_pnl_usd: Option<Decimal>,
}

fn default_activity_kind() -> String {
    "decision".into()
}

fn default_feed_type() -> String {
    "buy".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct PerformanceMetrics {
    pub total_return_usd: Decimal,
    pub roi_percent: Option<Decimal>,
    pub realized_pnl_usd: Decimal,
    pub unrealized_pnl_usd: Decimal,
    pub win_rate_percent: Option<Decimal>,
    pub winning_positions: i64,
    pub losing_positions: i64,
    pub avg_win_usd: Option<Decimal>,
    pub avg_loss_usd: Option<Decimal>,
    pub profit_factor: Option<Decimal>,
    pub max_drawdown_percent: Option<Decimal>,
    pub avg_holding_hours: Option<Decimal>,
    pub fees_usd: Decimal,
    pub slippage_bps_avg: Option<Decimal>,
    pub slippage_bps_avg_clob: Option<Decimal>,
    pub slippage_bps_avg_synthetic: Option<Decimal>,
    pub clob_fill_count: i64,
    pub synthetic_fill_count: i64,
    pub legacy_unlabeled_fill_count: i64,
    pub performance_fill_scope: &'static str,
    pub sample_size_note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EquityPoint {
    pub at: DateTime<Utc>,
    pub equity_metric_usd: Decimal,
    pub realized_pnl_usd: Decimal,
    pub unrealized_pnl_usd: Decimal,
    pub deployed_capital_usd: Decimal,
    pub reserved_capital_usd: Decimal,
    pub available_capital_usd: Decimal,
    pub allocated_capital_usd: Decimal,
    pub total_pnl_usd: Decimal,
    pub data_quality: String,
}

pub fn position_row_total_pnl(row: &PositionRow) -> Decimal {
    (row.realized_pnl_usd + row.unrealized_pnl_usd).round_dp(8)
}

struct TokenMark {
    price: Decimal,
    source: &'static str,
    observed_at: Option<DateTime<Utc>>,
}

async fn token_mark(
    tx: &mut crate::db::Tx<'_>,
    strategy_id: Uuid,
    token_id: &str,
    avg_price: Decimal,
) -> anyhow::Result<TokenMark> {
    let row: Option<(Decimal, DateTime<Utc>)> = sqlx::query_as(
        "SELECT f.price, f.filled_at FROM fills f
         JOIN orders o ON o.id = f.order_id
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND o.token_id = $2 AND f.paper_execution_mode = 'clob'
         ORDER BY f.filled_at DESC LIMIT 1",
    )
    .bind(strategy_id)
    .bind(token_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(match row {
        Some((price, at)) => {
            let stale = Utc::now().signed_duration_since(at).num_seconds() > STALE_MARK_SECONDS;
            TokenMark {
                price,
                source: if stale {
                    "last_fill_stale"
                } else {
                    "last_fill"
                },
                observed_at: Some(at),
            }
        }
        None => TokenMark {
            price: avg_price,
            source: "cost_basis_estimate",
            observed_at: None,
        },
    })
}

pub fn bucket_start(now: DateTime<Utc>, interval_minutes: i64) -> DateTime<Utc> {
    let secs = interval_minutes * 60;
    let floored = (now.timestamp() / secs) * secs;
    Utc.timestamp_opt(floored, 0).single().unwrap_or(now)
}

pub fn resolve_period(
    period: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<(DateTime<Utc>, DateTime<Utc>, String), String> {
    let now = Utc::now();
    if let (Some(f), Some(t)) = (from, to) {
        let start = DateTime::parse_from_rfc3339(f)
            .map_err(|_| "invalid from timestamp")?
            .with_timezone(&Utc);
        let end = DateTime::parse_from_rfc3339(t)
            .map_err(|_| "invalid to timestamp")?
            .with_timezone(&Utc);
        return Ok((start, end, "custom".into()));
    }
    let label = period.unwrap_or("1M").to_string();
    let start = match label.to_ascii_uppercase().as_str() {
        "TODAY" | "1D" => now - Duration::days(1),
        "1W" => now - Duration::weeks(1),
        "1M" => now - Duration::days(30),
        "3M" => now - Duration::days(90),
        "6M" => now - Duration::days(180),
        "1Y" => now - Duration::days(365),
        "ALL" => DateTime::<Utc>::from_naive_utc_and_offset(
            NaiveDate::from_ymd_opt(1970, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            Utc,
        ),
        other => return Err(format!("unknown period {other}")),
    };
    Ok((start, now, label.to_ascii_uppercase()))
}

pub async fn assert_strategy_owner(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
) -> anyhow::Result<(String, bool, String, String, Uuid)> {
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let row: Option<(String, bool, String, String, Uuid)> = sqlx::query_as(
        "SELECT t.alias, s.enabled, s.operating_mode, s.sizing_mode, s.trading_account_id
         FROM copy_strategies s
         JOIN source_traders t ON t.id = s.source_trader_id
         WHERE s.id = $1 AND s.user_id = $2",
    )
    .bind(strategy_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    row.ok_or_else(|| anyhow::anyhow!("strategy not found"))
}

pub async fn compute_metrics(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    cfg: &Config,
) -> anyhow::Result<StrategyMetrics> {
    let util = utilization_report_for_user(pool, user_id, strategy_id, cfg).await?;
    let mut tx = begin_as(pool, Some(user_id), "user").await?;

    let realized: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(realized_pnl_usd), 0) FROM positions WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await?;

    let fees: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(f.fee_usd), 0)
         FROM fills f
         JOIN orders o ON o.id = f.order_id
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND f.paper_execution_mode = 'clob'",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await?;

    let clob_fill_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fills f
         JOIN orders o ON o.id = f.order_id
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND f.paper_execution_mode = 'clob'",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await?;
    let legacy_unlabeled_fill_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fills f
         JOIN orders o ON o.id = f.order_id
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND f.paper_execution_mode IS NULL",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await?;
    let synthetic_experimental_fill_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fills f
         JOIN orders o ON o.id = f.order_id
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND f.paper_execution_mode = 'synthetic_source_price'",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await?;

    let position_rows: Vec<(
        Uuid,
        String,
        Option<String>,
        Option<String>,
        Decimal,
        Decimal,
        Decimal,
        Decimal,
    )> = sqlx::query_as(
        "SELECT id, token_id, market_title, outcome, open_shares, avg_price, cost_basis_usd, realized_pnl_usd
         FROM positions
         WHERE strategy_id = $1 AND open_shares > 0
         ORDER BY cost_basis_usd DESC",
    )
    .bind(strategy_id)
    .fetch_all(&mut *tx)
    .await?;

    let mut unrealized = Decimal::ZERO;
    let mut market_value = Decimal::ZERO;
    let mut saw_estimate = false;
    let mut saw_stale = false;

    for (_, token_id, _, _, open_shares, avg_price, _, _) in &position_rows {
        let tm = token_mark(&mut tx, strategy_id, token_id, *avg_price).await?;
        if tm.source == "cost_basis_estimate" {
            saw_estimate = true;
        }
        if tm.source == "last_fill_stale" {
            saw_stale = true;
        }
        let mv = (open_shares * tm.price).round_dp(8);
        market_value += mv;
        unrealized += ((tm.price - avg_price) * open_shares).round_dp(8);
    }

    let closed_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM closed_positions WHERE strategy_id = $1")
            .bind(strategy_id)
            .fetch_one(&mut *tx)
            .await?;

    let trades: i64 = clob_fill_count;

    let capital: Option<(Decimal, Decimal, Decimal, Decimal, String)> = sqlx::query_as(
        "SELECT available_cash_usd, reserved_cash_usd, invested_capital_usd, funded_capital_usd, bootstrap_quality
         FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .fetch_optional(&mut *tx)
    .await?;

    tx.commit().await?;

    let total_pnl = realized + unrealized;
    let allocated = util.effective_allocation_cap_usd;
    let roi = if allocated > Decimal::ZERO {
        Some((total_pnl / allocated * Decimal::from(100)).round_dp(4))
    } else {
        None
    };

    let (
        available_cap,
        reserved_cap,
        invested_cap,
        equity_metric,
        equity_quality,
        equity_label,
        equity_note,
    ) = if let Some((avail, res, inv, funded, ref quality)) = capital {
        if quality != "pending" {
            let eq = avail + res + market_value;
            (
                avail,
                res,
                inv,
                eq,
                "full",
                FULL_EQUITY_LABEL,
                FULL_EQUITY_NOTE,
            )
        } else if avail == Decimal::ZERO
            && res == Decimal::ZERO
            && inv == Decimal::ZERO
            && funded == Decimal::ZERO
        {
            (
                Decimal::ZERO,
                Decimal::ZERO,
                Decimal::ZERO,
                market_value + realized,
                "pending_capital",
                PENDING_CAPITAL_LABEL,
                PENDING_CAPITAL_NOTE,
            )
        } else {
            (
                avail,
                res,
                inv,
                avail + res + market_value,
                "partial",
                PARTIAL_EQUITY_LABEL,
                EQUITY_MODEL_NOTE,
            )
        }
    } else if util.deployed_capital_usd > Decimal::ZERO || util.reserved_capital_usd > Decimal::ZERO
    {
        (
            Decimal::ZERO,
            util.reserved_capital_usd,
            util.deployed_capital_usd,
            market_value + realized,
            "partial",
            PARTIAL_EQUITY_LABEL,
            EQUITY_MODEL_NOTE,
        )
    } else {
        (
            Decimal::ZERO,
            util.reserved_capital_usd,
            util.deployed_capital_usd,
            market_value + realized,
            "pending_capital",
            PENDING_CAPITAL_LABEL,
            PENDING_CAPITAL_NOTE,
        )
    };

    let bootstrap_quality = capital.as_ref().map(|(_, _, _, _, q)| q.to_string());
    let ledger_funded = capital
        .as_ref()
        .map(|(_, _, _, funded, _)| *funded)
        .unwrap_or(Decimal::ZERO);
    let ledger_allocated = capital
        .as_ref()
        .filter(|(_, _, _, funded, q)| q != "pending" && *funded > Decimal::ZERO)
        .map(|(_, _, _, funded, _)| *funded)
        .unwrap_or(Decimal::ZERO);
    let allocated_display = if ledger_allocated > Decimal::ZERO {
        ledger_allocated
    } else {
        allocated
    };
    let funding_status = capital_funding_status(allocated, ledger_funded);

    Ok(StrategyMetrics {
        allocated_capital_usd: allocated_display,
        configured_allocation_cap_usd: allocated,
        funded_capital_usd: ledger_funded,
        capital_funding_status: funding_status,
        deployed_capital_usd: util.deployed_capital_usd,
        reserved_capital_usd: reserved_cap,
        available_capital_usd: available_cap,
        invested_capital_usd: invested_cap,
        positions_market_value_usd: market_value,
        realized_pnl_usd: realized,
        unrealized_pnl_usd: unrealized,
        total_pnl_usd: total_pnl,
        fees_usd: fees,
        roi_percent: roi,
        open_positions_count: util.open_positions_count,
        closed_positions_count: closed_count,
        total_trades_count: trades,
        mark_quality: if position_rows.is_empty() {
            "n/a"
        } else if saw_estimate {
            "estimated"
        } else if saw_stale {
            "stale"
        } else {
            "last_fill"
        },
        mark_quality_note: if position_rows.is_empty() {
            None
        } else if saw_estimate {
            Some(
                "Some marks use average entry (no strategy fill yet); not a live market price."
                    .into(),
            )
        } else if saw_stale {
            Some("Last fill price is older than 24h; treat as indicative only.".into())
        } else {
            None
        },
        equity: EquityModel {
            strategy_equity_usd: equity_metric,
            equity_quality,
            equity_label,
            note: equity_note,
            cash_attribution_usd: if equity_quality == "full" {
                Some(available_cap + reserved_cap)
            } else {
                None
            },
        },
        capital_bootstrap_quality: bootstrap_quality,
        performance_fill_scope: "clob_fills_only",
        clob_fill_count,
        legacy_unlabeled_fill_count,
        synthetic_experimental_fill_count,
    })
}

pub async fn list_positions(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    status: &str,
    limit: i64,
    offset: i64,
) -> anyhow::Result<(Vec<PositionRow>, i64)> {
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let owned: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM copy_strategies WHERE id = $1 AND user_id = $2)",
    )
    .bind(strategy_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !owned {
        anyhow::bail!("strategy not found");
    }

    let open_only = status.eq_ignore_ascii_case("open");
    let closed_only = status.eq_ignore_ascii_case("closed");

    let mut rows = Vec::new();

    if !closed_only {
        let open_rows: Vec<(
            Uuid,
            String,
            Option<String>,
            Option<String>,
            Decimal,
            Decimal,
            Decimal,
            Decimal,
            Option<DateTime<Utc>>,
        )> = sqlx::query_as(
            "SELECT id, token_id, market_title, outcome, open_shares, avg_price, cost_basis_usd, realized_pnl_usd, updated_at
             FROM positions
             WHERE strategy_id = $1 AND open_shares > 0
             ORDER BY cost_basis_usd DESC
             LIMIT $2 OFFSET $3",
        )
        .bind(strategy_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&mut *tx)
        .await?;
        for (
            id,
            token_id,
            market_title,
            outcome,
            open_shares,
            avg_price,
            cost_basis,
            realized,
            updated_at,
        ) in open_rows
        {
            let tm = token_mark(&mut tx, strategy_id, &token_id, avg_price).await?;
            let mv = (open_shares * tm.price).round_dp(8);
            let unreal = ((tm.price - avg_price) * open_shares).round_dp(8);
            let ret = if cost_basis > Decimal::ZERO {
                Some((unreal / cost_basis * Decimal::from(100)).round_dp(4))
            } else {
                None
            };
            rows.push(PositionRow {
                id,
                token_id,
                market_title,
                outcome,
                status: "open",
                opened_at: updated_at,
                open_shares,
                avg_price,
                cost_basis_usd: cost_basis,
                mark_price: tm.price,
                mark_source: tm.source,
                market_value_usd: mv,
                realized_pnl_usd: realized,
                unrealized_pnl_usd: unreal,
                return_percent: ret,
                mark_observed_at: tm.observed_at,
                closed_at: None,
                opened_at_fill: None,
                avg_exit_price: None,
                proceeds_usd: None,
                position_fees_usd: None,
            });
        }
    }

    if !open_only {
        let closed_rows: Vec<(
            Uuid,
            String,
            Option<String>,
            Decimal,
            Decimal,
            DateTime<Utc>,
        )> = sqlx::query_as(
            "SELECT id, token_id, market_title, closed_shares, realized_pnl_usd, closed_at
             FROM closed_positions
             WHERE strategy_id = $1
             ORDER BY closed_at DESC
             LIMIT $2 OFFSET $3",
        )
        .bind(strategy_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&mut *tx)
        .await?;
        for (id, token_id, market_title, closed_shares, realized, closed_at) in closed_rows {
            let stats: (
                Option<DateTime<Utc>>,
                Option<DateTime<Utc>>,
                Decimal,
                Decimal,
                Decimal,
                Decimal,
            ) = sqlx::query_as(
                "SELECT
                    MIN(f.filled_at) FILTER (WHERE o.side = 'buy'),
                    MAX(f.filled_at) FILTER (WHERE o.side = 'sell'),
                    COALESCE(SUM(f.shares) FILTER (WHERE o.side = 'buy'), 0),
                    COALESCE(SUM(f.notional_usd) FILTER (WHERE o.side = 'buy'), 0),
                    COALESCE(SUM(f.notional_usd) FILTER (WHERE o.side = 'sell'), 0),
                    COALESCE(SUM(f.fee_usd), 0)
                 FROM fills f
                 JOIN orders o ON o.id = f.order_id
                 JOIN copy_decisions d ON d.id = o.decision_id
                 WHERE d.strategy_id = $1 AND o.token_id = $2",
            )
            .bind(strategy_id)
            .bind(&token_id)
            .fetch_one(&mut *tx)
            .await?;
            let (opened, _last_sell, buy_shares, buy_notional, sell_notional, fees) = stats;
            let avg_entry = if buy_shares > Decimal::ZERO {
                (buy_notional / buy_shares).round_dp(8)
            } else {
                Decimal::ZERO
            };
            let avg_exit = if closed_shares > Decimal::ZERO {
                Some((sell_notional / closed_shares).round_dp(8))
            } else {
                None
            };
            let cost = buy_notional.min(closed_shares * avg_entry).round_dp(8);
            let ret = if cost > Decimal::ZERO {
                Some((realized / cost * Decimal::from(100)).round_dp(4))
            } else {
                None
            };
            rows.push(PositionRow {
                id,
                token_id,
                market_title,
                outcome: None,
                status: "closed",
                opened_at: opened,
                open_shares: closed_shares,
                avg_price: avg_entry,
                cost_basis_usd: cost,
                mark_price: avg_exit.unwrap_or(Decimal::ZERO),
                mark_source: "exit_fill",
                market_value_usd: sell_notional,
                realized_pnl_usd: realized,
                unrealized_pnl_usd: Decimal::ZERO,
                return_percent: ret,
                mark_observed_at: Some(closed_at),
                closed_at: Some(closed_at),
                opened_at_fill: opened,
                avg_exit_price: avg_exit,
                proceeds_usd: Some(sell_notional),
                position_fees_usd: Some(fees),
            });
        }
    }

    let total_open: i64 = if open_only || status.eq_ignore_ascii_case("all") || status.is_empty() {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM positions WHERE strategy_id = $1 AND open_shares > 0",
        )
        .bind(strategy_id)
        .fetch_one(&mut *tx)
        .await?
    } else {
        0
    };

    let total = if open_only {
        total_open
    } else if closed_only {
        sqlx::query_scalar("SELECT COUNT(*) FROM closed_positions WHERE strategy_id = $1")
            .bind(strategy_id)
            .fetch_one(&mut *tx)
            .await?
    } else {
        total_open
            + sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM closed_positions WHERE strategy_id = $1",
            )
            .bind(strategy_id)
            .fetch_one(&mut *tx)
            .await?
    };

    tx.commit().await?;
    Ok((rows, total))
}

fn json_decimal(value: &serde_json::Value) -> Option<Decimal> {
    match value {
        serde_json::Value::String(s) => s.parse().ok(),
        serde_json::Value::Number(n) => n.to_string().parse().ok(),
        _ => None,
    }
}

fn sizing_audit_raw_target(checks: &serde_json::Value) -> Option<Decimal> {
    checks
        .get("sizing_audit")
        .and_then(|a| a.get("raw_target_notional_usd"))
        .and_then(json_decimal)
}

fn sizing_audit_pvr_multiplier(checks: &serde_json::Value) -> Option<Decimal> {
    checks
        .get("sizing_audit")
        .and_then(|a| a.get("pvr_trade_multiplier"))
        .and_then(json_decimal)
        .filter(|m| *m > Decimal::ZERO && *m != Decimal::ONE)
}

fn sizing_audit_mode(checks: &serde_json::Value) -> Option<String> {
    checks
        .get("sizing_audit")
        .and_then(|a| a.get("sizing_mode"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn sizing_audit_json(checks: &serde_json::Value) -> Option<&serde_json::Value> {
    checks.get("sizing_audit")
}

fn sizing_audit_decimal(audit: &serde_json::Value, key: &str) -> Option<Decimal> {
    audit.get(key).and_then(json_decimal)
}

fn execution_status(decision_status: &str, order_status: Option<&str>) -> String {
    if decision_status == "rejected" || decision_status == "skipped" {
        return "skipped".into();
    }
    match order_status {
        Some("filled") => "executed".into(),
        Some("partially_filled") => "partial".into(),
        Some("failed") | Some("rejected") | Some("cancelled") => "failed".into(),
        Some(_) => "pending".into(),
        None if decision_status == "approved" => "no_order".into(),
        _ => "skipped".into(),
    }
}


pub struct ActivityListParams<'a> {
    pub activity_type: Option<&'a str>,
    pub side: Option<&'a str>,
    pub status: Option<&'a str>,
    pub search: Option<&'a str>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub limit: i64,
    pub offset: i64,
}

fn effective_activity_type<'a>(
    activity_type: Option<&'a str>,
    side: Option<&'a str>,
) -> Option<String> {
    activity_type
        .or(side)
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
}

const ACTIVITY_FEED_SQL: &str = r#"
WITH decision_feed AS (
    SELECT
        d.id AS activity_id,
        'decision'::text AS activity_kind,
        d.decided_at AS occurred_at,
        lower(d.side) AS feed_type,
        (f.fill_id IS NOT NULL) AS has_fill,
        CASE
            WHEN d.status IN ('rejected', 'skipped') THEN 'skipped'
            WHEN o.status = 'filled' THEN 'executed'
            WHEN o.status = 'partially_filled' THEN 'partial'
            WHEN o.status IN ('failed', 'rejected', 'cancelled') THEN 'failed'
            WHEN o.status IS NOT NULL THEN 'pending'
            WHEN d.status = 'approved' THEN 'no_order'
            ELSE 'skipped'
        END AS exec_status,
        d.id AS decision_id,
        d.status AS decision_status,
        d.side,
        d.reason,
        d.source_event_id,
        d.decided_at,
        d.detected_at,
        e.event_ts AS source_event_at,
        CASE
            WHEN e.usdc_size IS NOT NULL AND e.usdc_size > 0 THEN e.usdc_size
            ELSE e.notional_usd
        END AS source_notional_usd,
        e.price AS source_price,
        CASE
            WHEN e.usdc_size IS NOT NULL AND e.usdc_size > 0 AND e.price > 0
            THEN e.usdc_size / e.price
            ELSE e.size_shares
        END AS source_shares,
        e.market_title,
        e.event_title,
        e.outcome,
        d.target_notional_usd,
        d.checks,
        o.id AS order_id,
        o.status AS order_status,
        o.submitted_at,
        o.size_shares AS order_target_shares,
        f.fill_id,
        f.filled_at,
        f.fill_price,
        f.fill_shares,
        f.fill_notional_usd,
        f.fill_legs,
        f.fee_usd,
        f.slippage_vs_source_bps,
        f.slippage_vs_limit_bps,
        f.paper_execution_mode,
        f.paper_execution_context,
        NULL::text AS ledger_entry_type,
        NULL::numeric AS settlement_payout_usd,
        o.position_open_shares_after AS position_open_shares,
        NULL::text AS close_reason,
        NULL::numeric AS close_closed_shares,
        NULL::numeric AS close_avg_entry_price,
        NULL::numeric AS close_avg_exit_price,
        NULL::numeric AS close_cost_basis_usd,
        NULL::numeric AS close_proceeds_usd,
        NULL::numeric AS close_fees_usd,
        NULL::numeric AS close_realized_pnl_usd
    FROM copy_decisions d
    JOIN copy_strategies cs ON cs.id = d.strategy_id
    LEFT JOIN source_events e ON e.id = d.source_event_id
    LEFT JOIN orders o ON o.decision_id = d.id
    LEFT JOIN LATERAL (
        SELECT
            (ARRAY_AGG(ff.id ORDER BY ff.filled_at DESC))[1] AS fill_id,
            MAX(ff.filled_at) AS filled_at,
            SUM(ff.shares) AS fill_shares,
            SUM(ff.notional_usd) AS fill_notional_usd,
            CASE
                WHEN COALESCE(SUM(ff.shares), 0) > 0
                THEN SUM(ff.price * ff.shares) / SUM(ff.shares)
                ELSE NULL
            END AS fill_price,
            SUM(ff.fee_usd) AS fee_usd,
            (ARRAY_AGG(ff.slippage_vs_source_bps ORDER BY ff.filled_at DESC))[1] AS slippage_vs_source_bps,
            (ARRAY_AGG(ff.slippage_vs_limit_bps ORDER BY ff.filled_at DESC))[1] AS slippage_vs_limit_bps,
            (ARRAY_AGG(ff.paper_execution_mode ORDER BY ff.filled_at DESC))[1] AS paper_execution_mode,
            (ARRAY_AGG(ff.paper_execution_context ORDER BY ff.filled_at DESC))[1] AS paper_execution_context,
            COALESCE(
                json_agg(
                    json_build_object(
                        'fill_id', ff.id,
                        'filled_at', ff.filled_at,
                        'price', ff.price,
                        'shares', ff.shares,
                        'notional_usd', ff.notional_usd,
                        'fee_usd', ff.fee_usd
                    )
                    ORDER BY ff.filled_at ASC
                ) FILTER (WHERE ff.id IS NOT NULL),
                '[]'::json
            ) AS fill_legs
        FROM fills ff
        WHERE ff.order_id = o.id
    ) f ON o.id IS NOT NULL
    WHERE d.strategy_id = $1
),
capital_feed AS (
    SELECT
        cle.id AS activity_id,
        'capital'::text AS activity_kind,
        cle.created_at AS occurred_at,
        cle.entry_type AS feed_type,
        false AS has_fill,
        'executed'::text AS exec_status,
        cle.id AS decision_id,
        'posted'::text AS decision_status,
        'capital'::text AS side,
        ''::text AS reason,
        '00000000-0000-0000-0000-000000000000'::uuid AS source_event_id,
        cle.created_at AS decided_at,
        cle.created_at AS detected_at,
        NULL::timestamptz AS source_event_at,
        NULL::numeric AS source_notional_usd,
        NULL::numeric AS source_price,
        NULL::numeric AS source_shares,
        NULL::text AS market_title,
        cle.entry_type AS event_title,
        NULL::text AS outcome,
        NULL::numeric AS target_notional_usd,
        '{}'::jsonb AS checks,
        cle.order_id,
        NULL::text AS order_status,
        NULL::timestamptz AS submitted_at,
        NULL::numeric AS order_target_shares,
        cle.fill_id,
        cle.created_at AS filled_at,
        NULL::numeric AS fill_price,
        NULL::numeric AS fill_shares,
        cle.amount_usd AS fill_notional_usd,
        '[]'::json AS fill_legs,
        NULL::numeric AS fee_usd,
        NULL::int AS slippage_vs_source_bps,
        NULL::int AS slippage_vs_limit_bps,
        NULL::text AS paper_execution_mode,
        NULL::jsonb AS paper_execution_context,
        cle.entry_type AS ledger_entry_type,
        cle.amount_usd AS settlement_payout_usd,
        NULL::numeric AS position_open_shares,
        NULL::text AS close_reason,
        NULL::numeric AS close_closed_shares,
        NULL::numeric AS close_avg_entry_price,
        NULL::numeric AS close_avg_exit_price,
        NULL::numeric AS close_cost_basis_usd,
        NULL::numeric AS close_proceeds_usd,
        NULL::numeric AS close_fees_usd,
        NULL::numeric AS close_realized_pnl_usd
    FROM capital_ledger_entries cle
    WHERE cle.strategy_id = $1
      AND cle.entry_type IN (
          'account_paper_funding',
          'strategy_allocation',
          'strategy_withdrawal',
          'reconciliation_adjustment',
          'order_reserve',
          'reserve_release'
      )
),
close_feed AS (
    SELECT
        pce.id AS activity_id,
        'closed'::text AS activity_kind,
        pce.closed_at AS occurred_at,
        'closed'::text AS feed_type,
        false AS has_fill,
        'closed'::text AS exec_status,
        pce.id AS decision_id,
        'closed'::text AS decision_status,
        'closed'::text AS side,
        ''::text AS reason,
        '00000000-0000-0000-0000-000000000000'::uuid AS source_event_id,
        pce.closed_at AS decided_at,
        pce.closed_at AS detected_at,
        NULL::timestamptz AS source_event_at,
        NULL::numeric AS source_notional_usd,
        NULL::numeric AS source_price,
        NULL::numeric AS source_shares,
        pce.market_title,
        NULL::text AS event_title,
        pce.outcome,
        NULL::numeric AS target_notional_usd,
        '{}'::jsonb AS checks,
        NULL::uuid AS order_id,
        NULL::text AS order_status,
        NULL::timestamptz AS submitted_at,
        NULL::numeric AS order_target_shares,
        NULL::uuid AS fill_id,
        pce.closed_at AS filled_at,
        pce.avg_exit_price AS fill_price,
        pce.closed_shares AS fill_shares,
        pce.total_proceeds_usd AS fill_notional_usd,
        '[]'::json AS fill_legs,
        pce.total_fees_usd AS fee_usd,
        NULL::int AS slippage_vs_source_bps,
        NULL::int AS slippage_vs_limit_bps,
        NULL::text AS paper_execution_mode,
        NULL::jsonb AS paper_execution_context,
        NULL::text AS ledger_entry_type,
        NULL::numeric AS settlement_payout_usd,
        NULL::numeric AS position_open_shares,
        pce.close_reason,
        pce.closed_shares AS close_closed_shares,
        pce.avg_entry_price AS close_avg_entry_price,
        pce.avg_exit_price AS close_avg_exit_price,
        pce.total_cost_basis_usd AS close_cost_basis_usd,
        pce.total_proceeds_usd AS close_proceeds_usd,
        pce.total_fees_usd AS close_fees_usd,
        pce.realized_pnl_usd AS close_realized_pnl_usd
    FROM position_close_events pce
    WHERE pce.strategy_id = $1
),
combined AS (
    SELECT * FROM decision_feed
    UNION ALL
    SELECT * FROM capital_feed
    UNION ALL
    SELECT * FROM close_feed
)
SELECT * FROM combined c
WHERE ($2::timestamptz IS NULL OR c.occurred_at >= $2)
  AND ($3::timestamptz IS NULL OR c.occurred_at < $3)
  AND ($4::text IS NULL OR c.exec_status = $4)
  AND (
    $5::text IS NULL OR
    ($5 = 'fill' AND c.activity_kind = 'decision' AND c.has_fill) OR
    ($5 = 'settlement' AND c.activity_kind = 'closed' AND c.close_reason LIKE 'resolution_%') OR
    ($5 IN ('buy', 'sell') AND c.activity_kind = 'decision' AND c.feed_type = $5) OR
    ($5 = 'closed' AND c.activity_kind = 'closed') OR
    ($5 = 'capital' AND c.activity_kind = 'capital')
  )
  AND (
    $6::text IS NULL OR
    c.market_title ILIKE '%' || $6 || '%' OR
    c.event_title ILIKE '%' || $6 || '%' OR
    c.outcome ILIKE '%' || $6 || '%'
  )
"#;

pub async fn list_activity(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    params: ActivityListParams<'_>,
) -> anyhow::Result<(Vec<ActivityRow>, i64)> {
    let effective_type = effective_activity_type(params.activity_type, params.side);
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let owned: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM copy_strategies WHERE id = $1 AND user_id = $2)",
    )
    .bind(strategy_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !owned {
        anyhow::bail!("strategy not found");
    }

    let count_sql = format!(
        "WITH filtered AS ({ACTIVITY_FEED_SQL}) SELECT COUNT(*)::bigint FROM filtered"
    );
    let total: i64 = sqlx::query_scalar(&count_sql)
        .bind(strategy_id)
        .bind(params.from)
        .bind(params.to)
        .bind(params.status)
        .bind(effective_type.as_deref())
        .bind(params.search)
        .fetch_one(&mut *tx)
        .await?;

    let page_sql = format!(
        "{ACTIVITY_FEED_SQL}
         ORDER BY c.occurred_at DESC, c.activity_id DESC
         LIMIT $7 OFFSET $8"
    );
    let raw_rows = sqlx::query(&page_sql)
        .bind(strategy_id)
        .bind(params.from)
        .bind(params.to)
        .bind(params.status)
        .bind(effective_type.as_deref())
        .bind(params.search)
        .bind(params.limit)
        .bind(params.offset)
        .fetch_all(&mut *tx)
        .await?;

    let rows = raw_rows
        .into_iter()
        .map(|row| {
            use sqlx::Row;
            let activity_kind: String = row.get("activity_kind");
            let feed_type: String = row.get("feed_type");
            let decision_id: Uuid = row.get("decision_id");
            let decision_status: String = row.get("decision_status");
            let side: String = row.get("side");
            let reason: String = row.get("reason");
            let source_event_id: Uuid = row.get("source_event_id");
            let decided_at: DateTime<Utc> = row.get("decided_at");
            let detected_at: DateTime<Utc> = row.get("detected_at");
            let source_event_at: Option<DateTime<Utc>> = row.get("source_event_at");
            let checks: serde_json::Value = row.get("checks");
            let audit = sizing_audit_json(&checks);
            let mut source_notional_usd: Option<Decimal> = row.get("source_notional_usd");
            if let Some(a) = audit {
                if let Some(d) = sizing_audit_decimal(a, "source_notional_usd") {
                    source_notional_usd = Some(d);
                }
            }
            let source_price: Option<Decimal> = row.get("source_price");
            let source_shares: Option<Decimal> = row.get("source_shares");
            let market_title: Option<String> = row.get("market_title");
            let event_title: Option<String> = row.get("event_title");
            let outcome: Option<String> = row.get("outcome");
            let target_notional_usd: Option<Decimal> = row
                .get::<Option<Decimal>, _>("target_notional_usd")
                .or_else(|| sizing_audit_raw_target(&checks))
                .filter(|d| *d > Decimal::ZERO);
            let pvr_trade_multiplier = sizing_audit_pvr_multiplier(&checks);
            let sizing_mode = sizing_audit_mode(&checks);
            let raw_target_notional_usd = audit.and_then(|a| sizing_audit_decimal(a, "raw_target_notional_usd"));
            let capped_target_notional_usd =
                audit.and_then(|a| sizing_audit_decimal(a, "capped_target_notional_usd"));
            let sizing_explanation = audit.and_then(crate::copy_sizing::explain_sizing_audit);
            let order_id: Option<Uuid> = row.get("order_id");
            let order_status: Option<String> = row.get("order_status");
            let submitted_at: Option<DateTime<Utc>> = row.get("submitted_at");
            let fill_id: Option<Uuid> = row.get("fill_id");
            let filled_at: Option<DateTime<Utc>> = row.get("filled_at");
            let fill_price: Option<Decimal> = row.get("fill_price");
            let fill_shares: Option<Decimal> = row.get("fill_shares");
            let fill_notional_usd: Option<Decimal> = row.get("fill_notional_usd");
            let order_target_shares: Option<Decimal> = row.get("order_target_shares");
            let fill_legs: Option<Vec<ActivityFillLeg>> = row
                .try_get::<Option<serde_json::Value>, _>("fill_legs")
                .ok()
                .flatten()
                .and_then(|v| {
                    if v.is_null() {
                        None
                    } else {
                        serde_json::from_value(v).ok()
                    }
                });
            let fee_usd: Option<Decimal> = row.get("fee_usd");
            let slip_src: Option<i32> = row.get("slippage_vs_source_bps");
            let slip_lim: Option<i32> = row.get("slippage_vs_limit_bps");
            let paper_execution_mode: Option<String> = row.get("paper_execution_mode");
            let paper_execution_context: Option<serde_json::Value> =
                row.get("paper_execution_context");
            let ledger_entry_type: Option<String> = row.get("ledger_entry_type");
            let settlement_payout_usd: Option<Decimal> = row.get("settlement_payout_usd");
            let position_open_shares: Option<Decimal> = row.get("position_open_shares");
            let close_reason: Option<String> = row.get("close_reason");
            let close_closed_shares: Option<Decimal> = row.get("close_closed_shares");
            let close_avg_entry_price: Option<Decimal> = row.get("close_avg_entry_price");
            let close_avg_exit_price: Option<Decimal> = row.get("close_avg_exit_price");
            let close_cost_basis_usd: Option<Decimal> = row.get("close_cost_basis_usd");
            let close_proceeds_usd: Option<Decimal> = row.get("close_proceeds_usd");
            let close_fees_usd: Option<Decimal> = row.get("close_fees_usd");
            let close_realized_pnl_usd: Option<Decimal> = row.get("close_realized_pnl_usd");
            let exec: String = row.get("exec_status");
            let (reason_title, reason_detail) = if reason.is_empty() {
                (None, None)
            } else {
                match crate::engine::decision_reason_explain(&reason) {
                    Some((t, d)) => (Some(t.to_string()), Some(d.to_string())),
                    None => (None, None),
                }
            };
            ActivityRow {
                decision_id,
                decision_status,
                side,
                reason: if reason.is_empty() {
                    None
                } else {
                    Some(reason)
                },
                reason_title,
                reason_detail,
                source_event_id,
                source_event_at,
                detected_at: Some(detected_at),
                decided_at,
                source_notional_usd,
                source_price,
                source_shares,
                market_title,
                outcome,
                target_notional_usd,
                pvr_trade_multiplier,
                sizing_mode,
                raw_target_notional_usd,
                capped_target_notional_usd,
                sizing_explanation,
                order_id,
                order_status,
                submitted_at,
                fill_id,
                filled_at,
                fill_price,
                fill_shares,
                fill_notional_usd,
                fill_legs,
                order_target_shares,
                fee_usd,
                slippage_vs_source_bps: slip_src,
                slippage_vs_limit_bps: slip_lim,
                paper_execution_mode,
                paper_execution_context,
                execution_status: exec,
                activity_kind,
                feed_type,
                ledger_entry_type,
                event_title,
                settlement_payout_usd,
                position_open_shares,
                close_reason,
                close_closed_shares,
                close_avg_entry_price,
                close_avg_exit_price,
                close_cost_basis_usd,
                close_proceeds_usd,
                close_fees_usd,
                close_realized_pnl_usd,
            }
        })
        .collect();

    tx.commit().await?;
    Ok((rows, total))
}

pub async fn compute_performance(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    cfg: &Config,
) -> anyhow::Result<PerformanceMetrics> {
    let metrics = compute_metrics(pool, user_id, strategy_id, cfg).await?;
    let mut tx = begin_as(pool, Some(user_id), "user").await?;

    let wins: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM closed_positions WHERE strategy_id = $1 AND realized_pnl_usd > 0",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await?;
    let losses: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM closed_positions WHERE strategy_id = $1 AND realized_pnl_usd < 0",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await?;
    let win_sum: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(realized_pnl_usd), 0) FROM closed_positions WHERE strategy_id = $1 AND realized_pnl_usd > 0",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await?;
    let loss_sum: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(realized_pnl_usd), 0) FROM closed_positions WHERE strategy_id = $1 AND realized_pnl_usd < 0",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await?;

    // AVG with no matching rows still returns one NULL row — use Option<Decimal> + fetch_one, not fetch_optional.
    let slip_avg: Option<Decimal> = sqlx::query_scalar(
        "SELECT AVG(f.slippage_vs_source_bps::numeric)
         FROM fills f
         JOIN orders o ON o.id = f.order_id
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND f.slippage_vs_source_bps IS NOT NULL
           AND f.paper_execution_mode = 'clob'",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await?;
    let slip_avg_clob: Option<Decimal> = sqlx::query_scalar(
        "SELECT AVG(f.slippage_vs_source_bps::numeric)
         FROM fills f
         JOIN orders o ON o.id = f.order_id
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND f.slippage_vs_source_bps IS NOT NULL
           AND f.paper_execution_mode = 'clob'",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await?;
    let slip_avg_synthetic: Option<Decimal> = sqlx::query_scalar(
        "SELECT AVG(f.slippage_vs_source_bps::numeric)
         FROM fills f
         JOIN orders o ON o.id = f.order_id
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND f.slippage_vs_source_bps IS NOT NULL
           AND f.paper_execution_mode = 'synthetic_source_price'",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await?;
    let clob_fill_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fills f
         JOIN orders o ON o.id = f.order_id
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND f.paper_execution_mode = 'clob'",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await?;
    let synthetic_fill_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fills f
         JOIN orders o ON o.id = f.order_id
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND f.paper_execution_mode = 'synthetic_source_price'",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await?;
    let legacy_unlabeled_fill_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fills f
         JOIN orders o ON o.id = f.order_id
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND f.paper_execution_mode IS NULL",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await?;

    let series = equity_series(pool, user_id, strategy_id, None, None, None).await?;
    let max_dd = max_drawdown_from_series(&series);

    tx.commit().await?;

    let closed_total = wins + losses;
    let win_rate = if closed_total >= 3 {
        Some((Decimal::from(wins) / Decimal::from(closed_total) * Decimal::from(100)).round_dp(2))
    } else {
        None
    };
    let avg_win = if wins > 0 {
        Some((win_sum / Decimal::from(wins)).round_dp(4))
    } else {
        None
    };
    let avg_loss = if losses > 0 {
        Some((loss_sum / Decimal::from(losses)).round_dp(4))
    } else {
        None
    };
    let profit_factor = if losses > 0 && win_sum > Decimal::ZERO {
        Some((win_sum / loss_sum.abs()).round_dp(4))
    } else {
        None
    };

    let sample_note = if closed_total < 3 {
        Some("Win rate and profit factor need at least 3 closed positions.".into())
    } else if series.len() < 2 {
        Some("Max drawdown requires historical snapshots (collected every 15 minutes after this feature is enabled).".into())
    } else {
        None
    };

    Ok(PerformanceMetrics {
        total_return_usd: metrics.total_pnl_usd,
        roi_percent: metrics.roi_percent,
        realized_pnl_usd: metrics.realized_pnl_usd,
        unrealized_pnl_usd: metrics.unrealized_pnl_usd,
        win_rate_percent: win_rate,
        winning_positions: wins,
        losing_positions: losses,
        avg_win_usd: avg_win,
        avg_loss_usd: avg_loss,
        profit_factor,
        max_drawdown_percent: max_dd,
        avg_holding_hours: None,
        fees_usd: metrics.fees_usd,
        slippage_bps_avg: slip_avg,
        slippage_bps_avg_clob: slip_avg_clob,
        slippage_bps_avg_synthetic: slip_avg_synthetic,
        clob_fill_count,
        synthetic_fill_count,
        legacy_unlabeled_fill_count,
        performance_fill_scope: "clob_fills_only",
        sample_size_note: sample_note,
    })
}

fn max_drawdown_from_series(series: &[EquityPoint]) -> Option<Decimal> {
    if series.len() < 2 {
        return None;
    }
    let mut peak = series[0].equity_metric_usd;
    let mut max_dd = Decimal::ZERO;
    for p in series {
        if p.equity_metric_usd > peak {
            peak = p.equity_metric_usd;
        }
        if peak > Decimal::ZERO {
            let dd = ((peak - p.equity_metric_usd) / peak * Decimal::from(100)).round_dp(4);
            if dd > max_dd {
                max_dd = dd;
            }
        }
    }
    Some(max_dd)
}

pub async fn equity_series(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    period: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> anyhow::Result<Vec<EquityPoint>> {
    let (start, end, _) = resolve_period(period, from, to).map_err(anyhow::Error::msg)?;
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let owned: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM copy_strategies WHERE id = $1 AND user_id = $2)",
    )
    .bind(strategy_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !owned {
        anyhow::bail!("strategy not found");
    }

    let rows: Vec<(
        DateTime<Utc>,
        Decimal,
        Decimal,
        Decimal,
        Decimal,
        Decimal,
        Decimal,
        Decimal,
        Decimal,
        String,
    )> = sqlx::query_as(
        "SELECT bucket_at, equity_metric_usd, realized_pnl_usd, unrealized_pnl_usd,
                deployed_capital_usd, reserved_capital_usd,
                GREATEST(allocated_capital_usd - deployed_capital_usd - reserved_capital_usd, 0),
                allocated_capital_usd,
                realized_pnl_usd + unrealized_pnl_usd,
                data_quality
         FROM strategy_analytics_snapshots
         WHERE strategy_id = $1 AND bucket_at >= $2 AND bucket_at <= $3
         ORDER BY bucket_at ASC",
    )
    .bind(strategy_id)
    .bind(start)
    .bind(end)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                at,
                equity,
                realized,
                unreal,
                deployed,
                reserved,
                available,
                allocated,
                total_pnl,
                data_quality,
            )| EquityPoint {
                at,
                equity_metric_usd: equity,
                realized_pnl_usd: realized,
                unrealized_pnl_usd: unreal,
                deployed_capital_usd: deployed,
                reserved_capital_usd: reserved,
                available_capital_usd: available,
                allocated_capital_usd: allocated,
                total_pnl_usd: total_pnl,
                data_quality,
            },
        )
        .collect())
}

pub async fn record_snapshots_for_enabled_strategies(
    pool: &PgPool,
    cfg: &Config,
) -> anyhow::Result<u64> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let ids: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as(
        "SELECT s.id, s.user_id, s.trading_account_id FROM copy_strategies s WHERE s.enabled",
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    let bucket = bucket_start(Utc::now(), SNAPSHOT_INTERVAL_MINUTES);
    let mut written = 0u64;
    for (strategy_id, user_id, trading_account_id) in ids {
        let metrics = match compute_metrics(pool, user_id, strategy_id, cfg).await {
            Ok(m) => m,
            Err(err) => {
                tracing::warn!(%strategy_id, error = %err, "strategy snapshot metrics failed");
                continue;
            }
        };
        let util = utilization_report_for_user(pool, user_id, strategy_id, cfg).await?;
        let mut tx = begin_as(pool, None, "worker").await?;
        let res = sqlx::query(
            "INSERT INTO strategy_analytics_snapshots (
                user_id, trading_account_id, strategy_id, bucket_at,
                allocated_capital_usd, deployed_capital_usd, reserved_capital_usd,
                positions_market_value_usd, realized_pnl_usd, unrealized_pnl_usd,
                fees_usd, equity_metric_usd, data_quality
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
             ON CONFLICT (strategy_id, bucket_at) DO UPDATE SET
                allocated_capital_usd = EXCLUDED.allocated_capital_usd,
                deployed_capital_usd = EXCLUDED.deployed_capital_usd,
                reserved_capital_usd = EXCLUDED.reserved_capital_usd,
                positions_market_value_usd = EXCLUDED.positions_market_value_usd,
                realized_pnl_usd = EXCLUDED.realized_pnl_usd,
                unrealized_pnl_usd = EXCLUDED.unrealized_pnl_usd,
                fees_usd = EXCLUDED.fees_usd,
                equity_metric_usd = EXCLUDED.equity_metric_usd,
                data_quality = EXCLUDED.data_quality",
        )
        .bind(user_id)
        .bind(trading_account_id)
        .bind(strategy_id)
        .bind(bucket)
        .bind(metrics.allocated_capital_usd)
        .bind(util.deployed_capital_usd)
        .bind(util.reserved_capital_usd)
        .bind(metrics.positions_market_value_usd)
        .bind(metrics.realized_pnl_usd)
        .bind(metrics.unrealized_pnl_usd)
        .bind(metrics.fees_usd)
        .bind(metrics.equity.strategy_equity_usd)
        .bind(metrics.equity.equity_quality)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        written += res.rows_affected();
    }
    Ok(written)
}

pub async fn build_overview(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    cfg: &Config,
) -> anyhow::Result<StrategyOverview> {
    let (alias, enabled, operating_mode, sizing_mode, _) =
        assert_strategy_owner(pool, user_id, strategy_id).await?;
    let metrics = compute_metrics(pool, user_id, strategy_id, cfg).await?;
    let (activity, _) = list_activity(
        pool,
        user_id,
        strategy_id,
        ActivityListParams {
            activity_type: None,
            side: None,
            status: None,
            search: None,
            from: None,
            to: None,
            limit: 8,
            offset: 0,
        },
    )
    .await?;
    let (positions, _) = list_positions(pool, user_id, strategy_id, "open", 50, 0).await?;
    let mut sorted = positions.clone();
    sorted.sort_by_key(|a| std::cmp::Reverse(a.unrealized_pnl_usd));
    let bottom_positions: Vec<PositionRow> = sorted.iter().rev().take(3).cloned().collect();
    let top_positions: Vec<PositionRow> = sorted.into_iter().take(3).collect();

    Ok(StrategyOverview {
        strategy_id,
        alias,
        enabled,
        operating_mode,
        sizing_mode,
        metrics,
        recent_activity: activity,
        top_positions,
        bottom_positions,
        as_of: Utc::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_and_period_helpers() {
        let now = Utc::now();
        let b = bucket_start(now, SNAPSHOT_INTERVAL_MINUTES);
        assert!(b <= now);
        let (s, e, label) = resolve_period(Some("1W"), None, None).unwrap();
        assert_eq!(label, "1W");
        assert!(e >= s);
    }

    #[test]
    fn execution_status_skipped_vs_executed() {
        assert_eq!(execution_status("skipped", None), "skipped");
        assert_eq!(execution_status("approved", Some("filled")), "executed");
        assert_eq!(execution_status("approved", Some("pending")), "pending");
    }

    #[test]
    fn partial_equity_label_is_explicit() {
        assert!(PARTIAL_EQUITY_LABEL.to_lowercase().contains("partial"));
    }

    #[test]
    fn capital_funding_status_from_ledger_only() {
        use rust_decimal::Decimal;
        use super::{capital_funding_status, CapitalFundingStatus};
        assert_eq!(
            capital_funding_status(Decimal::from(2000), Decimal::ZERO),
            CapitalFundingStatus::NotFunded
        );
        assert_eq!(
            capital_funding_status(Decimal::from(2000), Decimal::from(500)),
            CapitalFundingStatus::PartiallyFunded
        );
        assert_eq!(
            capital_funding_status(Decimal::from(2000), Decimal::from(2000)),
            CapitalFundingStatus::Funded
        );
        assert_eq!(
            capital_funding_status(Decimal::from(2000), Decimal::from(2500)),
            CapitalFundingStatus::Funded
        );
    }
}
