//! Per-strategy capital allocation on a trading account (PAPER first).

use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::Tx;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AllocationMode {
    FixedUsd,
    PercentOfAccount,
}

impl AllocationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FixedUsd => "fixed_usd",
            Self::PercentOfAccount => "percent_of_account",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fixed_usd" => Some(Self::FixedUsd),
            "percent_of_account" => Some(Self::PercentOfAccount),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct StrategyAllocation {
    pub allocation_mode: AllocationMode,
    pub strategy_budget_usd: Decimal,
    pub allocation_percent: Option<Decimal>,
}

/// PAPER account equity breakdown for allocation UI and validation.
#[derive(Clone, Debug, Serialize)]
pub struct PaperAccountEquity {
    pub paper_cash_usd: Decimal,
    /// Mark-to-model: `open_shares * avg_price` (paper has no live marks; not cost_basis label in API).
    pub positions_market_value_usd: Decimal,
    pub open_order_reserves_usd: Decimal,
    /// `paper_cash + positions_market_value` (reserves are already reflected in reduced cash when orders open).
    pub account_equity_usd: Decimal,
    pub allocated_across_strategies_usd: Decimal,
    pub unallocated_usd: Decimal,
}

pub async fn paper_account_equity_detail(
    tx: &mut Tx<'_>,
    trading_account_id: Uuid,
) -> anyhow::Result<PaperAccountEquity> {
    let cash: Decimal = sqlx::query_scalar(
        "SELECT paper_cash_usd FROM account_risk_limits WHERE trading_account_id = $1",
    )
    .bind(trading_account_id)
    .fetch_one(&mut **tx)
    .await?;
    let positions_market: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(open_shares * avg_price), 0) FROM positions
         WHERE trading_account_id = $1 AND operating_mode = 'paper_trading' AND open_shares > 0",
    )
    .bind(trading_account_id)
    .fetch_one(&mut **tx)
    .await?;
    let reserves: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(position_reserved_usd + fee_reserved_usd), 0) FROM orders
         WHERE trading_account_id = $1 AND operating_mode = 'paper_trading'
           AND status IN ('pending','submitting','submitted','partially_filled','unknown')",
    )
    .bind(trading_account_id)
    .fetch_one(&mut **tx)
    .await?;
    let equity = (cash + positions_market).round_dp(2);
    let allocated = sum_peer_allocations_usd(tx, trading_account_id, None, equity).await?;
    let unallocated =
        if crate::strategy_accounting::account_has_bootstrapped_capital(tx, trading_account_id)
            .await?
        {
            crate::strategy_accounting::unallocated_cash_in_tx(tx, trading_account_id).await?
        } else {
            (equity - allocated).max(Decimal::ZERO).round_dp(2)
        };
    Ok(PaperAccountEquity {
        paper_cash_usd: cash,
        positions_market_value_usd: positions_market.round_dp(2),
        open_order_reserves_usd: reserves.round_dp(2),
        account_equity_usd: equity,
        allocated_across_strategies_usd: allocated,
        unallocated_usd: unallocated.round_dp(2),
    })
}

/// Back-compat helper for sizing paths.
pub async fn paper_account_equity_usd(
    tx: &mut Tx<'_>,
    trading_account_id: Uuid,
) -> anyhow::Result<Decimal> {
    Ok(paper_account_equity_detail(tx, trading_account_id)
        .await?
        .account_equity_usd)
}

pub fn effective_allocation_cap(alloc: &StrategyAllocation, account_equity: Decimal) -> Decimal {
    match alloc.allocation_mode {
        AllocationMode::FixedUsd => alloc.strategy_budget_usd.round_dp(2),
        AllocationMode::PercentOfAccount => {
            let pct = alloc.allocation_percent.unwrap_or(Decimal::ZERO);
            if pct <= Decimal::ZERO || pct > Decimal::from(100) {
                return Decimal::ZERO;
            }
            (account_equity * pct / Decimal::from(100))
                .round_dp_with_strategy(2, RoundingStrategy::ToZero)
        }
    }
}

pub async fn sum_peer_allocations_usd(
    tx: &mut Tx<'_>,
    trading_account_id: Uuid,
    exclude_strategy_id: Option<Uuid>,
    account_equity: Decimal,
) -> anyhow::Result<Decimal> {
    let rows: Vec<(String, Decimal, Option<Decimal>)> = sqlx::query_as(
        "SELECT allocation_mode, strategy_budget_usd, allocation_percent
         FROM copy_strategies
         WHERE trading_account_id = $1 AND ($2::uuid IS NULL OR id <> $2)",
    )
    .bind(trading_account_id)
    .bind(exclude_strategy_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut sum = Decimal::ZERO;
    for (mode, budget, pct) in rows {
        let alloc = StrategyAllocation {
            allocation_mode: AllocationMode::parse(&mode).unwrap_or(AllocationMode::FixedUsd),
            strategy_budget_usd: budget,
            allocation_percent: pct,
        };
        sum += effective_allocation_cap(&alloc, account_equity);
    }
    Ok(sum.round_dp(2))
}

pub fn parse_allocation_from_request(
    allocation_mode: Option<&str>,
    strategy_budget_usd: Decimal,
    allocation_percent: Option<Decimal>,
) -> Result<StrategyAllocation, String> {
    let mode = allocation_mode
        .and_then(AllocationMode::parse)
        .unwrap_or(AllocationMode::FixedUsd);
    match mode {
        AllocationMode::FixedUsd => {
            if strategy_budget_usd <= Decimal::ZERO {
                return Err("strategy budget must be positive for fixed allocation".into());
            }
            Ok(StrategyAllocation {
                allocation_mode: mode,
                strategy_budget_usd,
                allocation_percent: None,
            })
        }
        AllocationMode::PercentOfAccount => {
            let pct = allocation_percent.ok_or_else(|| {
                "allocation_percent is required for percent_of_account".to_string()
            })?;
            if pct <= Decimal::ZERO || pct > Decimal::from(100) {
                return Err("allocation_percent must be between 0 and 100".into());
            }
            Ok(StrategyAllocation {
                allocation_mode: mode,
                strategy_budget_usd,
                allocation_percent: Some(pct),
            })
        }
    }
}

pub async fn validate_new_allocation_in_tx(
    tx: &mut Tx<'_>,
    trading_account_id: Uuid,
    alloc: &StrategyAllocation,
    exclude_strategy_id: Option<Uuid>,
) -> Result<(), String> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1::text))")
        .bind(trading_account_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    let equity = paper_account_equity_usd(tx, trading_account_id)
        .await
        .map_err(|e| e.to_string())?;
    let peer = sum_peer_allocations_usd(tx, trading_account_id, exclude_strategy_id, equity)
        .await
        .map_err(|e| e.to_string())?;
    let cap = effective_allocation_cap(alloc, equity);
    if cap <= Decimal::ZERO {
        return Err("allocation must be greater than zero".into());
    }
    if peer + cap > equity {
        return Err(format!(
            "strategy allocations would exceed account equity (${} peer + ${} new > ${} equity)",
            peer, cap, equity
        ));
    }
    Ok(())
}

pub async fn validate_new_allocation(
    pool: &PgPool,
    user_id: Uuid,
    trading_account_id: Uuid,
    alloc: &StrategyAllocation,
    exclude_strategy_id: Option<Uuid>,
) -> Result<(), String> {
    let mut tx = crate::db::begin_as(pool, Some(user_id), "user")
        .await
        .map_err(|e| e.to_string())?;
    validate_new_allocation_in_tx(&mut tx, trading_account_id, alloc, exclude_strategy_id).await?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}
