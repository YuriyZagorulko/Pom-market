//! Record immutable position-close snapshots for Activity (no ledger side effects).

use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Clone, Copy, Debug)]
pub enum CloseReason {
    LeaderSell,
    HardClose,
    ResolutionWin,
    ResolutionLoss,
    ResolutionRefund,
}

impl CloseReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LeaderSell => "leader_sell",
            Self::HardClose => "hard_close",
            Self::ResolutionWin => "resolution_win",
            Self::ResolutionLoss => "resolution_loss",
            Self::ResolutionRefund => "resolution_refund",
        }
    }
}

pub struct CloseSnapshot {
    pub user_id: Uuid,
    pub trading_account_id: Uuid,
    pub strategy_id: Uuid,
    pub position_id: Uuid,
    pub token_id: String,
    pub market_title: Option<String>,
    pub outcome: Option<String>,
    pub reason: CloseReason,
    pub close_cycle: i64,
    pub closed_shares: Decimal,
    pub avg_entry_price: Decimal,
    pub avg_exit_price: Option<Decimal>,
    pub total_cost_basis_usd: Decimal,
    pub total_proceeds_usd: Decimal,
    pub total_fees_usd: Decimal,
    pub realized_pnl_usd: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordCloseResult {
    Inserted,
    AlreadyRecorded,
}

/// Lifecycle totals for the open position cycle (since the previous close snapshot).
pub struct OpenCycleTotals {
    pub buy_shares: Decimal,
    pub buy_cost_usd: Decimal,
    pub sell_shares: Decimal,
    pub sell_proceeds_usd: Decimal,
    pub sell_fees_usd: Decimal,
    pub sell_exit_vwap: Option<Decimal>,
}

pub async fn cycle_since(
    tx: &mut Transaction<'_, Postgres>,
    position_id: Uuid,
) -> anyhow::Result<Option<chrono::DateTime<chrono::Utc>>> {
    let since: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT MAX(closed_at) FROM position_close_events WHERE position_id = $1",
    )
    .bind(position_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(since)
}

pub async fn open_cycle_totals(
    tx: &mut Transaction<'_, Postgres>,
    strategy_id: Uuid,
    token_id: &str,
    operating_mode: &str,
    position_id: Uuid,
) -> anyhow::Result<OpenCycleTotals> {
    let since = cycle_since(tx, position_id).await?;
    let buy: (Decimal, Decimal) = sqlx::query_as(
        "SELECT COALESCE(SUM(f.shares), 0),
                COALESCE(SUM(f.notional_usd + COALESCE(f.fee_usd, 0)), 0)
         FROM fills f
         JOIN orders o ON o.id = f.order_id
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1
           AND o.token_id = $2
           AND o.operating_mode = $3
           AND lower(o.side) = 'buy'
           AND ($4::timestamptz IS NULL OR f.filled_at > $4)",
    )
    .bind(strategy_id)
    .bind(token_id)
    .bind(operating_mode)
    .bind(since)
    .fetch_one(&mut **tx)
    .await?;

    let sell: (Decimal, Decimal, Decimal, Option<Decimal>) = sqlx::query_as(
        "SELECT COALESCE(SUM(f.shares), 0),
                COALESCE(SUM(f.notional_usd), 0),
                COALESCE(SUM(COALESCE(f.fee_usd, 0)), 0),
                CASE WHEN COALESCE(SUM(f.shares), 0) > 0
                     THEN SUM(f.price * f.shares) / SUM(f.shares)
                     ELSE NULL END
         FROM fills f
         JOIN orders o ON o.id = f.order_id
         WHERE o.token_id = $2
           AND o.operating_mode = $3
           AND lower(o.side) = 'sell'
           AND ($4::timestamptz IS NULL OR f.filled_at > $4)
           AND (
               o.decision_id IS NOT NULL AND EXISTS (
                   SELECT 1 FROM copy_decisions d
                   WHERE d.id = o.decision_id AND d.strategy_id = $1
               )
               OR (o.origin = 'hard_close' AND EXISTS (
                      SELECT 1 FROM hard_close_items hci
                      JOIN positions p ON p.id = hci.position_id
                      WHERE hci.order_id = o.id AND p.strategy_id = $1
                  ))
           )",
    )
    .bind(strategy_id)
    .bind(token_id)
    .bind(operating_mode)
    .bind(since)
    .fetch_one(&mut **tx)
    .await?;

    Ok(OpenCycleTotals {
        buy_shares: buy.0,
        buy_cost_usd: buy.1,
        sell_shares: sell.0,
        sell_proceeds_usd: sell.1,
        sell_fees_usd: sell.2,
        sell_exit_vwap: sell.3,
    })
}

pub fn idempotency_key(position_id: Uuid, close_cycle: i64) -> String {
    format!("close|pos:{position_id}|cycle:{close_cycle}")
}

/// Build a close snapshot from ledger/fill history (display-only; realized P&L from position row).
pub fn snapshot_from_cycle(
    meta: &CloseSnapshotMeta,
    cycle: &OpenCycleTotals,
    resolution_proceeds: Option<Decimal>,
) -> CloseSnapshot {
    let closed_shares = if cycle.buy_shares > Decimal::ZERO {
        cycle.buy_shares
    } else if cycle.sell_shares > Decimal::ZERO {
        cycle.sell_shares
    } else {
        meta.fallback_closed_shares
    };
    let total_cost = if cycle.buy_cost_usd > Decimal::ZERO {
        cycle.buy_cost_usd
    } else {
        meta.fallback_cost_basis
    };
    let avg_entry = if cycle.buy_shares > Decimal::ZERO {
        (cycle.buy_cost_usd / cycle.buy_shares).round_dp(8)
    } else {
        meta.fallback_avg_entry
    };
    let proceeds = resolution_proceeds.unwrap_or(cycle.sell_proceeds_usd);
    let exit_vwap = if cycle.sell_shares > Decimal::ZERO {
        cycle.sell_exit_vwap
    } else {
        None
    };
    CloseSnapshot {
        user_id: meta.user_id,
        trading_account_id: meta.trading_account_id,
        strategy_id: meta.strategy_id,
        position_id: meta.position_id,
        token_id: meta.token_id.clone(),
        market_title: meta.market_title.clone(),
        outcome: meta.outcome.clone(),
        reason: meta.reason,
        close_cycle: meta.close_cycle,
        closed_shares,
        avg_entry_price: avg_entry,
        avg_exit_price: exit_vwap,
        total_cost_basis_usd: total_cost,
        total_proceeds_usd: proceeds,
        total_fees_usd: cycle.sell_fees_usd,
        realized_pnl_usd: meta.realized_pnl_usd,
    }
}

#[derive(Clone)]
pub struct CloseSnapshotMeta {
    pub user_id: Uuid,
    pub trading_account_id: Uuid,
    pub strategy_id: Uuid,
    pub position_id: Uuid,
    pub token_id: String,
    pub operating_mode: String,
    pub market_title: Option<String>,
    pub outcome: Option<String>,
    pub reason: CloseReason,
    pub close_cycle: i64,
    pub realized_pnl_usd: Decimal,
    pub fallback_closed_shares: Decimal,
    pub fallback_cost_basis: Decimal,
    pub fallback_avg_entry: Decimal,
}

pub async fn record_close(
    tx: &mut Transaction<'_, Postgres>,
    snap: &CloseSnapshot,
) -> anyhow::Result<RecordCloseResult> {
    let key = idempotency_key(snap.position_id, snap.close_cycle);
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO position_close_events (
            user_id, trading_account_id, strategy_id, position_id, token_id,
            market_title, outcome, close_reason, close_cycle, closed_shares, avg_entry_price,
            avg_exit_price, total_cost_basis_usd, total_proceeds_usd, total_fees_usd,
            realized_pnl_usd, idempotency_key, closed_at
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17, now())
         ON CONFLICT (position_id, close_cycle) DO NOTHING
         RETURNING id",
    )
    .bind(snap.user_id)
    .bind(snap.trading_account_id)
    .bind(snap.strategy_id)
    .bind(snap.position_id)
    .bind(&snap.token_id)
    .bind(&snap.market_title)
    .bind(&snap.outcome)
    .bind(snap.reason.as_str())
    .bind(snap.close_cycle)
    .bind(snap.closed_shares)
    .bind(snap.avg_entry_price)
    .bind(snap.avg_exit_price)
    .bind(snap.total_cost_basis_usd)
    .bind(snap.total_proceeds_usd)
    .bind(snap.total_fees_usd)
    .bind(snap.realized_pnl_usd)
    .bind(&key)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(if inserted.is_some() {
        RecordCloseResult::Inserted
    } else {
        RecordCloseResult::AlreadyRecorded
    })
}

