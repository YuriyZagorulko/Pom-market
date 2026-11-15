//! Safe deletion of a copy strategy and strategy-scoped data.
//!
//! Source events and shared account data are not removed. Capital is returned to the
//! account unallocated pool before the strategy row is deleted.
//!
//! Paper strategies may be deleted while disabled even with open simulated positions:
//! active paper orders are cancelled and positions are discarded at cost basis before purge.

use chrono::Utc;
use rust_decimal::Decimal;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{begin_as, Tx};
use crate::pipeline::{
    cancel_open_buy_orders_for_strategy, cancel_open_sell_orders_for_strategy,
};
use crate::strategy_accounting::{
    lock_strategy_capital, lock_strategy_capital_scoped, sync_invested_from_positions,
    try_lock_strategy_capital, withdraw_from_strategy,
};
use crate::trading_control::{
    STATE_HARD_CLOSE_COMPLETED, STATE_HARD_CLOSE_IN_PROGRESS, STATE_HARD_CLOSE_PARTIAL,
    STATE_KILL_SWITCH, STATE_NORMAL, STATE_SOFT_CLOSE,
};

const OPEN_ORDER_STATUSES: &[&str] = &[
    "pending",
    "submitting",
    "submitted",
    "partially_filled",
    "unknown",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteStrategyReject {
    NotFound,
    InProgress,
    Enabled,
    TradingControlBlocked { state: String },
    OpenPositions { count: i64 },
    ActiveOrders { count: i64 },
    HardCloseInProgress { count: i64 },
    CapitalStillLocked {
        reserved_cash_usd: Decimal,
        invested_capital_usd: Decimal,
    },
    /// All pre-delete checks that failed (shown together in the UI).
    Blocked(Vec<String>),
}

impl DeleteStrategyReject {
    pub fn user_message(&self) -> String {
        match self {
            Self::Blocked(messages) => messages.join("\n"),
            Self::NotFound => "strategy not found".into(),
            Self::InProgress => {
                "strategy delete or capital update already in progress; try again shortly".into()
            }
            Self::Enabled => {
                "disable the strategy before deleting it".into()
            }
            Self::TradingControlBlocked { state } => {
                if state == STATE_KILL_SWITCH {
                    "strategy kill switch is on — open Trading control on this page and choose \
                     \"Deactivate kill switch\", or disable the strategy and delete again (paper \
                     strategies clear kill switch automatically on delete)."
                        .into()
                } else if state == STATE_SOFT_CLOSE {
                    "strategy soft close is on — disable soft close under Trading control, or disable \
                     the paper strategy and delete (soft close clears automatically on delete)."
                        .into()
                } else {
                    format!(
                        "finish or reset trading control (current state: {state}) before deleting the strategy"
                    )
                }
            }
            Self::OpenPositions { count } => format!(
                "close all open positions before deleting ({count} open position(s) remain). \
                 Use hard close or wait until positions are flat."
            ),
            Self::ActiveOrders { count } => format!(
                "wait for or cancel active orders before deleting ({count} open order(s) remain)"
            ),
            Self::HardCloseInProgress { count } => format!(
                "a hard close is still in progress ({count} operation(s)); wait until it finishes or fails"
            ),
            Self::CapitalStillLocked {
                reserved_cash_usd,
                invested_capital_usd,
            } => format!(
                "strategy capital is still deployed or reserved \
                 (reserved ${reserved_cash_usd}, invested ${invested_capital_usd}); \
                 close positions and release orders first"
            ),
        }
    }
}

#[derive(Serialize)]
pub struct StrategyDeletePreview {
    pub can_delete: bool,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    pub paper_trading: bool,
}

#[derive(sqlx::FromRow)]
struct StrategyDeleteRow {
    trading_account_id: Uuid,
    enabled: bool,
    trading_control_state: String,
    operating_mode: String,
}

/// Read-only delete eligibility for the user UI (no locks).
pub async fn delete_preview_for_user(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
) -> Result<StrategyDeletePreview, DeleteStrategyReject> {
    let mut tx = begin_as(pool, Some(user_id), "user")
        .await
        .map_err(|_| DeleteStrategyReject::NotFound)?;
    let row: Option<StrategyDeleteRow> = sqlx::query_as(
        "SELECT s.trading_account_id, s.enabled, s.trading_control_state, s.operating_mode
         FROM copy_strategies s
         WHERE s.id = $1 AND s.user_id = $2",
    )
    .bind(strategy_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| DeleteStrategyReject::NotFound)?;
    let Some(row) = row else {
        return Err(DeleteStrategyReject::NotFound);
    };
    let paper = row.operating_mode == "paper_trading";
    let (blockers, warnings) =
        classify_delete_blockers(&mut tx, &row, strategy_id, paper).await?;
    tx.commit().await.ok();
    Ok(StrategyDeletePreview {
        can_delete: blockers.is_empty(),
        blockers,
        warnings,
        paper_trading: paper,
    })
}

/// Delete a strategy owned by `user_id` inside an open transaction.
pub async fn delete_strategy_for_user(
    tx: &mut Tx<'_>,
    user_id: Uuid,
    strategy_id: Uuid,
) -> Result<(), DeleteStrategyReject> {
    let acquired: bool = sqlx::query_scalar(
        "SELECT pg_try_advisory_xact_lock(hashtext('strategy-delete:' || $1::text))",
    )
    .bind(strategy_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| DeleteStrategyReject::NotFound)?;
    if !acquired {
        return Err(DeleteStrategyReject::InProgress);
    }

    let row: Option<StrategyDeleteRow> = sqlx::query_as(
        "SELECT s.trading_account_id, s.enabled, s.trading_control_state, s.operating_mode
         FROM copy_strategies s
         WHERE s.id = $1 AND s.user_id = $2
         FOR UPDATE OF s",
    )
    .bind(strategy_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| DeleteStrategyReject::NotFound)?;

    let Some(mut row) = row else {
        return Err(DeleteStrategyReject::NotFound);
    };

    let paper = row.operating_mode == "paper_trading";
    if paper && !row.enabled {
        paper_clear_control_for_delete(tx, strategy_id, user_id, &mut row).await?;
        paper_discard_strategy_trading_state(tx, strategy_id).await?;
    }

    let (blockers, _) = classify_delete_blockers(tx, &row, strategy_id, paper).await?;
    if !blockers.is_empty() {
        return Err(DeleteStrategyReject::Blocked(blockers));
    }

    return_capital_to_account(tx, user_id, row.trading_account_id, strategy_id, paper).await?;

    purge_strategy_scoped_rows(tx, strategy_id)
        .await
        .map_err(|_| DeleteStrategyReject::NotFound)?;

    let deleted = sqlx::query("DELETE FROM copy_strategies WHERE id = $1 AND user_id = $2")
        .bind(strategy_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(|_| DeleteStrategyReject::NotFound)?;
    if deleted.rows_affected() == 0 {
        return Err(DeleteStrategyReject::NotFound);
    }

    Ok(())
}

async fn classify_delete_blockers(
    tx: &mut Tx<'_>,
    row: &StrategyDeleteRow,
    strategy_id: Uuid,
    paper_trading: bool,
) -> Result<(Vec<String>, Vec<String>), DeleteStrategyReject> {
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();

    if row.enabled {
        blockers.push(DeleteStrategyReject::Enabled.user_message());
    }

    let control = row.trading_control_state.as_str();
    if control == STATE_HARD_CLOSE_IN_PROGRESS {
        blockers.push(
            DeleteStrategyReject::TradingControlBlocked {
                state: row.trading_control_state.clone(),
            }
            .user_message(),
        );
    } else if matches!(control, STATE_KILL_SWITCH | STATE_SOFT_CLOSE) {
        let msg = DeleteStrategyReject::TradingControlBlocked {
            state: row.trading_control_state.clone(),
        }
        .user_message();
        if paper_trading && !row.enabled {
            warnings.push(format!(
                "Trading control is \"{control}\" — it will be cleared automatically when you delete this disabled paper strategy."
            ));
        } else {
            blockers.push(msg);
        }
    } else if matches!(control, STATE_HARD_CLOSE_PARTIAL | STATE_HARD_CLOSE_COMPLETED) && !paper_trading
    {
        blockers.push(
            DeleteStrategyReject::TradingControlBlocked {
                state: row.trading_control_state.clone(),
            }
            .user_message(),
        );
    }

    let hard_close_ops: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hard_close_operations
         WHERE strategy_id = $1 AND status IN ('queued', 'in_progress')",
    )
    .bind(strategy_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| DeleteStrategyReject::NotFound)?;
    if hard_close_ops > 0 {
        blockers.push(
            DeleteStrategyReject::HardCloseInProgress {
                count: hard_close_ops,
            }
            .user_message(),
        );
    }

    let open_positions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM positions
         WHERE strategy_id = $1 AND open_shares > 0",
    )
    .bind(strategy_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| DeleteStrategyReject::NotFound)?;

    let active_orders: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM orders o
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND o.status = ANY($2::text[])",
    )
    .bind(strategy_id)
    .bind(OPEN_ORDER_STATUSES)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| DeleteStrategyReject::NotFound)?;

    let (reserved, invested) = if let Ok(Some(cap)) = try_lock_strategy_capital(tx, strategy_id).await {
        (cap.reserved_cash_usd, cap.invested_capital_usd)
    } else {
        (Decimal::ZERO, Decimal::ZERO)
    };

    let market_state_msgs = |open_positions: i64, active_orders: i64, reserved: Decimal, invested: Decimal| {
        let mut msgs = Vec::new();
        if open_positions > 0 {
            msgs.push(if paper_trading {
                format!(
                    "{open_positions} open paper position(s) will be discarded (simulated shares only)."
                )
            } else {
                DeleteStrategyReject::OpenPositions { count: open_positions }.user_message()
            });
        }
        if active_orders > 0 {
            msgs.push(if paper_trading {
                format!(
                    "{active_orders} active paper order(s) will be cancelled automatically."
                )
            } else {
                DeleteStrategyReject::ActiveOrders { count: active_orders }.user_message()
            });
        }
        msgs
    };

    let msgs = market_state_msgs(open_positions, active_orders, reserved, invested);
    if paper_trading && !row.enabled {
        warnings.extend(msgs);
    } else {
        blockers.extend(msgs);
    }

    Ok((blockers, warnings))
}

