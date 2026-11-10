//! Per-strategy PAPER cash ledger and virtual capital accounts.
//!
//! Invariant (per strategy): `funded = available + reserved + invested` (modulo rounding).
//! Account: `paper_cash_usd >= unallocated + SUM(available + reserved)` over strategies on account.
//! Manual lots (`strategy_id IS NULL`) stay on account-level cash / positions only.

use rust_decimal::Decimal;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::account_allocation::{
    effective_allocation_cap, paper_account_equity_detail, paper_account_equity_usd,
    parse_allocation_from_request, AllocationMode, StrategyAllocation,
};
use crate::db::Tx;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StrategyCapitalRow {
    pub strategy_id: Uuid,
    pub trading_account_id: Uuid,
    pub funded_capital_usd: Decimal,
    pub available_cash_usd: Decimal,
    pub reserved_cash_usd: Decimal,
    pub invested_capital_usd: Decimal,
    pub bootstrap_quality: String,
    pub bootstrap_note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountReconciliation {
    pub trading_account_id: Uuid,
    pub paper_cash_usd: Decimal,
    pub unallocated_cash_usd: Decimal,
    pub strategy_liquid_usd: Decimal,
    pub strategy_invested_usd: Decimal,
    pub manual_position_cost_usd: Decimal,
    pub open_order_reserves_usd: Decimal,
    pub configured_allocation_targets_usd: Decimal,
    pub reconciled: bool,
    pub discrepancy_usd: Decimal,
    pub notes: Vec<String>,
    pub strategies: Vec<StrategyReconciliationLine>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StrategyReconciliationLine {
    pub strategy_id: Uuid,
    pub funded_capital_usd: Decimal,
    pub available_cash_usd: Decimal,
    pub reserved_cash_usd: Decimal,
    pub invested_capital_usd: Decimal,
    pub deployed_from_positions_usd: Decimal,
    pub reserved_from_orders_usd: Decimal,
    pub configured_allocation_target_usd: Decimal,
    pub bootstrap_quality: String,
    pub proposed_bootstrap: bool,
}

pub async fn lock_strategy_capital(
    tx: &mut Tx<'_>,
    strategy_id: Uuid,
) -> anyhow::Result<StrategyCapitalRow> {
    let row: StrategyCapitalRow = sqlx::query_as(
        "SELECT strategy_id, trading_account_id, funded_capital_usd, available_cash_usd,
                reserved_cash_usd, invested_capital_usd, bootstrap_quality, bootstrap_note
         FROM strategy_capital_accounts
         WHERE strategy_id = $1
         FOR UPDATE",
    )
    .bind(strategy_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row)
}

pub async fn is_capital_bootstrapped(tx: &mut Tx<'_>, strategy_id: Uuid) -> anyhow::Result<bool> {
    let quality: Option<String> = sqlx::query_scalar(
        "SELECT bootstrap_quality FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(matches!(
        quality.as_deref(),
        Some("migrated") | Some("estimated") | Some("needs_review")
    ))
}

pub async fn account_has_bootstrapped_capital(
    tx: &mut Tx<'_>,
    trading_account_id: Uuid,
) -> anyhow::Result<bool> {
    let any: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM strategy_capital_accounts
            WHERE trading_account_id = $1 AND bootstrap_quality <> 'pending'
         )",
    )
    .bind(trading_account_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(any)
}