/// Incremental realized P&L for the active close cycle (not lifetime cumulative).
pub fn cycle_realized_pnl(cumulative: Decimal, baseline: Decimal) -> Decimal {
    (cumulative - baseline).round_dp(8)
}

/// `closed_positions.realized_pnl_usd` stores **per-cycle** realized P&L (same semantics as
/// `position_close_events`). `positions.realized_pnl_usd` is **cumulative lifetime** per token row.

/// After a full exit (`open_shares == 0`), record one CLOSED row for this position cycle.
pub async fn record_close_if_full_exit(
    tx: &mut Transaction<'_, Postgres>,
    meta: CloseSnapshotMeta,
    resolution_proceeds: Option<Decimal>,
) -> anyhow::Result<RecordCloseResult> {
    let row: (Decimal, Decimal, Decimal) = sqlx::query_as(
        "SELECT open_shares, realized_pnl_usd, close_cycle_realized_baseline
         FROM positions WHERE id = $1 FOR UPDATE",
    )
    .bind(meta.position_id)
    .fetch_one(&mut **tx)
    .await?;
    if row.0 > Decimal::ZERO {
        return Ok(RecordCloseResult::AlreadyRecorded);
    }
    let cycle = open_cycle_totals(
        tx,
        meta.strategy_id,
        &meta.token_id,
        &meta.operating_mode,
        meta.position_id,
    )
    .await?;
    let mut snap = snapshot_from_cycle(&meta, &cycle, resolution_proceeds);
    snap.realized_pnl_usd = cycle_realized_pnl(row.1, row.2);
    record_close(tx, &snap).await
}