/// Paper-only: reset per-strategy danger-zone state so delete does not require manual resume.
async fn paper_clear_control_for_delete(
    tx: &mut Tx<'_>,
    strategy_id: Uuid,
    user_id: Uuid,
    row: &mut StrategyDeleteRow,
) -> Result<(), DeleteStrategyReject> {
    if row.trading_control_state == STATE_NORMAL {
        return Ok(());
    }
    if row.trading_control_state == STATE_HARD_CLOSE_IN_PROGRESS {
        return Ok(());
    }
    sqlx::query(
        "UPDATE copy_strategies
         SET trading_control_state = $3, updated_at = now()
         WHERE id = $1 AND user_id = $2",
    )
    .bind(strategy_id)
    .bind(user_id)
    .bind(STATE_NORMAL)
    .execute(&mut **tx)
    .await
    .map_err(|_| DeleteStrategyReject::NotFound)?;
    row.trading_control_state = STATE_NORMAL.to_string();
    Ok(())
}

async fn paper_discard_strategy_trading_state(
    tx: &mut Tx<'_>,
    strategy_id: Uuid,
) -> Result<(), DeleteStrategyReject> {
    cancel_open_buy_orders_for_strategy(tx, strategy_id, "paper_trading")
        .await
        .map_err(|_| DeleteStrategyReject::NotFound)?;
    cancel_open_sell_orders_for_strategy(tx, strategy_id, "paper_trading")
        .await
        .map_err(|_| DeleteStrategyReject::NotFound)?;

    let cost_deployed: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(cost_basis_usd), 0) FROM positions
         WHERE strategy_id = $1 AND open_shares > 0",
    )
    .bind(strategy_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| DeleteStrategyReject::NotFound)?;

    if cost_deployed > Decimal::ZERO {
        let _ = lock_strategy_capital(tx, strategy_id)
            .await
            .map_err(|_| DeleteStrategyReject::NotFound)?;
        sqlx::query(
            "UPDATE strategy_capital_accounts
             SET invested_capital_usd = GREATEST(invested_capital_usd - $2, 0),
                 available_cash_usd = available_cash_usd + $2,
                 updated_at = now()
             WHERE strategy_id = $1",
        )
        .bind(strategy_id)
        .bind(cost_deployed)
        .execute(&mut **tx)
        .await
        .map_err(|_| DeleteStrategyReject::NotFound)?;
    }

    sqlx::query(
        "UPDATE positions
         SET open_shares = 0, cost_basis_usd = 0, pending_exit_shares = 0, updated_at = now()
         WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .execute(&mut **tx)
    .await
    .map_err(|_| DeleteStrategyReject::NotFound)?;

    sync_invested_from_positions(tx, strategy_id)
        .await
        .map_err(|_| DeleteStrategyReject::NotFound)?;

    Ok(())
}