pub async fn strategy_liquid_balances(
    tx: &mut Tx<'_>,
    strategy_id: Uuid,
) -> anyhow::Result<(Decimal, Decimal)> {
    let row: Option<(Decimal, Decimal)> = sqlx::query_as(
        "SELECT available_cash_usd, reserved_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.unwrap_or((Decimal::ZERO, Decimal::ZERO)))
}

/// Release copy BUY order reserves back to strategy available cash before order row zeroes reserves.
#[allow(clippy::type_complexity)]
pub async fn release_copy_order_reserves(
    tx: &mut Tx<'_>,
    order_id: Uuid,
    idempotency_key: &str,
) -> anyhow::Result<()> {
    let row: Option<(Uuid, Uuid, String, String, Decimal, Decimal, Option<Uuid>)> = sqlx::query_as(
        "SELECT o.user_id, o.trading_account_id, o.operating_mode, o.side,
                o.position_reserved_usd, o.fee_reserved_usd, d.strategy_id
         FROM orders o
         LEFT JOIN copy_decisions d ON d.id = o.decision_id
         WHERE o.id = $1
         FOR UPDATE OF o",
    )
    .bind(order_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((user_id, account_id, mode, side, pos_r, fee_r, strategy_id)) = row else {
        return Ok(());
    };
    if mode != "paper_trading" || side != "buy" {
        return Ok(());
    }
    let release = pos_r + fee_r;
    if release <= Decimal::ZERO {
        return Ok(());
    }
    let Some(strategy_id) = strategy_id else {
        return Ok(());
    };
    if !is_capital_bootstrapped(tx, strategy_id).await? {
        return Ok(());
    }
    release_order_reserve(
        tx,
        user_id,
        account_id,
        strategy_id,
        order_id,
        release,
        idempotency_key,
    )
    .await?;
    Ok(())
}

/// Explicit account-level PAPER cash funding (audited); does not allocate to strategies.
pub async fn apply_account_paper_funding(
    tx: &mut Tx<'_>,
    user_id: Uuid,
    trading_account_id: Uuid,
    delta_usd: Decimal,
    idempotency_key: &str,
    reason: &str,
) -> Result<(), String> {
    if delta_usd == Decimal::ZERO {
        return Err("funding delta must be non-zero".into());
    }
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1::text))")
        .bind(trading_account_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    let fresh: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM capital_ledger_entries WHERE idempotency_key = $1)",
    )
    .bind(idempotency_key)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    if !fresh {
        return Ok(());
    }
    if delta_usd < Decimal::ZERO {
        let paper: Decimal = sqlx::query_scalar(
            "SELECT paper_cash_usd FROM account_risk_limits WHERE trading_account_id = $1",
        )
        .bind(trading_account_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
        let unallocated = unallocated_cash_in_tx(tx, trading_account_id)
            .await
            .map_err(|e| e.to_string())?;
        if (-delta_usd) > unallocated.min(paper) {
            return Err("withdrawal exceeds unallocated paper cash".into());
        }
    }
    let updated = sqlx::query(
        "UPDATE account_risk_limits SET paper_cash_usd = paper_cash_usd + $2, updated_at = now()
         WHERE trading_account_id = $1",
    )
    .bind(trading_account_id)
    .bind(delta_usd)
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    if updated.rows_affected() == 0 {
        return Err("account risk limits row missing; create account or patch risk first".into());
    }
    insert_ledger(
        tx,
        user_id,
        trading_account_id,
        None,
        None,
        None,
        None,
        "account_paper_funding",
        delta_usd,
        idempotency_key,
        serde_json::json!({ "reason": reason, "delta_usd": delta_usd }),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn ledger_idempotency_exists(
    tx: &mut Tx<'_>,
    idempotency_key: &str,
) -> anyhow::Result<bool> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM capital_ledger_entries WHERE idempotency_key = $1)",
    )
    .bind(idempotency_key)
    .fetch_one(&mut **tx)
    .await?;
    Ok(exists)
}

