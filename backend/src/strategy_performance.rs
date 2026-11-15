//! Strategy Performance report: summary rankings, paginated positions, key events, execution stats.
//!
//! Reuses [`crate::strategy_analytics`] metrics and position rows; does not duplicate ledger math.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;
use crate::db::begin_as;
use crate::strategy_analytics::{
    compute_metrics, compute_performance, list_positions, position_row_total_pnl, resolve_period,
    PerformanceMetrics, PositionRow, StrategyMetrics,
};

pub const MAX_POSITIONS_MATERIALIZE: i64 = 2_000;
const KEY_EVENT_FILL_FLOOR_USD: Decimal = Decimal::from_parts(25, 0, 0, false, 0);

#[derive(Debug, Clone, Serialize)]
pub struct PerformanceDashboard {
    pub metrics: StrategyMetrics,
    pub performance: PerformanceMetrics,
    pub top_winners: Vec<PositionRow>,
    pub top_losers: Vec<PositionRow>,
    pub mark_quality_note: Option<String>,
    pub period: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct KeyEventRow {
    pub id: String,
    pub at: DateTime<Utc>,
    pub kind: String,
    pub title: String,
    pub detail: Option<String>,
    pub amount_usd: Decimal,
    pub market_title: Option<String>,
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionStats {
    pub period: String,
    pub source_events_seen: i64,
    pub decisions_total: i64,
    pub decisions_approved: i64,
    pub decisions_skipped: i64,
    pub decisions_rejected: i64,
    pub orders_filled: i64,
    pub orders_partial: i64,
    pub fills_clob: i64,
    pub fills_synthetic: i64,
    pub fills_legacy_unlabeled: i64,
    pub leader_notional_usd: Decimal,
    pub copy_notional_usd: Decimal,
    pub avg_copy_notional_usd: Option<Decimal>,
    pub skip_reason_top: Vec<(String, i64)>,
    pub performance_fill_scope: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct PositionHistoryRow {
    pub at: DateTime<Utc>,
    pub kind: String,
    pub side: Option<String>,
    pub shares: Option<Decimal>,
    pub price: Option<Decimal>,
    pub notional_usd: Option<Decimal>,
    pub fee_usd: Option<Decimal>,
    pub execution_mode: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PerformancePositionQuery {
    pub status: &'static str,
    pub sort: &'static str,
    pub pnl_filter: Option<&'static str>,
    pub search: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

pub async fn build_performance_dashboard(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    cfg: &Config,
    period: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> anyhow::Result<PerformanceDashboard> {
    let (_, _, label) = resolve_period(period, from, to).map_err(anyhow::Error::msg)?;
    let metrics = compute_metrics(pool, user_id, strategy_id, cfg).await?;
    let performance = compute_performance(pool, user_id, strategy_id, cfg).await?;
    let top_winners = rank_positions(pool, user_id, strategy_id, true, 5).await?;
    let top_losers = rank_positions(pool, user_id, strategy_id, false, 5).await?;
    let mark_quality_note = metrics.mark_quality_note.clone();
    Ok(PerformanceDashboard {
        metrics,
        performance,
        top_winners,
        top_losers,
        mark_quality_note,
        period: label,
    })
}

fn position_matches_search(row: &PositionRow, q: &str) -> bool {
    let needle = q.to_ascii_lowercase();
    row.market_title
        .as_ref()
        .is_some_and(|t| t.to_ascii_lowercase().contains(&needle))
        || row.outcome
            .as_ref()
            .is_some_and(|o| o.to_ascii_lowercase().contains(&needle))
        || row.token_id.to_ascii_lowercase().contains(&needle)
}

fn position_matches_pnl_filter(row: &PositionRow, filter: &str) -> bool {
    let pnl = position_row_total_pnl(row);
    match filter {
        "profit" => pnl > Decimal::ZERO,
        "loss" => pnl < Decimal::ZERO,
        _ => true,
    }
}

fn sort_positions(rows: &mut [PositionRow], sort: &str) {
    match sort {
        "profit_asc" => rows.sort_by(|a, b| position_row_total_pnl(a).cmp(&position_row_total_pnl(b))),
        "roi_desc" => rows.sort_by(|a, b| {
            let roi_a = a.return_percent.unwrap_or(Decimal::ZERO);
            let roi_b = b.return_percent.unwrap_or(Decimal::ZERO);
            roi_b
                .cmp(&roi_a)
                .then_with(|| position_row_total_pnl(b).cmp(&position_row_total_pnl(a)))
        }),
        "invested_desc" => rows.sort_by(|a, b| b.cost_basis_usd.cmp(&a.cost_basis_usd)),
        "closed_recent" => rows.sort_by(|a, b| {
            let ta = a.closed_at.unwrap_or(a.opened_at.unwrap_or(DateTime::<Utc>::MIN_UTC));
            let tb = b.closed_at.unwrap_or(b.opened_at.unwrap_or(DateTime::<Utc>::MIN_UTC));
            tb.cmp(&ta)
        }),
        _ => rows.sort_by(|a, b| position_row_total_pnl(b).cmp(&position_row_total_pnl(a))),
    }
}

async fn materialize_positions(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    status: &str,
) -> anyhow::Result<Vec<PositionRow>> {
    let cap = MAX_POSITIONS_MATERIALIZE;
    let mut rows = Vec::new();
    if status.eq_ignore_ascii_case("open") || status.eq_ignore_ascii_case("all") {
        let (open, _) = list_positions(pool, user_id, strategy_id, "open", cap, 0).await?;
        rows.extend(open);
    }
    if status.eq_ignore_ascii_case("closed") || status.eq_ignore_ascii_case("all") {
        let (closed, _) = list_positions(pool, user_id, strategy_id, "closed", cap, 0).await?;
        rows.extend(closed);
    }
    Ok(rows)
}

pub async fn list_performance_positions(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    q: PerformancePositionQuery,
) -> anyhow::Result<(Vec<PositionRow>, i64, bool)> {
    let mut rows = materialize_positions(pool, user_id, strategy_id, q.status).await?;
    if let Some(search) = q.search.as_deref().filter(|s| !s.trim().is_empty()) {
        rows.retain(|r| position_matches_search(r, search.trim()));
    }
    if let Some(pf) = q.pnl_filter {
        rows.retain(|r| position_matches_pnl_filter(r, pf));
    }
    sort_positions(&mut rows, q.sort);
    let total = rows.len() as i64;
    let truncated = total > MAX_POSITIONS_MATERIALIZE;
    let start = q.offset.min(total) as usize;
    let end = (q.offset + q.limit).min(total) as usize;
    let page = if start < end {
        rows[start..end].to_vec()
    } else {
        Vec::new()
    };
    Ok((page, total, truncated))
}

pub async fn rank_positions(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    winners: bool,
    limit: i64,
) -> anyhow::Result<Vec<PositionRow>> {
    let mut rows = materialize_positions(pool, user_id, strategy_id, "all").await?;
    sort_positions(&mut rows, if winners { "profit_desc" } else { "profit_asc" });
    if winners {
        rows.retain(|r| position_row_total_pnl(r) > Decimal::ZERO);
    } else {
        rows.retain(|r| position_row_total_pnl(r) < Decimal::ZERO);
    }
    Ok(rows.into_iter().take(limit as usize).collect())
}

pub async fn list_key_events(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    period: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
    limit: i64,
) -> anyhow::Result<Vec<KeyEventRow>> {
    let (start, end, _) = resolve_period(period, from, to).map_err(anyhow::Error::msg)?;
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let ledger_rows: Vec<(
        Uuid,
        DateTime<Utc>,
        String,
        Decimal,
        serde_json::Value,
    )> = sqlx::query_as(
        "SELECT id, created_at, entry_type, amount_usd, metadata
         FROM capital_ledger_entries
         WHERE strategy_id = $1 AND created_at >= $2 AND created_at < $3
           AND entry_type = ANY($4::text[])
         ORDER BY abs(amount_usd) DESC, created_at DESC
         LIMIT $5",
    )
    .bind(strategy_id)
    .bind(start)
    .bind(end)
    .bind(
        &[
            "resolution_redeem",
            "resolution_loss",
            "resolution_refund",
            "hard_close_sell",
            "buy_fill",
            "sell_fill",
            "strategy_allocation",
            "trading_fee",
        ][..],
    )
    .bind(limit.clamp(1, 200))
    .fetch_all(&mut *tx)
    .await?;

    let mut events: Vec<KeyEventRow> = ledger_rows
        .into_iter()
        .filter(|(_, _, entry_type, amount, _)| {
            if matches!(
                entry_type.as_str(),
                "buy_fill" | "sell_fill" | "trading_fee"
            ) {
                return amount.abs() >= KEY_EVENT_FILL_FLOOR_USD;
            }
            true
        })
        .map(|(id, at, entry_type, amount, meta)| {
            let market_title = meta
                .get("market_title")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let outcome = meta
                .get("outcome")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            KeyEventRow {
                id: id.to_string(),
                at,
                kind: entry_type.clone(),
                title: key_event_title(&entry_type, amount),
                detail: key_event_detail(&entry_type, &meta),
                amount_usd: amount,
                market_title,
                outcome,
            }
        })
        .collect();

    let decisions: Vec<(Uuid, DateTime<Utc>, String, String, Option<String>, Option<String>)> =
        sqlx::query_as(
            "SELECT d.id, d.decided_at, d.status, d.reason, e.market_title, e.outcome
             FROM copy_decisions d
             JOIN source_events e ON e.id = d.source_event_id
             WHERE d.strategy_id = $1 AND d.decided_at >= $2 AND d.decided_at < $3
               AND d.status IN ('skipped', 'rejected')
             ORDER BY d.decided_at DESC
             LIMIT 30",
        )
        .bind(strategy_id)
        .bind(start)
        .bind(end)
        .fetch_all(&mut *tx)
        .await?;

    for (id, at, status, reason, market, outcome) in decisions {
        events.push(KeyEventRow {
            id: format!("decision-{id}"),
            at,
            kind: format!("copy_{status}"),
            title: format!("Copy {status}: {reason}"),
            detail: Some("Pipeline decision did not create a fill.".into()),
            amount_usd: Decimal::ZERO,
            market_title: market,
            outcome,
        });
    }

    events.sort_by(|a, b| {
        b.amount_usd
            .abs()
            .cmp(&a.amount_usd.abs())
            .then_with(|| b.at.cmp(&a.at))
    });
    events.truncate(limit.clamp(1, 200) as usize);
    tx.commit().await?;
    Ok(events)
}

fn key_event_title(entry_type: &str, amount: Decimal) -> String {
    match entry_type {
        "resolution_redeem" => format!("Market won · payout {amount}"),
        "resolution_loss" => format!("Market lost · {amount}"),
        "resolution_refund" => format!("Resolution refund · {amount}"),
        "hard_close_sell" => format!("Hard close sell · {amount}"),
        "buy_fill" => format!("Buy fill · {amount}"),
        "sell_fill" => format!("Sell fill · {amount}"),
        "strategy_allocation" => format!("Capital allocated · {amount}"),
        "trading_fee" => format!("Trading fee · {amount}"),
        other => format!("{other} · {amount}"),
    }
}

fn key_event_detail(entry_type: &str, meta: &serde_json::Value) -> Option<String> {
    if matches!(
        entry_type,
        "resolution_redeem" | "resolution_loss" | "resolution_refund"
    ) {
        return meta
            .get("note")
            .and_then(|v| v.as_str())
            .map(str::to_string);
    }
    None
}

pub async fn compute_execution_stats(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    period: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> anyhow::Result<ExecutionStats> {
    let (start, end, label) = resolve_period(period, from, to).map_err(anyhow::Error::msg)?;
    let mut tx = begin_as(pool, Some(user_id), "user").await?;

    let (
        decisions_total,
        decisions_approved,
        decisions_skipped,
        decisions_rejected,
        leader_notional,
    ): (i64, i64, i64, i64, Decimal) = sqlx::query_as(
        "SELECT
            COUNT(*)::bigint,
            COUNT(*) FILTER (WHERE d.status = 'approved')::bigint,
            COUNT(*) FILTER (WHERE d.status = 'skipped')::bigint,
            COUNT(*) FILTER (WHERE d.status = 'rejected')::bigint,
            COALESCE(SUM(e.notional_usd), 0)
         FROM copy_decisions d
         JOIN source_events e ON e.id = d.source_event_id
         WHERE d.strategy_id = $1 AND d.decided_at >= $2 AND d.decided_at < $3",
    )
    .bind(strategy_id)
    .bind(start)
    .bind(end)
    .fetch_one(&mut *tx)
    .await?;

    let source_events_seen: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT d.source_event_id)
         FROM copy_decisions d
         WHERE d.strategy_id = $1 AND d.decided_at >= $2 AND d.decided_at < $3",
    )
    .bind(strategy_id)
    .bind(start)
    .bind(end)
    .fetch_one(&mut *tx)
    .await?;

    let (
        orders_filled,
        orders_partial,
        copy_notional,
        fills_clob,
        fills_synthetic,
        fills_legacy,
    ): (i64, i64, Decimal, i64, i64, i64) = sqlx::query_as(
        "SELECT
            COUNT(*) FILTER (WHERE o.status = 'filled')::bigint,
            COUNT(*) FILTER (WHERE o.status = 'partially_filled')::bigint,
            COALESCE(SUM(f.notional_usd) FILTER (WHERE f.id IS NOT NULL), 0),
            COUNT(*) FILTER (WHERE f.paper_execution_mode = 'clob')::bigint,
            COUNT(*) FILTER (WHERE f.paper_execution_mode = 'synthetic_source_price')::bigint,
            COUNT(*) FILTER (WHERE f.paper_execution_mode IS NULL)::bigint
         FROM copy_decisions d
         LEFT JOIN orders o ON o.decision_id = d.id
         LEFT JOIN fills f ON f.order_id = o.id
         WHERE d.strategy_id = $1 AND d.decided_at >= $2 AND d.decided_at < $3",
    )
    .bind(strategy_id)
    .bind(start)
    .bind(end)
    .fetch_one(&mut *tx)
    .await?;

    let skip_reason_top: Vec<(String, i64)> = sqlx::query_as(
        "SELECT reason, COUNT(*)::bigint
         FROM copy_decisions
         WHERE strategy_id = $1 AND decided_at >= $2 AND decided_at < $3 AND status = 'skipped'
         GROUP BY reason ORDER BY COUNT(*) DESC LIMIT 8",
    )
    .bind(strategy_id)
    .bind(start)
    .bind(end)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;

    let fill_count = fills_clob + fills_synthetic + fills_legacy;
    let avg_copy = if fill_count > 0 {
        Some((copy_notional / Decimal::from(fill_count)).round_dp(4))
    } else {
        None
    };

    Ok(ExecutionStats {
        period: label,
        source_events_seen,
        decisions_total,
        decisions_approved,
        decisions_skipped,
        decisions_rejected,
        orders_filled,
        orders_partial,
        fills_clob,
        fills_synthetic,
        fills_legacy_unlabeled: fills_legacy,
        leader_notional_usd: leader_notional,
        copy_notional_usd: copy_notional,
        avg_copy_notional_usd: avg_copy,
        skip_reason_top,
        performance_fill_scope: "clob_fills_only",
    })
}

pub async fn position_history(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    token_id: &str,
) -> anyhow::Result<Vec<PositionHistoryRow>> {
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

    let mut rows = Vec::new();

    let fills: Vec<(
        DateTime<Utc>,
        String,
        Decimal,
        Decimal,
        Decimal,
        Option<Decimal>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT f.filled_at, o.side, f.shares, f.price, f.notional_usd, f.fee_usd, f.paper_execution_mode
         FROM fills f
         JOIN orders o ON o.id = f.order_id
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND o.token_id = $2
         ORDER BY f.filled_at ASC",
    )
    .bind(strategy_id)
    .bind(token_id)
    .fetch_all(&mut *tx)
    .await?;

    for (at, side, shares, price, notional, fee, mode) in fills {
        rows.push(PositionHistoryRow {
            at,
            kind: "fill".into(),
            side: Some(side),
            shares: Some(shares),
            price: Some(price),
            notional_usd: Some(notional),
            fee_usd: fee,
            execution_mode: mode,
            note: None,
        });
    }

    let ledger: Vec<(DateTime<Utc>, String, Decimal, serde_json::Value)> = sqlx::query_as(
        "SELECT created_at, entry_type, amount_usd, metadata
         FROM capital_ledger_entries
         WHERE strategy_id = $1
           AND (metadata->>'token_id' = $2 OR metadata->>'token_id' IS NULL)
           AND entry_type = ANY($3::text[])
         ORDER BY created_at ASC",
    )
    .bind(strategy_id)
    .bind(token_id)
    .bind(&[
        "resolution_redeem",
        "resolution_loss",
        "resolution_refund",
        "hard_close_sell",
    ][..])
    .fetch_all(&mut *tx)
    .await?;

    for (at, entry_type, amount, meta) in ledger {
        if meta
            .get("token_id")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t != token_id)
        {
            continue;
        }
        rows.push(PositionHistoryRow {
            at,
            kind: entry_type.clone(),
            side: Some("sell".into()),
            shares: None,
            price: None,
            notional_usd: Some(amount),
            fee_usd: None,
            execution_mode: None,
            note: meta.get("note").and_then(|v| v.as_str()).map(str::to_string),
        });
    }

    rows.sort_by_key(|r| r.at);
    tx.commit().await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

  fn sample_row(pnl: &str, roi: &str, cost: &str) -> PositionRow {
        PositionRow {
            id: Uuid::new_v4(),
            token_id: "t".into(),
            market_title: None,
            outcome: None,
            status: "open",
            opened_at: None,
            open_shares: Decimal::ONE,
            avg_price: Decimal::ONE,
            cost_basis_usd: cost.parse().unwrap(),
            mark_price: Decimal::ONE,
            mark_source: "test",
            market_value_usd: Decimal::ONE,
            realized_pnl_usd: Decimal::ZERO,
            unrealized_pnl_usd: pnl.parse().unwrap(),
            return_percent: Some(roi.parse().unwrap()),
            mark_observed_at: None,
            closed_at: None,
            opened_at_fill: None,
            avg_exit_price: None,
            proceeds_usd: None,
            position_fees_usd: None,
        }
    }

    #[test]
    fn sort_profit_prefers_dollar_pnl_over_roi() {
        let mut rows = vec![
            sample_row("1", "100", "1"),
            sample_row("100", "10", "1000"),
        ];
        sort_positions(&mut rows, "profit_desc");
        assert_eq!(position_row_total_pnl(&rows[0]), Decimal::from(100));
    }
}