/// Merge reserved/invested into available and align `funded_capital_usd` before withdrawal.
async fn consolidate_strategy_liquid_for_delete(
    tx: &mut Tx<'_>,
    strategy_id: Uuid,
    paper_trading: bool,
) -> Result<(), DeleteStrategyReject> {
    let _ = lock_strategy_capital_scoped(tx, strategy_id)
        .await
        .map_err(|_| DeleteStrategyReject::NotFound)?;

    let reserved_on_open_orders: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(o.position_reserved_usd + o.fee_reserved_usd), 0)
         FROM orders o
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND o.status = ANY($2::text[])",
    )
    .bind(strategy_id)
    .bind(OPEN_ORDER_STATUSES)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| DeleteStrategyReject::NotFound)?;

    if reserved_on_open_orders > Decimal::ZERO && !paper_trading {
        let (reserved, invested): (Decimal, Decimal) = sqlx::query_as(
            "SELECT reserved_cash_usd, invested_capital_usd FROM strategy_capital_accounts WHERE strategy_id = $1",
        )
        .bind(strategy_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| DeleteStrategyReject::NotFound)?;
        return Err(DeleteStrategyReject::CapitalStillLocked {
            reserved_cash_usd: reserved,
            invested_capital_usd: invested,
        });
    }

    sqlx::query(
        "UPDATE strategy_capital_accounts
         SET available_cash_usd = available_cash_usd + reserved_cash_usd + invested_capital_usd,
             funded_capital_usd = available_cash_usd + reserved_cash_usd + invested_capital_usd,
             reserved_cash_usd = 0,
             invested_capital_usd = 0,
             updated_at = now()
         WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .execute(&mut **tx)
    .await
    .map_err(|_| DeleteStrategyReject::NotFound)?;

    Ok(())
}