pub async fn prior_close_realized_sum(
    tx: &mut Transaction<'_, Postgres>,
    position_id: Uuid,
    before_cycle: i64,
) -> anyhow::Result<Decimal> {
    let sum: Option<Decimal> = sqlx::query_scalar(
        "SELECT COALESCE(SUM(realized_pnl_usd), 0) FROM position_close_events
         WHERE position_id = $1 AND close_cycle < $2",
    )
    .bind(position_id)
    .bind(before_cycle)
    .fetch_one(&mut **tx)
    .await?;
    Ok(sum.unwrap_or(Decimal::ZERO))
}

pub async fn set_order_position_open_shares_after(
    tx: &mut Transaction<'_, Postgres>,
    order_id: Uuid,
    remaining: Decimal,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE orders SET position_open_shares_after = $2, updated_at = now() WHERE id = $1",
    )
    .bind(order_id)
    .bind(remaining)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Sell-fill proceeds and fees for a strategy token since the last close (or forever).
pub async fn sell_fill_totals_for_open_cycle(
    tx: &mut Transaction<'_, Postgres>,
    strategy_id: Uuid,
    token_id: &str,
    operating_mode: &str,
    position_id: Uuid,
) -> anyhow::Result<(Decimal, Decimal, Option<Decimal>)> {
    let t = open_cycle_totals(tx, strategy_id, token_id, operating_mode, position_id).await?;
    Ok((t.sell_proceeds_usd, t.sell_fees_usd, t.sell_exit_vwap))
}

#[cfg(test)]
mod tests {
    use super::{idempotency_key, CloseReason, snapshot_from_cycle, CloseSnapshotMeta, OpenCycleTotals};
    use rust_decimal::Decimal;
    use uuid::Uuid;

