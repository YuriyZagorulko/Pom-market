use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::account_allocation::{
    effective_allocation_cap, paper_account_equity_detail, AllocationMode, StrategyAllocation,
};
use crate::config::Config;
use crate::copy_sizing::scaling_ratio;
use crate::db::begin_as;
use crate::source_equity;

#[derive(Debug, Clone, Serialize)]
pub struct BudgetUtilization {
    pub allocated_budget_usd: Decimal,
    pub deployed_capital_usd: Decimal,
    pub reserved_capital_usd: Decimal,
    pub available_budget_usd: Decimal,
    pub open_positions_count: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StrategyUtilizationReport {
    pub allocated_budget_usd: Decimal,
    pub configured_allocation_mode: String,
    pub configured_allocation_percent: Option<Decimal>,
    pub configured_fixed_usd: Option<Decimal>,
    pub effective_allocation_cap_usd: Decimal,
    pub account_equity_usd: Decimal,
    pub account_paper_cash_usd: Decimal,
    pub account_positions_market_value_usd: Decimal,
    pub account_order_reserves_usd: Decimal,
    pub peer_allocations_usd: Decimal,
    pub unallocated_account_usd: Decimal,
    pub deployed_capital_usd: Decimal,
    pub reserved_capital_usd: Decimal,
    pub available_budget_usd: Decimal,
    pub open_positions_count: i64,
    pub updated_at: DateTime<Utc>,
    pub source_equity_usd: Option<Decimal>,
    pub source_equity_status: Option<String>,
    pub source_equity_quality_note: Option<String>,
    pub source_equity_observed_at: Option<DateTime<Utc>>,
    pub source_equity_age_seconds: Option<i64>,
    pub scaling_ratio_percent: Option<Decimal>,
    pub sizing_mode: String,
}

const OPEN_ORDER_STATUSES: &[&str] = &[
    "pending",
    "submitting",
    "submitted",
    "partially_filled",
    "unknown",
];

pub async fn utilization_for_user(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    cfg: &Config,
) -> anyhow::Result<BudgetUtilization> {
    let report = utilization_report_for_user(pool, user_id, strategy_id, cfg).await?;
    Ok(BudgetUtilization {
        allocated_budget_usd: report.effective_allocation_cap_usd,
        deployed_capital_usd: report.deployed_capital_usd,
        reserved_capital_usd: report.reserved_capital_usd,
        available_budget_usd: report.available_budget_usd,
        open_positions_count: report.open_positions_count,
        updated_at: report.updated_at,
    })
}

pub async fn utilization_report_for_user(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    cfg: &Config,
) -> anyhow::Result<StrategyUtilizationReport> {
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let owns: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM copy_strategies WHERE id = $1 AND user_id = $2)",
    )
    .bind(strategy_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !owns {
        anyhow::bail!("strategy not found");
    }
    let row: (
        Uuid,
        String,
        Decimal,
        Option<Decimal>,
        String,
        Uuid,
    ) = sqlx::query_as(
        "SELECT trading_account_id, allocation_mode, strategy_budget_usd, allocation_percent, sizing_mode, source_trader_id
         FROM copy_strategies WHERE id = $1",
    )
    .bind(strategy_id)
    .fetch_one(&mut *tx)
    .await?;
    let (account_id, alloc_mode, budget, alloc_pct, sizing_mode, source_trader_id) = row;
    let equity_detail = paper_account_equity_detail(&mut tx, account_id).await?;
    let alloc = StrategyAllocation {
        allocation_mode: AllocationMode::parse(&alloc_mode).unwrap_or(AllocationMode::FixedUsd),
        strategy_budget_usd: budget,
        allocation_percent: alloc_pct,
    };
    let effective = effective_allocation_cap(&alloc, equity_detail.account_equity_usd);
    let util = utilization_in_tx(&mut tx, strategy_id, effective).await?;
    tx.commit().await?;

    let equity_snap = source_equity::load_snapshot(pool, source_trader_id, cfg).await?;
    let scaling = equity_snap
        .as_ref()
        .and_then(|s| s.total_equity_usd)
        .and_then(|eq| scaling_ratio(effective, eq));
    let configured_fixed = if alloc.allocation_mode == AllocationMode::FixedUsd {
        Some(budget)
    } else {
        None
    };

    Ok(StrategyUtilizationReport {
        allocated_budget_usd: util.allocated_budget_usd,
        configured_allocation_mode: alloc_mode,
        configured_allocation_percent: alloc_pct,
        configured_fixed_usd: configured_fixed,
        effective_allocation_cap_usd: effective,
        account_equity_usd: equity_detail.account_equity_usd,
        account_paper_cash_usd: equity_detail.paper_cash_usd,
        account_positions_market_value_usd: equity_detail.positions_market_value_usd,
        account_order_reserves_usd: equity_detail.open_order_reserves_usd,
        peer_allocations_usd: equity_detail.allocated_across_strategies_usd,
        unallocated_account_usd: equity_detail.unallocated_usd,
        deployed_capital_usd: util.deployed_capital_usd,
        reserved_capital_usd: util.reserved_capital_usd,
        available_budget_usd: util.available_budget_usd,
        open_positions_count: util.open_positions_count,
        updated_at: util.updated_at,
        source_equity_usd: equity_snap.as_ref().and_then(|s| s.total_equity_usd),
        source_equity_status: equity_snap.as_ref().map(|s| s.status.as_str().to_string()),
        source_equity_quality_note: equity_snap.as_ref().and_then(|s| s.quality_note.clone()),
        source_equity_observed_at: equity_snap.as_ref().map(|s| s.observed_at),
        source_equity_age_seconds: equity_snap.as_ref().map(|s| s.age_seconds),
        scaling_ratio_percent: scaling,
        sizing_mode,
    })
}

