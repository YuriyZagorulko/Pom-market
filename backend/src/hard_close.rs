use chrono::Utc;
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::begin_as;
use crate::engine::{limit_price, walk_book, FeeSchedule, OrderBook, RejectReason};
use crate::pipeline::{
    cancel_open_buy_orders, cancel_open_buy_orders_for_strategy,
    cancel_open_sell_orders, cancel_open_sell_orders_for_strategy, paper_sell_position,
    reserved_sell_shares, reset_pending_exit_shares, sellable_shares,
};

/// Cap per worker/API tick so one HTTP hard-close start can drain a full strategy book.
const HARD_CLOSE_ITEMS_PER_TICK: u32 = 128;
use crate::paper_settlement;
use crate::polymarket::http::PublicClient;
use crate::trading_control::{
    next_state_after_hard_close, TradingControlState, STATE_HARD_CLOSE_COMPLETED,
    STATE_HARD_CLOSE_IN_PROGRESS, STATE_HARD_CLOSE_PARTIAL, STATE_KILL_SWITCH,
};

pub async fn preview_hard_close(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    strategy_id: Option<Uuid>,
    client: &PublicClient,
) -> anyhow::Result<serde_json::Value> {
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    if let Some(sid) = strategy_id {
        let strat_mode: String = sqlx::query_scalar(
            "SELECT operating_mode FROM copy_strategies
             WHERE id = $1 AND user_id = $2 AND trading_account_id = $3",
        )
        .bind(sid)
        .bind(user_id)
        .bind(account_id)
        .fetch_one(&mut *tx)
        .await?;
        if strat_mode != "paper_trading" {
            anyhow::bail!("hard close is only available for paper trading strategies");
        }
    } else {
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
    }
    if let Some(sid) = strategy_id {
        let owned: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM copy_strategies WHERE id = $1 AND user_id = $2 AND trading_account_id = $3)",
        )
        .bind(sid)
        .bind(user_id)
        .bind(account_id)
        .fetch_one(&mut *tx)
        .await?;
        if !owned {
            anyhow::bail!("strategy not found");
        }
    }
    let rows = load_open_positions(&mut tx, account_id, strategy_id).await?;
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
        "strategy_id": strategy_id,
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
    let fill = walk_book(book, false, limit, shares, None, true)?;
    Ok(fill.notional)
}