pub async fn ensure_strategy_capital_row(
    tx: &mut Tx<'_>,
    user_id: Uuid,
    trading_account_id: Uuid,
    strategy_id: Uuid,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO strategy_capital_accounts (strategy_id, user_id, trading_account_id)
         VALUES ($1, $2, $3)
         ON CONFLICT (strategy_id) DO NOTHING",
    )
    .bind(strategy_id)
    .bind(user_id)
    .bind(trading_account_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_ledger(
    tx: &mut Tx<'_>,
    user_id: Uuid,
    trading_account_id: Uuid,
    strategy_id: Option<Uuid>,
    order_id: Option<Uuid>,
    fill_id: Option<Uuid>,
    position_id: Option<Uuid>,
    entry_type: &str,
    amount_usd: Decimal,
    idempotency_key: &str,
    metadata: serde_json::Value,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO capital_ledger_entries (
            user_id, trading_account_id, strategy_id, order_id, fill_id, position_id,
            entry_type, amount_usd, idempotency_key, metadata
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
         ON CONFLICT (idempotency_key) DO NOTHING",
    )
    .bind(user_id)
    .bind(trading_account_id)
    .bind(strategy_id)
    .bind(order_id)
    .bind(fill_id)
    .bind(position_id)
    .bind(entry_type)
    .bind(amount_usd)
    .bind(idempotency_key)
    .bind(metadata)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Move cash from account unallocated pool into strategy available balance.
pub async fn allocate_to_strategy(
    tx: &mut Tx<'_>,
    user_id: Uuid,
    trading_account_id: Uuid,
    strategy_id: Uuid,
    amount: Decimal,
    idempotency_key: &str,
) -> Result<(), String> {
    if amount <= Decimal::ZERO {
        return Err("allocation amount must be positive".into());
    }
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1::text))")
        .bind(trading_account_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    ensure_strategy_capital_row(tx, user_id, trading_account_id, strategy_id)
        .await
        .map_err(|e| e.to_string())?;
    let unallocated = unallocated_cash_in_tx(tx, trading_account_id)
        .await
        .map_err(|e| e.to_string())?;
    if amount > unallocated {
        return Err(format!(
            "insufficient unallocated cash: need ${amount}, unallocated ${unallocated}"
        ));
    }
    let inserted: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM capital_ledger_entries WHERE idempotency_key = $1)",
    )
    .bind(idempotency_key)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    if !inserted {
        return Ok(());
    }
    sqlx::query(
        "UPDATE strategy_capital_accounts
         SET funded_capital_usd = funded_capital_usd + $2,
             available_cash_usd = available_cash_usd + $2,
             updated_at = now()
         WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .bind(amount)
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    insert_ledger(
        tx,
        user_id,
        trading_account_id,
        Some(strategy_id),
        None,
        None,
        None,
        "strategy_allocation",
        amount,
        idempotency_key,
        serde_json::json!({ "amount_usd": amount }),
    )
    .await
    .map_err(|e| e.to_string())?;
    mark_strategy_capital_initialized(tx, strategy_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// After explicit allocation, mark ledger active so the pipeline may trade.
async fn mark_strategy_capital_initialized(
    tx: &mut Tx<'_>,
    strategy_id: Uuid,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE strategy_capital_accounts
         SET bootstrap_quality = 'migrated',
             initialized_at = COALESCE(initialized_at, now()),
             updated_at = now()
         WHERE strategy_id = $1
           AND bootstrap_quality = 'pending'
           AND funded_capital_usd > 0",
    )
    .bind(strategy_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Align funded ledger balance with the configured allocation cap (PAPER).
pub async fn sync_strategy_funded_to_target(
    tx: &mut Tx<'_>,
    user_id: Uuid,
    trading_account_id: Uuid,
    strategy_id: Uuid,
    target_funded: Decimal,
    idempotency_prefix: &str,
) -> Result<(), String> {
    if target_funded < Decimal::ZERO {
        return Err("allocation target must be non-negative".into());
    }
    ensure_strategy_capital_row(tx, user_id, trading_account_id, strategy_id)
        .await
        .map_err(|e| e.to_string())?;
    let cap = lock_strategy_capital(tx, strategy_id)
        .await
        .map_err(|e| e.to_string())?;
    if cap.bootstrap_quality != "pending" {
        let min_funded = cap.invested_capital_usd + cap.reserved_cash_usd;
        if target_funded < min_funded {
            return Err(format!(
                "strategy budget cannot be below capital already deployed (${min_funded} invested+reserved); close positions or withdraw reserves first"
            ));
        }
    } else if cap.invested_capital_usd > Decimal::ZERO || cap.reserved_cash_usd > Decimal::ZERO {
        return Err(
            "strategy capital is pending but has legacy exposure; run account capital bootstrap first"
                .into(),
        );
    }
    let delta = target_funded - cap.funded_capital_usd;
    if delta > Decimal::ZERO {
        let key = format!("{idempotency_prefix}:alloc:{delta}");
        allocate_to_strategy(tx, user_id, trading_account_id, strategy_id, delta, &key).await?;
    } else if delta < Decimal::ZERO {
        let key = format!("{idempotency_prefix}:withdraw:{}", -delta);
        withdraw_from_strategy(tx, user_id, trading_account_id, strategy_id, -delta, &key).await?;
    }
    Ok(())
}

/// Fund or withdraw ledger cash to match the strategy's configured allocation cap (PAPER only).
pub async fn sync_strategy_capital_to_configured_cap(
    tx: &mut Tx<'_>,
    user_id: Uuid,
    strategy_id: Uuid,
    idempotency_prefix: &str,
) -> Result<(), String> {
    let row: Option<(Uuid, String, Decimal, Option<Decimal>, String)> = sqlx::query_as(
        "SELECT s.trading_account_id, s.allocation_mode, s.strategy_budget_usd, s.allocation_percent, a.operating_mode
         FROM copy_strategies s
         JOIN trading_accounts a ON a.id = s.trading_account_id
         WHERE s.id = $1 AND s.user_id = $2",
    )
    .bind(strategy_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    let Some((account_id, mode, budget, pct, operating_mode)) = row else {
        return Err("strategy not found".into());
    };
    if operating_mode != "paper_trading" {
        return Ok(());
    }
    let alloc = parse_allocation_from_request(Some(&mode), budget, pct)?;
    let equity = paper_account_equity_usd(tx, account_id)
        .await
        .map_err(|e| e.to_string())?;
    let target = effective_allocation_cap(&alloc, equity);
    sync_strategy_funded_to_target(
        tx,
        user_id,
        account_id,
        strategy_id,
        target,
        idempotency_prefix,
    )
    .await
}

pub async fn withdraw_from_strategy(
    tx: &mut Tx<'_>,
    user_id: Uuid,
    trading_account_id: Uuid,
    strategy_id: Uuid,
    amount: Decimal,
    idempotency_key: &str,
) -> Result<(), String> {
    if amount <= Decimal::ZERO {
        return Err("withdrawal amount must be positive".into());
    }
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1::text))")
        .bind(trading_account_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    let cap = lock_strategy_capital(tx, strategy_id)
        .await
        .map_err(|e| e.to_string())?;
    if amount > cap.available_cash_usd {
        return Err(format!(
            "cannot withdraw ${amount}: only ${} available (reserved ${} invested ${})",
            cap.available_cash_usd, cap.reserved_cash_usd, cap.invested_capital_usd
        ));
    }
    let inserted: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM capital_ledger_entries WHERE idempotency_key = $1)",
    )
    .bind(idempotency_key)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    if !inserted {
        return Ok(());
    }
    sqlx::query(
        "UPDATE strategy_capital_accounts
         SET funded_capital_usd = funded_capital_usd - $2,
             available_cash_usd = available_cash_usd - $2,
             updated_at = now()
         WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .bind(amount)
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    insert_ledger(
        tx,
        user_id,
        trading_account_id,
        Some(strategy_id),
        None,
        None,
        None,
        "strategy_withdrawal",
        -amount,
        idempotency_key,
        serde_json::json!({ "amount_usd": amount }),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Reserve strategy cash when a paper order is opened (mirrors order USD reserves).
pub async fn reserve_order_cash(
    tx: &mut Tx<'_>,
    user_id: Uuid,
    trading_account_id: Uuid,
    strategy_id: Uuid,
    order_id: Uuid,
    total_reserve: Decimal,
    idempotency_key: &str,
) -> anyhow::Result<()> {
    if total_reserve <= Decimal::ZERO {
        return Ok(());
    }
    ensure_strategy_capital_row(tx, user_id, trading_account_id, strategy_id).await?;
    let cap = lock_strategy_capital(tx, strategy_id).await?;
    if total_reserve > cap.available_cash_usd {
        anyhow::bail!(
            "strategy insufficient cash for reserve: need {total_reserve}, available {}",
            cap.available_cash_usd
        );
    }
    let fresh: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM capital_ledger_entries WHERE idempotency_key = $1)",
    )
    .bind(idempotency_key)
    .fetch_one(&mut **tx)
    .await?;
    if !fresh {
        return Ok(());
    }
    sqlx::query(
        "UPDATE strategy_capital_accounts
         SET available_cash_usd = available_cash_usd - $2,
             reserved_cash_usd = reserved_cash_usd + $2,
             updated_at = now()
         WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .bind(total_reserve)
    .execute(&mut **tx)
    .await?;
    insert_ledger(
        tx,
        user_id,
        trading_account_id,
        Some(strategy_id),
        Some(order_id),
        None,
        None,
        "order_reserve",
        -total_reserve,
        idempotency_key,
        serde_json::json!({ "reserve_usd": total_reserve }),
    )
    .await?;
    Ok(())
}

pub async fn release_order_reserve(
    tx: &mut Tx<'_>,
    user_id: Uuid,
    trading_account_id: Uuid,
    strategy_id: Uuid,
    order_id: Uuid,
    release_usd: Decimal,
    idempotency_key: &str,
) -> anyhow::Result<()> {
    if release_usd <= Decimal::ZERO {
        return Ok(());
    }
    let _ = lock_strategy_capital(tx, strategy_id).await?;
    let fresh: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM capital_ledger_entries WHERE idempotency_key = $1)",
    )
    .bind(idempotency_key)
    .fetch_one(&mut **tx)
    .await?;
    if !fresh {
        return Ok(());
    }
    sqlx::query(
        "UPDATE strategy_capital_accounts
         SET available_cash_usd = available_cash_usd + $2,
             reserved_cash_usd = GREATEST(reserved_cash_usd - $2, 0),
             updated_at = now()
         WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .bind(release_usd)
    .execute(&mut **tx)
    .await?;
    insert_ledger(
        tx,
        user_id,
        trading_account_id,
        Some(strategy_id),
        Some(order_id),
        None,
        None,
        "reserve_release",
        release_usd,
        idempotency_key,
        serde_json::json!({ "release_usd": release_usd }),
    )
    .await?;
    Ok(())
}

/// PAPER BUY fill: spend reserved (then available), increase invested, debit account `paper_cash_usd`.
#[allow(clippy::too_many_arguments)]
pub async fn apply_paper_buy_fill(
    tx: &mut Tx<'_>,
    user_id: Uuid,
    trading_account_id: Uuid,
    strategy_id: Uuid,
    order_id: Uuid,
    fill_id: Uuid,
    notional: Decimal,
    fee: Decimal,
    idempotency_key: &str,
) -> anyhow::Result<()> {
    let total_cost = notional + fee;
    let fresh: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM capital_ledger_entries WHERE idempotency_key = $1)",
    )
    .bind(idempotency_key)
    .fetch_one(&mut **tx)
    .await?;
    if !fresh {
        return Ok(());
    }
    let cap = lock_strategy_capital(tx, strategy_id).await?;
    let from_reserved = cap.reserved_cash_usd.min(total_cost);
    let from_available = total_cost - from_reserved;
    if from_available > cap.available_cash_usd {
        anyhow::bail!(
            "strategy cash shortfall on buy: need {}, reserved {}, available {}",
            total_cost,
            cap.reserved_cash_usd,
            cap.available_cash_usd
        );
    }
    sqlx::query(
        "UPDATE strategy_capital_accounts
         SET reserved_cash_usd = reserved_cash_usd - $2,
             available_cash_usd = available_cash_usd - $3,
             invested_capital_usd = invested_capital_usd + $4,
             updated_at = now()
         WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .bind(from_reserved)
    .bind(from_available)
    .bind(total_cost)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        "UPDATE account_risk_limits SET paper_cash_usd = paper_cash_usd - $2, updated_at = now()
         WHERE trading_account_id = $1",
    )
    .bind(trading_account_id)
    .bind(total_cost)
    .execute(&mut **tx)
    .await?;

    insert_ledger(
        tx,
        user_id,
        trading_account_id,
        Some(strategy_id),
        Some(order_id),
        Some(fill_id),
        None,
        "buy_fill",
        -total_cost,
        idempotency_key,
        serde_json::json!({ "notional_usd": notional, "fee_usd": fee }),
    )
    .await?;
    Ok(())
}

