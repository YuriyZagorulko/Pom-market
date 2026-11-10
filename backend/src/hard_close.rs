use chrono::Utc;
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::begin_as;
use crate::engine::{limit_price, walk_book, FeeSchedule, OrderBook, RejectReason};
use crate::pipeline::{
    cancel_open_buy_orders, cancel_open_sell_orders, paper_sell_position, reserved_sell_shares,
    reset_pending_exit_shares, sellable_shares,
};
use crate::polymarket::http::PublicClient;
use crate::trading_control::{
    next_state_after_hard_close, TradingControlState, STATE_HARD_CLOSE_COMPLETED,
    STATE_HARD_CLOSE_IN_PROGRESS, STATE_HARD_CLOSE_PARTIAL, STATE_KILL_SWITCH,
};

pub async fn preview_hard_close(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    client: &PublicClient,
) -> anyhow::Result<serde_json::Value> {
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let mode: String = sqlx::query_scalar(
        "SELECT operating_mode FROM trading_accounts WHERE id = $1 AND user_id = $2",
    )
    .bind(account_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if mode != "paper_trading" {
        anyhow::bail!("hard close is only available for paper trading accounts");
    }
    let rows = load_open_positions(&mut tx, account_id).await?;
    let mut positions = Vec::new();
    let mut est_proceeds = Decimal::ZERO;
    let mut est_sellable = Decimal::ZERO;
    for row in &rows {
        let reserved =
            reserved_sell_shares(&mut tx, account_id, &row.token_id, "paper_trading").await?;
        let sellable = sellable_shares(row.open_shares, row.pending_exit_shares, reserved);
        est_sellable += sellable;
        let (line_est, sellable_now, slip_bps) = client
            .order_book(&row.token_id)
            .await
            .ok()
            .map(|book| {
                let est = estimate_sell_proceeds(&book, sellable).ok();
                let slip = book
                    .bids
                    .first()
                    .map(|l| crate::engine::slippage_bps(l.price, l.price))
                    .unwrap_or(0);
                (est.unwrap_or(Decimal::ZERO), sellable > Decimal::ZERO, slip)
            })
            .unwrap_or((Decimal::ZERO, false, 0));
        let unsellable = if sellable <= Decimal::ZERO {
            Some("shares_reserved_by_pending_sell_or_exit_queue")
        } else if !sellable_now {
            Some("no_liquidity_or_below_minimum")
        } else {
            None
        };
        est_proceeds += line_est;
        positions.push(serde_json::json!({
            "position_id": row.id,
            "token_id": row.token_id,
            "market_title": row.market_title,
            "strategy_id": row.strategy_id,
            "source_wallet": row.source_wallet,
            "open_shares": row.open_shares,
            "pending_exit_shares": row.pending_exit_shares,
            "reserved_sell_order_shares": reserved,
            "sellable_shares": sellable,
            "estimated_proceeds_usd": line_est,
            "estimated_slippage_bps_vs_top_bid": slip_bps,
            "unsellable_reason": unsellable,
        }));
    }
    tx.commit().await?;
    Ok(serde_json::json!({
        "position_count": positions.len(),
        "estimated_total_proceeds_usd": est_proceeds,
        "estimated_sellable_shares": est_sellable,
        "positions": positions,
        "disclaimer": "Estimates use current order book depth and are not guaranteed execution prices.",
        "read_only": true,
    }))
}

fn estimate_sell_proceeds(book: &OrderBook, shares: Decimal) -> Result<Decimal, RejectReason> {
    let slip = 500i32;
    let tick = book.tick;
    let limit = limit_price(
        false,
        book.bids.first().map(|l| l.price).unwrap_or(tick),
        slip,
        tick,
    );
    let fill = walk_book(book, false, limit, shares, None)?;
    Ok(fill.notional)
}

pub async fn start_hard_close(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    idempotency_key: Option<String>,
) -> anyhow::Result<Uuid> {
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let row: (String, String) = sqlx::query_as(
        "SELECT operating_mode, trading_control_state FROM trading_accounts WHERE id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(account_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if row.0 != "paper_trading" {
        anyhow::bail!("hard close is only available for paper trading accounts");
    }
    let state = TradingControlState::parse(&row.1).unwrap_or(TradingControlState::Normal);
    if state == TradingControlState::KillSwitch {
        anyhow::bail!("deactivate kill switch before hard close");
    }
    if let Some(active) = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM hard_close_operations WHERE trading_account_id = $1 AND status IN ('queued', 'in_progress')",
    )
    .bind(account_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if let Some(ref key) = idempotency_key {
            let same: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM hard_close_operations WHERE id = $1 AND idempotency_key = $2",
            )
            .bind(active)
            .bind(key)
            .fetch_optional(&mut *tx)
            .await?;
            if same.is_some() {
                tx.commit().await?;
                return Ok(active);
            }
        }
        anyhow::bail!("a hard close operation is already active for this account");
    }
    if !crate::trading_control::can_start_hard_close(state) {
        anyhow::bail!("cannot start hard close from state {}", row.1);
    }
    if let Some(ref key) = idempotency_key {
        if let Some(existing) = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM hard_close_operations WHERE trading_account_id = $1 AND idempotency_key = $2",
        )
        .bind(account_id)
        .bind(key)
        .fetch_optional(&mut *tx)
        .await?
        {
            tx.commit().await?;
            return Ok(existing);
        }
    }
    let op_id: Uuid = sqlx::query_scalar(
        "INSERT INTO hard_close_operations (user_id, trading_account_id, idempotency_key, status)
         VALUES ($1,$2,$3,'queued') RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(idempotency_key)
    .fetch_one(&mut *tx)
    .await?;
    cancel_open_buy_orders(&mut tx, account_id, "paper_trading").await?;
    cancel_open_sell_orders(&mut tx, account_id, "paper_trading").await?;
    reset_pending_exit_shares(&mut tx, account_id, "paper_trading").await?;
    let positions = load_open_positions(&mut tx, account_id).await?;
    for row in positions {
        let sellable = sellable_shares(row.open_shares, Decimal::ZERO, Decimal::ZERO);
        sqlx::query(
            "INSERT INTO hard_close_items (operation_id, position_id, token_id, market_title, requested_shares, remaining_shares)
             VALUES ($1,$2,$3,$4,$5,$6)",
        )
        .bind(op_id)
        .bind(row.id)
        .bind(row.token_id)
        .bind(row.market_title)
        .bind(row.open_shares)
        .bind(sellable)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE trading_accounts SET trading_control_state = $3, emergency_stop = true, updated_at = now()
         WHERE id = $1 AND user_id = $2",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(STATE_HARD_CLOSE_IN_PROGRESS)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO audit_log (actor_user_id, action, target, metadata)
         VALUES ($1, 'hard_close_started', $2, $3)",
    )
    .bind(user_id)
    .bind(account_id.to_string())
    .bind(serde_json::json!({
        "operation_id": op_id,
        "previous_state": row.1,
        "new_state": STATE_HARD_CLOSE_IN_PROGRESS,
    }))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(op_id)
}