async fn utilization_in_tx(
    tx: &mut crate::db::Tx<'_>,
    strategy_id: Uuid,
    allocated: Decimal,
) -> anyhow::Result<BudgetUtilization> {
    let bootstrapped = crate::strategy_accounting::is_capital_bootstrapped(tx, strategy_id).await?;
    let deployed: Decimal = if bootstrapped {
        let cap = crate::strategy_accounting::lock_strategy_capital(tx, strategy_id).await?;
        cap.invested_capital_usd
    } else {
        sqlx::query_scalar(
            "SELECT COALESCE(SUM(cost_basis_usd), 0) FROM positions
             WHERE strategy_id = $1 AND open_shares > 0",
        )
        .bind(strategy_id)
        .fetch_one(&mut **tx)
        .await?
    };

    let reserved: Decimal = if bootstrapped {
        crate::strategy_accounting::strategy_liquid_balances(tx, strategy_id)
            .await?
            .1
    } else {
        sqlx::query_scalar(
            "SELECT COALESCE(SUM(o.position_reserved_usd + o.fee_reserved_usd), 0)
             FROM orders o
             JOIN copy_decisions d ON d.id = o.decision_id
             WHERE d.strategy_id = $1
               AND o.status = ANY($2::text[])",
        )
        .bind(strategy_id)
        .bind(
            OPEN_ORDER_STATUSES
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>(),
        )
        .fetch_one(&mut **tx)
        .await?
    };

    let open_positions_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM positions WHERE strategy_id = $1 AND open_shares > 0",
    )
    .bind(strategy_id)
    .fetch_one(&mut **tx)
    .await?;

    let updated_at: DateTime<Utc> = sqlx::query_scalar(
        "SELECT GREATEST(
            COALESCE((SELECT MAX(updated_at) FROM positions WHERE strategy_id = $1), '-infinity'::timestamptz),
            COALESCE((SELECT MAX(o.updated_at) FROM orders o JOIN copy_decisions d ON d.id = o.decision_id WHERE d.strategy_id = $1), '-infinity'::timestamptz),
            (SELECT updated_at FROM copy_strategies WHERE id = $1)
         )",
    )
    .bind(strategy_id)
    .fetch_one(&mut **tx)
    .await?;

    let funded_on_ledger: Option<Decimal> = sqlx::query_scalar(
        "SELECT funded_capital_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .fetch_optional(&mut **tx)
    .await?;
    let available = if bootstrapped || funded_on_ledger.is_some_and(|f| f > Decimal::ZERO) {
        crate::strategy_accounting::strategy_liquid_balances(tx, strategy_id)
            .await?
            .0
    } else {
        Decimal::ZERO
    };

    Ok(BudgetUtilization {
        allocated_budget_usd: allocated,
        deployed_capital_usd: deployed,
        reserved_capital_usd: reserved,
        available_budget_usd: available,
        open_positions_count,
        updated_at,
    })
}

pub async fn strategy_exposure_with_cap(
    tx: &mut crate::db::Tx<'_>,
    strategy_id: Uuid,
    trading_account_id: Uuid,
    allocation_mode: &str,
    strategy_budget_usd: Decimal,
    allocation_percent: Option<Decimal>,
) -> anyhow::Result<(Decimal, Decimal)> {
    let equity =
        crate::account_allocation::paper_account_equity_usd(tx, trading_account_id).await?;
    let alloc = StrategyAllocation {
        allocation_mode: AllocationMode::parse(allocation_mode).unwrap_or(AllocationMode::FixedUsd),
        strategy_budget_usd,
        allocation_percent,
    };
    let cap = effective_allocation_cap(&alloc, equity);
    let util = utilization_in_tx(tx, strategy_id, cap).await?;
    Ok((util.deployed_capital_usd + util.reserved_capital_usd, cap))
}