/// PAPER SELL fill: reduce invested by released cost basis, credit proceeds (net of fee) to available.
#[allow(clippy::too_many_arguments)]
pub async fn apply_paper_sell_fill(
    tx: &mut Tx<'_>,
    user_id: Uuid,
    trading_account_id: Uuid,
    strategy_id: Option<Uuid>,
    order_id: Uuid,
    fill_id: Uuid,
    proceeds_net: Decimal,
    cost_released: Decimal,
    idempotency_key: &str,
    entry_type: &str,
) -> anyhow::Result<()> {
    let fresh: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM capital_ledger_entries WHERE idempotency_key = $1)",
    )
    .bind(idempotency_key)
    .fetch_one(&mut **tx)
    .await?;
    if !fresh {
        return Ok(());
    }
    if let Some(sid) = strategy_id {
        let _ = lock_strategy_capital(tx, sid).await?;
        sqlx::query(
            "UPDATE strategy_capital_accounts
             SET invested_capital_usd = GREATEST(invested_capital_usd - $2, 0),
                 available_cash_usd = available_cash_usd + $3,
                 updated_at = now()
             WHERE strategy_id = $1",
        )
        .bind(sid)
        .bind(cost_released)
        .bind(proceeds_net)
        .execute(&mut **tx)
        .await?;
    }
    if strategy_id.is_none() {
        sqlx::query(
            "UPDATE account_risk_limits SET paper_cash_usd = paper_cash_usd + $2, updated_at = now()
             WHERE trading_account_id = $1",
        )
        .bind(trading_account_id)
        .bind(proceeds_net)
        .execute(&mut **tx)
        .await?;
    }
    insert_ledger(
        tx,
        user_id,
        trading_account_id,
        strategy_id,
        Some(order_id),
        Some(fill_id),
        None,
        entry_type,
        proceeds_net,
        idempotency_key,
        serde_json::json!({ "proceeds_net_usd": proceeds_net, "cost_released_usd": cost_released }),
    )
    .await?;
    Ok(())
}