pub async fn process_pending_operations(
    pool: &PgPool,
    books: &dyn crate::pipeline::Books,
    default_fee_rate: Decimal,
    default_fee_exponent: u32,
) -> anyhow::Result<()> {
    let mut poll = begin_as(pool, None, "worker").await?;
    let op_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM hard_close_operations WHERE status IN ('queued', 'in_progress') ORDER BY created_at LIMIT 4",
    )
    .fetch_all(&mut *poll)
    .await?;
    poll.commit().await?;
    for op_id in op_ids {
        if let Err(err) =
            process_one_operation(pool, books, op_id, default_fee_rate, default_fee_exponent).await
        {
            tracing::warn!(operation_id = %op_id, error = %err, "hard close tick failed");
        }
    }
    Ok(())
}

async fn process_one_operation(
    pool: &PgPool,
    books: &dyn crate::pipeline::Books,
    op_id: Uuid,
    default_fee_rate: Decimal,
    default_fee_exponent: u32,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let op: (Uuid, Uuid, Uuid, String, String) = sqlx::query_as(
        "SELECT o.id, o.user_id, o.trading_account_id, o.status, a.trading_control_state
         FROM hard_close_operations o
         JOIN trading_accounts a ON a.id = o.trading_account_id
         WHERE o.id = $1 FOR UPDATE OF o",
    )
    .bind(op_id)
    .fetch_one(&mut *tx)
    .await?;
    if op.3 == "paused" || op.4 == STATE_KILL_SWITCH {
        tx.commit().await?;
        return Ok(());
    }
    if op.3 == "queued" {
        sqlx::query(
            "UPDATE hard_close_operations SET status = 'in_progress', started_at = now(), updated_at = now() WHERE id = $1",
        )
        .bind(op_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    reclaim_stale_in_progress_items(pool, op_id).await?;

    if let Some((item_id, position_id, token_id, remaining)) = claim_next_item(pool, op_id).await? {
        let book = books.book(&token_id).await;
        let fee_sched = books.fee(&token_id).await.or(Some(FeeSchedule {
            fees_enabled: true,
            rate: default_fee_rate,
            exponent: default_fee_exponent,
            taker_only: true,
            rebate_rate: None,
            source: "default".into(),
        }));
        let result = paper_sell_position(
            pool,
            op.1,
            op.2,
            op_id,
            item_id,
            position_id,
            &token_id,
            remaining,
            &book,
            fee_sched.as_ref(),
        )
        .await;
        if let Err(err) = result {
            let mut tx = begin_as(pool, None, "worker").await?;
            sqlx::query(
                "UPDATE hard_close_items SET status = 'failed', error = $2, updated_at = now() WHERE id = $1",
            )
            .bind(item_id)
            .bind(err.to_string())
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
        }
    }

    finalize_operation(pool, op_id).await
}

type HardCloseOperationRow = (
    Uuid,
    String,
    Option<String>,
    chrono::DateTime<Utc>,
    Option<chrono::DateTime<Utc>>,
);

#[derive(Debug)]
struct PositionLot {
    id: Uuid,
    token_id: String,
    market_title: Option<String>,
    source_wallet: String,
    strategy_id: Option<Uuid>,
    open_shares: Decimal,
    pending_exit_shares: Decimal,
}

async fn load_open_positions(
    tx: &mut crate::db::Tx<'_>,
    account_id: Uuid,
) -> anyhow::Result<Vec<PositionLot>> {
    Ok(sqlx::query_as::<_, (Uuid, String, Option<String>, String, Option<Uuid>, Decimal, Decimal)>(
        "SELECT id, token_id, market_title, source_wallet, strategy_id, open_shares, pending_exit_shares
         FROM positions
         WHERE trading_account_id = $1 AND operating_mode = 'paper_trading' AND open_shares > 0
         ORDER BY token_id, strategy_id NULLS FIRST, id",
    )
    .bind(account_id)
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(
        |(id, token_id, market_title, source_wallet, strategy_id, open_shares, pending_exit_shares)| {
            PositionLot {
                id,
                token_id,
                market_title,
                source_wallet,
                strategy_id,
                open_shares,
                pending_exit_shares,
            }
        },
    )
    .collect())
}

async fn reclaim_stale_in_progress_items(pool: &PgPool, op_id: Uuid) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query(
        "UPDATE hard_close_items SET status = 'pending', updated_at = now()
         WHERE operation_id = $1 AND status = 'in_progress' AND updated_at < now() - interval '30 seconds'",
    )
    .bind(op_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn claim_next_item(
    pool: &PgPool,
    op_id: Uuid,
) -> anyhow::Result<Option<(Uuid, Uuid, String, Decimal)>> {
    loop {
        let mut tx = begin_as(pool, None, "worker").await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
            .bind(format!("hard-close:{op_id}"))
            .execute(&mut *tx)
            .await?;
        let row: Option<(Uuid, Uuid, String, Decimal, Decimal, Decimal)> = sqlx::query_as(
            "SELECT hi.id, hi.position_id, hi.token_id, p.open_shares, p.pending_exit_shares, hi.remaining_shares
             FROM hard_close_items hi
             JOIN positions p ON p.id = hi.position_id
             WHERE hi.operation_id = $1 AND hi.status IN ('pending', 'partial')
               AND p.open_shares > 0
             ORDER BY hi.created_at
             FOR UPDATE OF hi SKIP LOCKED
             LIMIT 1",
        )
        .bind(op_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((item_id, position_id, token_id, open, pending_exit, _stored)) = row else {
            tx.commit().await?;
            return Ok(None);
        };
        let account_id: Uuid = sqlx::query_scalar(
            "SELECT trading_account_id FROM hard_close_operations WHERE id = $1",
        )
        .bind(op_id)
        .fetch_one(&mut *tx)
        .await?;
        let reserved =
            reserved_sell_shares(&mut tx, account_id, &token_id, "paper_trading").await?;
        let remaining = sellable_shares(open, pending_exit, reserved);
        if remaining <= Decimal::ZERO {
            sqlx::query(
                "UPDATE hard_close_items SET status = 'failed', error = 'no_sellable_shares', remaining_shares = 0, updated_at = now() WHERE id = $1",
            )
            .bind(item_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            continue;
        }
        sqlx::query(
            "UPDATE hard_close_items SET status = 'in_progress', remaining_shares = $2, updated_at = now() WHERE id = $1",
        )
        .bind(item_id)
        .bind(remaining)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(Some((item_id, position_id, token_id, remaining)));
    }
}

async fn finalize_operation(pool: &PgPool, op_id: Uuid) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hard_close_items WHERE operation_id = $1 AND status IN ('pending', 'partial', 'in_progress') AND remaining_shares > 0",
    )
    .bind(op_id)
    .fetch_one(&mut *tx)
    .await?;
    let failed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hard_close_items WHERE operation_id = $1 AND status = 'failed'",
    )
    .bind(op_id)
    .fetch_one(&mut *tx)
    .await?;
    let closed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hard_close_items WHERE operation_id = $1 AND status = 'closed'",
    )
    .bind(op_id)
    .fetch_one(&mut *tx)
    .await?;
    if pending > 0 {
        tx.commit().await?;
        return Ok(());
    }
    let (account_id, user_id): (Uuid, Uuid) = sqlx::query_as(
        "SELECT trading_account_id, user_id FROM hard_close_operations WHERE id = $1",
    )
    .bind(op_id)
    .fetch_one(&mut *tx)
    .await?;
    let open_left: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM positions WHERE trading_account_id = $1 AND open_shares > 0",
    )
    .bind(account_id)
    .fetch_one(&mut *tx)
    .await?;
    let all_closed = open_left == 0;
    let op_status = if all_closed && failed == 0 {
        "completed"
    } else if closed > 0 || failed > 0 {
        "partial"
    } else {
        "failed"
    };
    let acct_state = next_state_after_hard_close(all_closed, closed > 0);
    let final_state = if all_closed && failed == 0 {
        STATE_HARD_CLOSE_COMPLETED
    } else {
        STATE_HARD_CLOSE_PARTIAL
    };
    sqlx::query(
        "UPDATE hard_close_operations SET status = $2, completed_at = now(), updated_at = now() WHERE id = $1",
    )
    .bind(op_id)
    .bind(op_status)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE trading_accounts SET trading_control_state = $3, emergency_stop = true, updated_at = now()
         WHERE id = $1 AND user_id = $2",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(final_state)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO audit_log (actor_user_id, action, target, metadata)
         VALUES ($1, 'hard_close_finished', $2, $3)",
    )
    .bind(user_id)
    .bind(account_id.to_string())
    .bind(serde_json::json!({
        "operation_id": op_id,
        "status": op_status,
        "new_state": acct_state,
    }))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn pause_operations_for_kill_switch(
    pool: &PgPool,
    account_id: Uuid,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query(
        "UPDATE hard_close_operations SET status = 'paused', updated_at = now()
         WHERE trading_account_id = $1 AND status IN ('queued', 'in_progress')",
    )
    .bind(account_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn get_operation(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    op_id: Uuid,
) -> anyhow::Result<serde_json::Value> {
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let op: Option<HardCloseOperationRow> = sqlx::query_as(
        "SELECT id, status, error, created_at, completed_at FROM hard_close_operations
             WHERE id = $1 AND trading_account_id = $2 AND user_id = $3",
    )
    .bind(op_id)
    .bind(account_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(op) = op else {
        anyhow::bail!("operation not found");
    };
    let items: Vec<serde_json::Value> = sqlx::query_as::<_, (Uuid, Uuid, String, String, Decimal, Decimal, Decimal, Option<String>)>(
        "SELECT id, position_id, token_id, status, requested_shares, closed_shares, remaining_shares, error
         FROM hard_close_items WHERE operation_id = $1 ORDER BY created_at",
    )
    .bind(op_id)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|(id, pos, tok, st, req, closed, rem, err)| {
        serde_json::json!({
            "id": id,
            "position_id": pos,
            "token_id": tok,
            "status": st,
            "requested_shares": req,
            "closed_shares": closed,
            "remaining_shares": rem,
            "error": err,
        })
    })
    .collect();
    tx.commit().await?;
    Ok(serde_json::json!({
        "id": op.0,
        "status": op.1,
        "error": op.2,
        "created_at": op.3,
        "completed_at": op.4,
        "items": items,
    }))
}

pub async fn retry_operation(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    op_id: Uuid,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let state: String = sqlx::query_scalar(
        "SELECT trading_control_state FROM trading_accounts WHERE id = $1 AND user_id = $2",
    )
    .bind(account_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if state == STATE_KILL_SWITCH {
        anyhow::bail!("deactivate kill switch before retry");
    }
    sqlx::query(
        "UPDATE hard_close_items SET status = 'pending', error = NULL, updated_at = now()
         WHERE operation_id = $1 AND status IN ('failed', 'partial') AND remaining_shares > 0",
    )
    .bind(op_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE hard_close_operations SET status = 'in_progress', updated_at = now() WHERE id = $1 AND trading_account_id = $2 AND user_id = $3",
    )
    .bind(op_id)
    .bind(account_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE trading_accounts SET trading_control_state = $3, updated_at = now() WHERE id = $1 AND user_id = $2",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(STATE_HARD_CLOSE_IN_PROGRESS)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}