async fn return_capital_to_account(
    tx: &mut Tx<'_>,
    user_id: Uuid,
    trading_account_id: Uuid,
    strategy_id: Uuid,
    paper_trading: bool,
) -> Result<(), DeleteStrategyReject> {
    if paper_trading {
        consolidate_strategy_liquid_for_delete(tx, strategy_id, paper_trading).await?;
    } else {
        let reserved_on_open_orders: Decimal = sqlx::query_scalar(
            "SELECT COALESCE(SUM(o.position_reserved_usd + o.fee_reserved_usd), 0)
             FROM orders o
             JOIN copy_decisions d ON d.id = o.decision_id
             WHERE d.strategy_id = $1 AND o.status = ANY($2::text[])",
        )
        .bind(strategy_id)
        .bind(OPEN_ORDER_STATUSES)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| DeleteStrategyReject::NotFound)?;
        let cap = lock_strategy_capital_scoped(tx, strategy_id)
            .await
            .map_err(|_| DeleteStrategyReject::NotFound)?;
        if cap.reserved_cash_usd > reserved_on_open_orders
            || cap.invested_capital_usd > Decimal::ZERO
        {
            return Err(DeleteStrategyReject::CapitalStillLocked {
                reserved_cash_usd: cap.reserved_cash_usd,
                invested_capital_usd: cap.invested_capital_usd,
            });
        }
    }

    for attempt in 0..4 {
        let cap = match lock_strategy_capital_scoped(tx, strategy_id).await {
            Ok(c) => c,
            Err(crate::strategy_accounting::StrategyCapitalError::CapitalAccountMissing) => {
                return Ok(());
            }
            Err(_) => return Err(DeleteStrategyReject::NotFound),
        };

        if cap.reserved_cash_usd > Decimal::ZERO || cap.invested_capital_usd > Decimal::ZERO {
            if paper_trading {
                consolidate_strategy_liquid_for_delete(tx, strategy_id, paper_trading).await?;
                continue;
            }
            return Err(DeleteStrategyReject::CapitalStillLocked {
                reserved_cash_usd: cap.reserved_cash_usd,
                invested_capital_usd: cap.invested_capital_usd,
            });
        }

        if cap.available_cash_usd > Decimal::ZERO {
            let key = format!(
                "delete-strategy:{strategy_id}:withdraw:{attempt}:{}",
                Utc::now().timestamp_millis()
            );
            withdraw_from_strategy(
                tx,
                user_id,
                trading_account_id,
                strategy_id,
                cap.available_cash_usd,
                &key,
            )
            .await
            .map_err(|msg| DeleteStrategyReject::Blocked(vec![msg]))?;
            continue;
        }

        return Ok(());
    }

    let cap = lock_strategy_capital_scoped(tx, strategy_id)
        .await
        .map_err(|_| DeleteStrategyReject::NotFound)?;
    if cap.reserved_cash_usd > Decimal::ZERO || cap.invested_capital_usd > Decimal::ZERO {
        return Err(DeleteStrategyReject::CapitalStillLocked {
            reserved_cash_usd: cap.reserved_cash_usd,
            invested_capital_usd: cap.invested_capital_usd,
        });
    }

    Ok(())
}

async fn purge_strategy_scoped_rows(
    tx: &mut Tx<'_>,
    strategy_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM capital_ledger_entries WHERE strategy_id = $1")
        .bind(strategy_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM closed_positions WHERE strategy_id = $1")
        .bind(strategy_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
