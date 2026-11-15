//! PAPER settlement for resolved Polymarket markets (redemption simulation).

use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::begin_as;
use crate::polymarket::http::{MarketResolutionKind, PublicClient, resolution_for_token};
use crate::strategy_accounting;

#[derive(Clone, Debug)]
pub struct SettlementResult {
    pub position_id: Uuid,
    pub status: &'static str,
    pub payout_usd: Decimal,
    pub cost_released: Decimal,
    pub pnl_usd: Decimal,
    pub detail: String,
}

/// Try to settle one open PAPER position when the CLOB market is definitively resolved.
pub async fn try_settle_position(
    pool: &PgPool,
    client: &PublicClient,
    position_id: Uuid,
) -> anyhow::Result<Option<SettlementResult>> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let row: Option<(
        Uuid,
        Uuid,
        Uuid,
        Option<Uuid>,
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        Decimal,
        Decimal,
        String,
        Decimal,
        Decimal,
        i64,
    )> = sqlx::query_as(
        "SELECT p.id, p.user_id, p.trading_account_id, p.strategy_id, p.token_id, p.operating_mode,
                p.condition_id, p.outcome, p.market_title, p.open_shares, p.cost_basis_usd, p.source_wallet,
                p.avg_price, p.realized_pnl_usd, p.close_cycle
         FROM positions p
         WHERE p.id = $1 AND p.open_shares > 0 FOR UPDATE OF p",
    )
    .bind(position_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        tx.commit().await?;
        return Ok(None);
    };
    if row.5 != "paper_trading" {
        tx.commit().await?;
        return Ok(None);
    }
    let idem = format!("paper-resolution:{}", position_id);
    if strategy_accounting::ledger_idempotency_exists(&mut tx, &idem).await? {
        tx.commit().await?;
        return Ok(None);
    }
    let condition_id = row
        .6
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("position missing condition_id"))?;
    tx.commit().await?;

    let market = client
        .market_by_condition_id(condition_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    let resolution = resolution_for_token(&market, &row.4, Some(row.7.as_str()));

    let (status, payout, entry_type, detail) = match resolution.kind {
        MarketResolutionKind::Active => {
            return Ok(Some(SettlementResult {
                position_id,
                status: "active_market",
                payout_usd: Decimal::ZERO,
                cost_released: Decimal::ZERO,
                pnl_usd: Decimal::ZERO,
                detail: "market still active".into(),
            }));
        }
        MarketResolutionKind::ClosedPending => {
            return Ok(Some(SettlementResult {
                position_id,
                status: "awaiting_resolution",
                payout_usd: Decimal::ZERO,
                cost_released: Decimal::ZERO,
                pnl_usd: Decimal::ZERO,
                detail: "market closed, winner not confirmed".into(),
            }));
        }
        MarketResolutionKind::Win => (
            "resolved_win",
            (row.9 * resolution.payout_per_share).round_dp(8),
            "resolution_redeem",
            format!(
                "redeemed winning outcome at ${}/share",
                resolution.payout_per_share
            ),
        ),
        MarketResolutionKind::Loss => (
            "resolved_loss",
            Decimal::ZERO,
            "resolution_loss",
            format!(
                "losing outcome; winner {:?}",
                resolution.winning_outcome
            ),
        ),
        MarketResolutionKind::RefundHalf => (
            "resolved_refund",
            (row.9 * resolution.payout_per_share).round_dp(8),
            "resolution_refund",
            "50-50 or refund settlement".into(),
        ),
    };

    let cost_released = row.10;
    let pnl = payout - cost_released;
    let mut tx = begin_as(pool, None, "worker").await?;
    let shares = row.9;
    sqlx::query(
        "UPDATE positions SET open_shares = 0, cost_basis_usd = 0, realized_pnl_usd = realized_pnl_usd + $2, updated_at = now() WHERE id = $1",
    )
    .bind(position_id)
    .bind(pnl)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO closed_positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, market_title, operating_mode, closed_shares, realized_pnl_usd)
         VALUES ($1,$2,$3,$4,$5,$6,'paper_trading',$7,$8)",
    )
    .bind(row.1)
    .bind(row.2)
    .bind(row.3)
    .bind(&row.11)
    .bind(&row.4)
    .bind(&row.8)
    .bind(shares)
    .bind(pnl)
    .execute(&mut *tx)
    .await?;

    if let Some(sid) = row.3 {
        let reason = match entry_type {
            "resolution_redeem" => crate::position_close::CloseReason::ResolutionWin,
            "resolution_loss" => crate::position_close::CloseReason::ResolutionLoss,
            "resolution_refund" => crate::position_close::CloseReason::ResolutionRefund,
            _ => crate::position_close::CloseReason::ResolutionWin,
        };
        let meta = crate::position_close::CloseSnapshotMeta {
            user_id: row.1,
            trading_account_id: row.2,
            strategy_id: sid,
            position_id,
            token_id: row.4.clone(),
            operating_mode: row.5.clone(),
            market_title: row.8.clone(),
            outcome: Some(row.7.clone()),
            reason,
            close_cycle: row.14,
            realized_pnl_usd: row.13 + pnl,
            fallback_closed_shares: shares,
            fallback_cost_basis: cost_released,
            fallback_avg_entry: row.12,
        };
        let _ = crate::position_close::record_close_if_full_exit(&mut tx, meta, Some(payout)).await?;
        strategy_accounting::apply_paper_resolution_settlement(
            &mut tx,
            row.1,
            row.2,
            sid,
            position_id,
            payout,
            cost_released,
            &idem,
            entry_type,
            serde_json::json!({
                "condition_id": condition_id,
                "token_id": row.4,
                "outcome": row.7,
                "shares": shares,
                "payout_usd": payout,
                "cost_released_usd": cost_released,
                "pnl_usd": pnl,
                "resolution_kind": format!("{:?}", resolution.kind),
            }),
        )
        .await?;
        strategy_accounting::sync_invested_from_positions(&mut tx, sid).await?;
    } else {
        anyhow::bail!("manual position resolution ledger not implemented");
    }
    tx.commit().await?;

    Ok(Some(SettlementResult {
        position_id,
        status,
        payout_usd: payout,
        cost_released,
        pnl_usd: pnl,
        detail,
    }))
}

pub async fn process_open_positions(
    pool: &PgPool,
    client: &PublicClient,
    limit: i64,
) -> anyhow::Result<Vec<SettlementResult>> {
    let mut poll = begin_as(pool, None, "worker").await?;
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM positions
         WHERE operating_mode = 'paper_trading' AND open_shares > 0 AND strategy_id IS NOT NULL
         ORDER BY updated_at ASC
         LIMIT $1",
    )
    .bind(limit)
    .fetch_all(&mut *poll)
    .await?;
    poll.commit().await?;
    let mut out = Vec::new();
    for id in ids {
        match try_settle_position(pool, client, id).await {
            Ok(Some(r)) if r.payout_usd > Decimal::ZERO || r.status.starts_with("resolved_") => {
                if r.status.starts_with("resolved_") {
                    tracing::info!(
                        position_id = %id,
                        status = r.status,
                        payout = %r.payout_usd,
                        "paper resolution settlement"
                    );
                }
                out.push(r);
            }
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(position_id = %id, error = %err, "paper settlement failed");
            }
        }
    }
    Ok(out)
}