/// Unallocated cash = account `paper_cash_usd` minus sum of strategy liquid balances.
pub async fn unallocated_cash_in_tx(
    tx: &mut Tx<'_>,
    trading_account_id: Uuid,
) -> anyhow::Result<Decimal> {
    let paper: Decimal = sqlx::query_scalar(
        "SELECT paper_cash_usd FROM account_risk_limits WHERE trading_account_id = $1",
    )
    .bind(trading_account_id)
    .fetch_one(&mut **tx)
    .await?;
    let strategy_liquid: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(available_cash_usd + reserved_cash_usd), 0)
         FROM strategy_capital_accounts WHERE trading_account_id = $1",
    )
    .bind(trading_account_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok((paper - strategy_liquid).max(Decimal::ZERO))
}

pub async fn strategy_available_cash(
    tx: &mut Tx<'_>,
    strategy_id: Uuid,
) -> anyhow::Result<Decimal> {
    ensure_row_or_zero(tx, strategy_id).await
}

async fn ensure_row_or_zero(tx: &mut Tx<'_>, strategy_id: Uuid) -> anyhow::Result<Decimal> {
    let v: Option<Decimal> = sqlx::query_scalar(
        "SELECT available_cash_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(v.unwrap_or(Decimal::ZERO))
}

/// Dry-run or apply bootstrap of strategy capital rows from existing positions/orders.
pub async fn bootstrap_account_strategies(
    pool: &PgPool,
    trading_account_id: Uuid,
    apply: bool,
) -> anyhow::Result<AccountReconciliation> {
    let mut tx = crate::db::begin_as(pool, None, "worker").await?;
    let report = reconcile_account_in_tx(&mut tx, trading_account_id).await?;
    if apply {
        for line in &report.strategies {
            let quality = if report.reconciled {
                "migrated"
            } else {
                "estimated"
            };
            let note = if report.reconciled {
                None::<String>
            } else {
                Some(report.discrepancy_usd.to_string())
            };
            let funded =
                line.available_cash_usd + line.reserved_cash_usd + line.invested_capital_usd;
            let existing_quality: Option<String> = sqlx::query_scalar(
                "SELECT bootstrap_quality FROM strategy_capital_accounts WHERE strategy_id = $1",
            )
            .bind(line.strategy_id)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(ref q) = existing_quality {
                if q != "pending" {
                    continue;
                }
                sqlx::query(
                    "UPDATE strategy_capital_accounts SET
                        funded_capital_usd = $2,
                        available_cash_usd = $3,
                        reserved_cash_usd = $4,
                        invested_capital_usd = $5,
                        bootstrap_quality = $6,
                        bootstrap_note = $7,
                        initialized_at = COALESCE(initialized_at, now()),
                        updated_at = now()
                     WHERE strategy_id = $1",
                )
                .bind(line.strategy_id)
                .bind(funded)
                .bind(line.available_cash_usd)
                .bind(line.reserved_cash_usd)
                .bind(line.invested_capital_usd)
                .bind(quality)
                .bind(&note)
                .execute(&mut *tx)
                .await?;
                continue;
            }
            let user_id: Uuid =
                sqlx::query_scalar("SELECT user_id FROM copy_strategies WHERE id = $1")
                    .bind(line.strategy_id)
                    .fetch_one(&mut *tx)
                    .await?;
            sqlx::query(
                "INSERT INTO strategy_capital_accounts (
                    strategy_id, user_id, trading_account_id,
                    funded_capital_usd, available_cash_usd, reserved_cash_usd, invested_capital_usd,
                    bootstrap_quality, bootstrap_note, initialized_at
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,now())",
            )
            .bind(line.strategy_id)
            .bind(user_id)
            .bind(trading_account_id)
            .bind(funded)
            .bind(line.available_cash_usd)
            .bind(line.reserved_cash_usd)
            .bind(line.invested_capital_usd)
            .bind(quality)
            .bind(note)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
    } else {
        tx.rollback().await?;
    }
    Ok(report)
}

pub async fn reconcile_account(
    pool: &PgPool,
    trading_account_id: Uuid,
) -> anyhow::Result<AccountReconciliation> {
    let mut tx = crate::db::begin_as(pool, None, "worker").await?;
    let report = reconcile_account_in_tx(&mut tx, trading_account_id).await?;
    tx.rollback().await?;
    Ok(report)
}

async fn reconcile_account_in_tx(
    tx: &mut Tx<'_>,
    trading_account_id: Uuid,
) -> anyhow::Result<AccountReconciliation> {
    let paper: Decimal = sqlx::query_scalar(
        "SELECT paper_cash_usd FROM account_risk_limits WHERE trading_account_id = $1",
    )
    .bind(trading_account_id)
    .fetch_one(&mut **tx)
    .await?;
    let equity_detail = paper_account_equity_detail(tx, trading_account_id).await?;
    let order_reserves: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(position_reserved_usd + fee_reserved_usd), 0) FROM orders
         WHERE trading_account_id = $1 AND operating_mode = 'paper_trading'
           AND status IN ('pending','submitting','submitted','partially_filled','unknown')",
    )
    .bind(trading_account_id)
    .fetch_one(&mut **tx)
    .await?;
    let manual_cost: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(cost_basis_usd), 0) FROM positions
         WHERE trading_account_id = $1 AND strategy_id IS NULL AND operating_mode = 'paper_trading' AND open_shares > 0",
    )
    .bind(trading_account_id)
    .fetch_one(&mut **tx)
    .await?;
    let strategies: Vec<(Uuid, String, Decimal, Option<Decimal>)> = sqlx::query_as(
        "SELECT id, allocation_mode, strategy_budget_usd, allocation_percent
         FROM copy_strategies WHERE trading_account_id = $1",
    )
    .bind(trading_account_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut lines = Vec::new();
    let mut strategy_liquid = Decimal::ZERO;
    let mut strategy_invested = Decimal::ZERO;
    for (sid, mode, budget, pct) in strategies {
        let alloc = StrategyAllocation {
            allocation_mode: AllocationMode::parse(&mode).unwrap_or(AllocationMode::FixedUsd),
            strategy_budget_usd: budget,
            allocation_percent: pct,
        };
        let cap = effective_allocation_cap(&alloc, equity_detail.account_equity_usd);
        let deployed: Decimal = sqlx::query_scalar(
            "SELECT COALESCE(SUM(cost_basis_usd), 0) FROM positions WHERE strategy_id = $1 AND open_shares > 0",
        )
        .bind(sid)
        .fetch_one(&mut **tx)
        .await?;
        let reserved: Decimal = sqlx::query_scalar(
            "SELECT COALESCE(SUM(o.position_reserved_usd + o.fee_reserved_usd), 0)
             FROM orders o JOIN copy_decisions d ON d.id = o.decision_id
             WHERE d.strategy_id = $1 AND o.status IN ('pending','submitting','submitted','partially_filled','unknown')",
        )
        .bind(sid)
        .fetch_one(&mut **tx)
        .await?;
        let available = (cap - deployed - reserved).max(Decimal::ZERO);
        let existing: Option<StrategyCapitalRow> = sqlx::query_as(
            "SELECT strategy_id, trading_account_id, funded_capital_usd, available_cash_usd,
                    reserved_cash_usd, invested_capital_usd, bootstrap_quality, bootstrap_note
             FROM strategy_capital_accounts WHERE strategy_id = $1",
        )
        .bind(sid)
        .fetch_optional(&mut **tx)
        .await?;
        let line = if let Some(row) = existing {
            let use_targets = row.bootstrap_quality == "pending";
            StrategyReconciliationLine {
                strategy_id: sid,
                funded_capital_usd: if use_targets {
                    available + reserved + deployed
                } else {
                    row.funded_capital_usd
                },
                available_cash_usd: if use_targets {
                    available
                } else {
                    row.available_cash_usd
                },
                reserved_cash_usd: if use_targets {
                    reserved
                } else {
                    row.reserved_cash_usd
                },
                invested_capital_usd: if use_targets {
                    deployed
                } else {
                    row.invested_capital_usd
                },
                deployed_from_positions_usd: deployed,
                reserved_from_orders_usd: reserved,
                configured_allocation_target_usd: cap,
                bootstrap_quality: row.bootstrap_quality.clone(),
                proposed_bootstrap: use_targets,
            }
        } else {
            StrategyReconciliationLine {
                strategy_id: sid,
                funded_capital_usd: available + reserved + deployed,
                available_cash_usd: available,
                reserved_cash_usd: reserved,
                invested_capital_usd: deployed,
                deployed_from_positions_usd: deployed,
                reserved_from_orders_usd: reserved,
                configured_allocation_target_usd: cap,
                bootstrap_quality: "pending".into(),
                proposed_bootstrap: true,
            }
        };
        strategy_liquid += line.available_cash_usd + line.reserved_cash_usd;
        strategy_invested += line.invested_capital_usd;
        lines.push(line);
    }
    let unallocated = (paper - strategy_liquid).max(Decimal::ZERO);
    let expected_paper = unallocated + strategy_liquid;
    let discrepancy = paper - expected_paper;
    let reconciled = discrepancy.abs() < Decimal::new(1, 2);
    let mut notes = Vec::new();
    if manual_cost > Decimal::ZERO {
        notes.push(
            "Manual positions (strategy_id IS NULL) remain account-level; they are not assigned to copy strategies."
                .into(),
        );
    }
    if order_reserves > Decimal::ZERO
        && !account_has_bootstrapped_capital(tx, trading_account_id).await?
    {
        notes.push(format!(
            "Open order reserves (${order_reserves}) are not yet mirrored on strategy capital accounts until bootstrap."
        ));
    }
    if !reconciled {
        notes.push(format!(
            "Paper cash minus strategy liquid balances differs by ${discrepancy}; manual lots or pre-ledger history may explain the gap."
        ));
    }
    Ok(AccountReconciliation {
        trading_account_id,
        paper_cash_usd: paper,
        unallocated_cash_usd: unallocated,
        strategy_liquid_usd: strategy_liquid,
        strategy_invested_usd: strategy_invested,
        manual_position_cost_usd: manual_cost,
        open_order_reserves_usd: order_reserves,
        configured_allocation_targets_usd: equity_detail.allocated_across_strategies_usd,
        reconciled,
        discrepancy_usd: discrepancy,
        notes,
        strategies: lines,
    })
}

/// Sync invested cache from open position cost basis (repair helper).
pub async fn sync_invested_from_positions(
    tx: &mut Tx<'_>,
    strategy_id: Uuid,
) -> anyhow::Result<()> {
    let deployed: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(cost_basis_usd), 0) FROM positions WHERE strategy_id = $1 AND open_shares > 0",
    )
    .bind(strategy_id)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE strategy_capital_accounts SET invested_capital_usd = $2, updated_at = now() WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .bind(deployed)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