    #[test]
    fn close_reason_codes_stable() {
        assert_eq!(CloseReason::LeaderSell.as_str(), "leader_sell");
        assert_eq!(CloseReason::HardClose.as_str(), "hard_close");
        assert_eq!(CloseReason::ResolutionWin.as_str(), "resolution_win");
        assert_eq!(CloseReason::ResolutionLoss.as_str(), "resolution_loss");
        assert_eq!(CloseReason::ResolutionRefund.as_str(), "resolution_refund");
    }

    #[test]
    fn idempotency_key_same_for_all_triggers_on_cycle() {
        let pos = Uuid::new_v4();
        let a = idempotency_key(pos, 2);
        let b = idempotency_key(pos, 2);
        assert_eq!(a, b);
        assert!(a.contains("cycle:2"));
    }

    #[test]
    fn cycle_realized_subtracts_baseline() {
        use super::cycle_realized_pnl;
        assert_eq!(
            cycle_realized_pnl(Decimal::from(15), Decimal::from(10)),
            Decimal::from(5)
        );
        assert_eq!(
            cycle_realized_pnl(Decimal::from(3), Decimal::ZERO),
            Decimal::from(3)
        );
    }

    #[test]
    fn snapshot_uses_cycle_buys_and_sells_not_last_slice_only() {
        let pos = Uuid::new_v4();
        let meta = CloseSnapshotMeta {
            user_id: Uuid::new_v4(),
            trading_account_id: Uuid::new_v4(),
            strategy_id: Uuid::new_v4(),
            position_id: pos,
            token_id: "tok".into(),
            operating_mode: "paper_trading".into(),
            market_title: Some("M".into()),
            outcome: Some("Yes".into()),
            reason: CloseReason::LeaderSell,
            close_cycle: 1,
            realized_pnl_usd: Decimal::from(5),
            fallback_closed_shares: Decimal::from(40),
            fallback_cost_basis: Decimal::from(16),
            fallback_avg_entry: Decimal::new(4, 1),
        };
        let cycle = OpenCycleTotals {
            buy_shares: Decimal::from(100),
            buy_cost_usd: Decimal::from(50),
            sell_shares: Decimal::from(100),
            sell_proceeds_usd: Decimal::from(55),
            sell_fees_usd: Decimal::new(5, 1),
            sell_exit_vwap: Some(Decimal::new(55, 2)),
        };
        let snap = snapshot_from_cycle(&meta, &cycle, None);
        assert_eq!(snap.closed_shares, Decimal::from(100), "uses cycle buy shares");
        assert_eq!(snap.total_cost_basis_usd, Decimal::from(50));
        assert_eq!(snap.total_proceeds_usd, Decimal::from(55));
        assert_eq!(snap.realized_pnl_usd, Decimal::from(5));
        assert_eq!(snap.avg_exit_price, Some(Decimal::new(55, 2)));
    }

    #[test]
    fn resolution_proceeds_override_sell_vwap() {
        let meta = CloseSnapshotMeta {
            user_id: Uuid::new_v4(),
            trading_account_id: Uuid::new_v4(),
            strategy_id: Uuid::new_v4(),
            position_id: Uuid::new_v4(),
            token_id: "tok".into(),
            operating_mode: "paper_trading".into(),
            market_title: None,
            outcome: None,
            reason: CloseReason::ResolutionWin,
            close_cycle: 0,
            realized_pnl_usd: Decimal::ZERO,
            fallback_closed_shares: Decimal::from(10),
            fallback_cost_basis: Decimal::from(4),
            fallback_avg_entry: Decimal::new(4, 1),
        };
        let cycle = OpenCycleTotals {
            buy_shares: Decimal::from(10),
            buy_cost_usd: Decimal::from(4),
            sell_shares: Decimal::ZERO,
            sell_proceeds_usd: Decimal::ZERO,
            sell_fees_usd: Decimal::ZERO,
            sell_exit_vwap: None,
        };
        let snap = snapshot_from_cycle(&meta, &cycle, Some(Decimal::from(10)));
        assert_eq!(snap.total_proceeds_usd, Decimal::from(10));
        assert!(snap.avg_exit_price.is_none());
    }
}