pub async fn start_hard_close(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    strategy_id: Option<Uuid>,
    idempotency_key: Option<String>,
) -> anyhow::Result<Uuid> {
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let account_row: (String, String) = sqlx::query_as(
        "SELECT operating_mode, trading_control_state FROM trading_accounts WHERE id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(account_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if strategy_id.is_none() && account_row.0 != "paper_trading" {
        anyhow::bail!("hard close is only available for paper trading accounts");
    }
    let account_state =
        TradingControlState::parse(&account_row.1).unwrap_or(TradingControlState::Normal);
    if account_state == TradingControlState::KillSwitch {
        anyhow::bail!("deactivate kill switch before hard close");
    }

    let (previous_state, audit_target, audit_action) = if let Some(sid) = strategy_id {
        let strat: (String, String) = sqlx::query_as(
            "SELECT trading_control_state, operating_mode FROM copy_strategies
             WHERE id = $1 AND user_id = $2 AND trading_account_id = $3 FOR UPDATE",
        )
        .bind(sid)
        .bind(user_id)
        .bind(account_id)
        .fetch_one(&mut *tx)
        .await?;
        if strat.1 != "paper_trading" {
            anyhow::bail!("hard close is only available for paper trading strategies");
        }
        let strategy_state =
            TradingControlState::parse(&strat.0).unwrap_or(TradingControlState::Normal);
        if account_state == TradingControlState::HardCloseInProgress {
            anyhow::bail!("account hard close is already in progress");
        }
        if strategy_state == TradingControlState::KillSwitch {
            anyhow::bail!("deactivate strategy kill switch before hard close");
        }
        if !crate::trading_control::can_start_hard_close(strategy_state) {
            anyhow::bail!("cannot start hard close from state {}", strat.0);
        }
        if let Some(active) = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM hard_close_operations WHERE strategy_id = $1 AND status IN ('queued', 'in_progress')",
        )
        .bind(sid)
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
            anyhow::bail!("a hard close operation is already active for this strategy");
        }
        if let Some(ref key) = idempotency_key {
            if let Some(existing) = sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM hard_close_operations WHERE strategy_id = $1 AND idempotency_key = $2",
            )
            .bind(sid)
            .bind(key)
            .fetch_optional(&mut *tx)
            .await?
            {
                tx.commit().await?;
                return Ok(existing);
            }
        }
        (
            strat.0.clone(),
            sid.to_string(),
            "strategy_hard_close_started",
        )
    } else {
        if let Some(active) = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM hard_close_operations
             WHERE trading_account_id = $1 AND strategy_id IS NULL AND status IN ('queued', 'in_progress')",
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
        if !crate::trading_control::can_start_hard_close(account_state) {
            anyhow::bail!("cannot start hard close from state {}", account_row.1);
        }
        if let Some(ref key) = idempotency_key {
            if let Some(existing) = sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM hard_close_operations
                 WHERE trading_account_id = $1 AND strategy_id IS NULL AND idempotency_key = $2",
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
        (
            account_row.1.clone(),
            account_id.to_string(),
            "hard_close_started",
        )
    };

    let op_id: Uuid = sqlx::query_scalar(
        "INSERT INTO hard_close_operations (user_id, trading_account_id, strategy_id, idempotency_key, status)
         VALUES ($1,$2,$3,$4,'queued') RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(strategy_id)
    .bind(idempotency_key)
    .fetch_one(&mut *tx)
    .await?;

    if let Some(sid) = strategy_id {
        cancel_open_buy_orders_for_strategy(&mut tx, sid, "paper_trading").await?;
        cancel_open_sell_orders_for_strategy(&mut tx, sid, "paper_trading").await?;
        sqlx::query(
            "UPDATE positions SET pending_exit_shares = 0, updated_at = now()
             WHERE strategy_id = $1 AND operating_mode = 'paper_trading'",
        )
        .bind(sid)
        .execute(&mut *tx)
        .await?;
    } else {
        cancel_open_buy_orders(&mut tx, account_id, "paper_trading").await?;
        cancel_open_sell_orders(&mut tx, account_id, "paper_trading").await?;
        reset_pending_exit_shares(&mut tx, account_id, "paper_trading").await?;
    }

    let positions = load_open_positions(&mut tx, account_id, strategy_id).await?;
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

    if let Some(sid) = strategy_id {
        sqlx::query(
            "UPDATE copy_strategies SET trading_control_state = $2, updated_at = now() WHERE id = $1",
        )
        .bind(sid)
        .bind(STATE_HARD_CLOSE_IN_PROGRESS)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query(
            "UPDATE trading_accounts SET trading_control_state = $3, emergency_stop = true, updated_at = now()
             WHERE id = $1 AND user_id = $2",
        )
        .bind(account_id)
        .bind(user_id)
        .bind(STATE_HARD_CLOSE_IN_PROGRESS)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "INSERT INTO audit_log (actor_user_id, action, target, metadata)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(audit_action)
    .bind(audit_target)
    .bind(serde_json::json!({
        "operation_id": op_id,
        "trading_account_id": account_id,
        "strategy_id": strategy_id,
        "previous_state": previous_state,
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
    client: &PublicClient,
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
            process_one_operation(pool, books, client, op_id, default_fee_rate, default_fee_exponent).await
        {
            tracing::warn!(operation_id = %op_id, error = %err, "hard close tick failed");
        }
    }
    Ok(())
}

async fn process_one_operation(
    pool: &PgPool,
    books: &dyn crate::pipeline::Books,
    client: &PublicClient,
    op_id: Uuid,
    default_fee_rate: Decimal,
    default_fee_exponent: u32,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let op: (Uuid, Uuid, Uuid, Option<Uuid>, String, String, Option<String>) = sqlx::query_as(
        "SELECT o.id, o.user_id, o.trading_account_id, o.strategy_id, o.status, a.trading_control_state,
                s.trading_control_state
         FROM hard_close_operations o
         JOIN trading_accounts a ON a.id = o.trading_account_id
         LEFT JOIN copy_strategies s ON s.id = o.strategy_id
         WHERE o.id = $1 FOR UPDATE OF o",
    )
    .bind(op_id)
    .fetch_one(&mut *tx)
    .await?;
    let strategy_kill = op.6.as_deref() == Some(STATE_KILL_SWITCH);
    if op.4 == "paused" || op.5 == STATE_KILL_SWITCH || strategy_kill {
        tx.commit().await?;
        return Ok(());
    }
    if op.4 == "queued" {
        sqlx::query(
            "UPDATE hard_close_operations SET status = 'in_progress', started_at = now(), updated_at = now() WHERE id = $1",
        )
        .bind(op_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    let mut deferred_partial_items = std::collections::HashSet::<Uuid>::new();
    for _ in 0..HARD_CLOSE_ITEMS_PER_TICK {
        reclaim_stale_in_progress_items(pool, op_id).await?;

        let Some((item_id, position_id, token_id, remaining)) =
            claim_next_item(pool, op_id, &deferred_partial_items).await?
        else {
            break;
        };
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
        match result {
            Ok(()) => {
                let mut status_tx = begin_as(pool, None, "worker").await?;
                let item_status: String = sqlx::query_scalar(
                    "SELECT status FROM hard_close_items WHERE id = $1",
                )
                .bind(item_id)
                .fetch_one(&mut *status_tx)
                .await?;
                status_tx.commit().await?;
                if item_status == "partial" {
                    deferred_partial_items.insert(item_id);
                }
            }
            Err(err) => {
                let err_s = err.to_string();
                if err_s.contains("404") {
                    if let Ok(Some(settled)) =
                        paper_settlement::try_settle_position(pool, client, position_id).await
                    {
                        if settled.status.starts_with("resolved_") {
                            let mut tx = begin_as(pool, None, "worker").await?;
                            sqlx::query(
                                "UPDATE hard_close_items SET status = 'closed', closed_shares = requested_shares,
                                    remaining_shares = 0, error = NULL, updated_at = now()
                                 WHERE id = $1",
                            )
                            .bind(item_id)
                            .execute(&mut *tx)
                            .await?;
                            tx.commit().await?;
                            continue;
                        }
                    }
                }
                let mut tx = begin_as(pool, None, "worker").await?;
                sqlx::query(
                    "UPDATE hard_close_items SET status = 'failed', error = $2, updated_at = now() WHERE id = $1",
                )
                .bind(item_id)
                .bind(err_s)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
            }
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
    strategy_id: Option<Uuid>,
) -> anyhow::Result<Vec<PositionLot>> {
    let sql = if strategy_id.is_some() {
        "SELECT id, token_id, market_title, source_wallet, strategy_id, open_shares, pending_exit_shares
         FROM positions
         WHERE trading_account_id = $1 AND operating_mode = 'paper_trading' AND open_shares > 0
           AND strategy_id = $2
         ORDER BY token_id, id"
    } else {
        "SELECT id, token_id, market_title, source_wallet, strategy_id, open_shares, pending_exit_shares
         FROM positions
         WHERE trading_account_id = $1 AND operating_mode = 'paper_trading' AND open_shares > 0
         ORDER BY token_id, strategy_id NULLS FIRST, id"
    };
    let rows = if let Some(sid) = strategy_id {
        sqlx::query_as::<_, (Uuid, String, Option<String>, String, Option<Uuid>, Decimal, Decimal)>(
            sql,
        )
        .bind(account_id)
        .bind(sid)
        .fetch_all(&mut **tx)
        .await?
    } else {
        sqlx::query_as::<_, (Uuid, String, Option<String>, String, Option<Uuid>, Decimal, Decimal)>(
            sql,
        )
        .bind(account_id)
        .fetch_all(&mut **tx)
        .await?
    };
    Ok(rows
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
    exclude_item_ids: &std::collections::HashSet<Uuid>,
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
               AND NOT (hi.id = ANY($2::uuid[]))
             ORDER BY hi.created_at
             FOR UPDATE OF hi SKIP LOCKED
             LIMIT 1",
        )
        .bind(op_id)
        .bind(
            exclude_item_ids
                .iter()
                .copied()
                .collect::<Vec<Uuid>>(),
        )
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
    let (account_id, user_id, strategy_id): (Uuid, Uuid, Option<Uuid>) = sqlx::query_as(
        "SELECT trading_account_id, user_id, strategy_id FROM hard_close_operations WHERE id = $1",
    )
    .bind(op_id)
    .fetch_one(&mut *tx)
    .await?;
    let open_left: i64 = if let Some(sid) = strategy_id {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM positions WHERE strategy_id = $1 AND open_shares > 0",
        )
        .bind(sid)
        .fetch_one(&mut *tx)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM positions WHERE trading_account_id = $1 AND open_shares > 0",
        )
        .bind(account_id)
        .fetch_one(&mut *tx)
        .await?
    };
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
    if let Some(sid) = strategy_id {
        sqlx::query(
            "UPDATE copy_strategies SET trading_control_state = $2, updated_at = now() WHERE id = $1",
        )
        .bind(sid)
        .bind(final_state)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query(
            "UPDATE trading_accounts SET trading_control_state = $3, emergency_stop = true, updated_at = now()
             WHERE id = $1 AND user_id = $2",
        )
        .bind(account_id)
        .bind(user_id)
        .bind(final_state)
        .execute(&mut *tx)
        .await?;
    }
  let audit_action = if strategy_id.is_some() {
        "strategy_hard_close_finished"
    } else {
        "hard_close_finished"
    };
    let audit_target = strategy_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| account_id.to_string());
    sqlx::query(
        "INSERT INTO audit_log (actor_user_id, action, target, metadata)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(audit_action)
    .bind(audit_target)
    .bind(serde_json::json!({
        "operation_id": op_id,
        "trading_account_id": account_id,
        "strategy_id": strategy_id,
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

pub async fn pause_operations_for_strategy_kill_switch(
    pool: &PgPool,
    strategy_id: Uuid,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query(
        "UPDATE hard_close_operations SET status = 'paused', updated_at = now()
         WHERE strategy_id = $1 AND status IN ('queued', 'in_progress')",
    )
    .bind(strategy_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn get_operation(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    strategy_id: Option<Uuid>,
    op_id: Uuid,
) -> anyhow::Result<serde_json::Value> {
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let op: Option<HardCloseOperationRow> = sqlx::query_as(
        "SELECT id, status, error, created_at, completed_at FROM hard_close_operations
             WHERE id = $1 AND trading_account_id = $2 AND user_id = $3
               AND ($4::uuid IS NULL OR strategy_id = $4)",
    )
    .bind(op_id)
    .bind(account_id)
    .bind(user_id)
    .bind(strategy_id)
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
        "strategy_id": strategy_id,
        "items": items,
    }))
}

pub async fn retry_operation(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    strategy_id: Option<Uuid>,
    op_id: Uuid,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let acct_state: String = sqlx::query_scalar(
        "SELECT COALESCE(trading_control_state, 'active') FROM trading_accounts WHERE id = $1 AND user_id = $2",
    )
    .bind(account_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if acct_state == STATE_KILL_SWITCH {
        anyhow::bail!("deactivate kill switch before retry");
    }
    let op_strategy: Option<Uuid> = sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT strategy_id FROM hard_close_operations WHERE id = $1 AND trading_account_id = $2 AND user_id = $3",
    )
    .bind(op_id)
    .bind(account_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if op_strategy != strategy_id {
        anyhow::bail!("operation not found");
    }
    if let Some(sid) = op_strategy {
        let st: String = sqlx::query_scalar(
            "SELECT COALESCE(trading_control_state, 'active') FROM copy_strategies WHERE id = $1 AND user_id = $2",
        )
        .bind(sid)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        if st == STATE_KILL_SWITCH {
            anyhow::bail!("deactivate strategy kill switch before retry");
        }
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
    if let Some(sid) = op_strategy {
        sqlx::query(
            "UPDATE copy_strategies SET trading_control_state = $2, updated_at = now() WHERE id = $1",
        )
        .bind(sid)
        .bind(STATE_HARD_CLOSE_IN_PROGRESS)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query(
            "UPDATE trading_accounts SET trading_control_state = $3, updated_at = now() WHERE id = $1 AND user_id = $2",
        )
        .bind(account_id)
        .bind(user_id)
        .bind(STATE_HARD_CLOSE_IN_PROGRESS)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}
