use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use super::{bad, internal, require_user, ApiError, AppState};
use crate::db::begin_as;
use crate::hard_close::{
    get_operation, pause_operations_for_strategy_kill_switch, preview_hard_close,
    process_pending_operations, retry_operation, start_hard_close,
};
use crate::pipeline::cancel_open_buy_orders_for_strategy;
use crate::trading_control::{
    TradingControlState, STATE_HARD_CLOSE_IN_PROGRESS, STATE_KILL_SWITCH, STATE_NORMAL,
    STATE_SOFT_CLOSE,
};

#[derive(Deserialize)]
pub struct ConfirmBody {
    pub confirm: Option<bool>,
}

async fn load_strategy_account(
    tx: &mut crate::db::Tx<'_>,
    user_id: Uuid,
    strategy_id: Uuid,
) -> Result<(Uuid, String), ApiError> {
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT cs.trading_account_id, cs.operating_mode FROM copy_strategies cs
         WHERE cs.id = $1 AND cs.user_id = $2",
    )
    .bind(strategy_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(internal)?;
    row.ok_or_else(|| ApiError(axum::http::StatusCode::NOT_FOUND, "strategy not found".into()))
}

pub async fn get_trading_control(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let control_state: String = sqlx::query_scalar(
        "SELECT trading_control_state FROM copy_strategies WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?
    .ok_or_else(|| ApiError(axum::http::StatusCode::NOT_FOUND, "strategy not found".into()))?;
    let (account_id, operating_mode) = load_strategy_account(&mut tx, user.id, id).await?;
    let account_state: String = sqlx::query_scalar(
        "SELECT trading_control_state FROM trading_accounts WHERE id = $1",
    )
    .bind(account_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let parsed = TradingControlState::parse(&control_state).unwrap_or(TradingControlState::Normal);
    let account_parsed =
        TradingControlState::parse(&account_state).unwrap_or(TradingControlState::Normal);
    let blocks_buy = account_parsed.blocks_buy() || parsed.blocks_buy();
    let blocks_all =
        account_parsed.blocks_all_trading() || parsed.blocks_all_trading();
    let allows_copy_sell = !account_parsed.blocks_all_trading()
        && account_parsed.allows_copy_sell()
        && parsed.allows_copy_sell();
    Ok(Json(json!({
        "trading_control_state": control_state,
        "account_trading_control_state": account_state,
        "operating_mode": operating_mode,
        "trading_account_id": account_id,
        "blocks_buy": blocks_buy,
        "blocks_all_trading": blocks_all,
        "allows_copy_sell": allows_copy_sell,
        "resume_requires_confirmation": crate::trading_control::resume_requires_confirmation(parsed),
    })))
}

async fn set_strategy_state(
    state: &AppState,
    user_id: Uuid,
    strategy_id: Uuid,
    new_state: &str,
    action: &str,
    cancel_buys: bool,
) -> Result<(Uuid, String), ApiError> {
    let mut tx = begin_as(&state.pool, Some(user_id), "user")
        .await
        .map_err(internal)?;
    let prev: String = sqlx::query_scalar(
        "SELECT trading_control_state FROM copy_strategies WHERE id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(strategy_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?
    .ok_or_else(|| ApiError(axum::http::StatusCode::NOT_FOUND, "strategy not found".into()))?;
    let (account_id, operating_mode) = load_strategy_account(&mut tx, user_id, strategy_id).await?;
    if operating_mode != "paper_trading" {
        return Err(bad("strategy trading control is only supported for paper trading accounts"));
    }
    if cancel_buys {
        cancel_open_buy_orders_for_strategy(&mut tx, strategy_id, "paper_trading")
            .await
            .map_err(internal)?;
    }
    sqlx::query(
        "UPDATE copy_strategies SET trading_control_state = $3, updated_at = now() WHERE id = $1 AND user_id = $2",
    )
    .bind(strategy_id)
    .bind(user_id)
    .bind(new_state)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    sqlx::query(
        "INSERT INTO audit_log (actor_user_id, action, target, metadata) VALUES ($1,$2,$3,$4)",
    )
    .bind(user_id)
    .bind(action)
    .bind(strategy_id.to_string())
    .bind(json!({
        "previous_state": prev,
        "new_state": new_state,
        "trading_account_id": account_id,
    }))
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok((account_id, prev))
}

pub async fn activate_kill_switch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<ConfirmBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    if body.confirm != Some(true) {
        return Err(bad("confirmation required"));
    }
    pause_operations_for_strategy_kill_switch(&state.pool, id)
        .await
        .map_err(internal)?;
    set_strategy_state(
        &state,
        user.id,
        id,
        STATE_KILL_SWITCH,
        "strategy_kill_switch_on",
        true,
    )
    .await?;
    Ok(Json(json!({"trading_control_state": STATE_KILL_SWITCH})))
}

pub async fn deactivate_kill_switch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<ConfirmBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    if body.confirm != Some(true) {
        return Err(bad("confirmation required"));
    }
    set_strategy_state(
        &state,
        user.id,
        id,
        STATE_NORMAL,
        "strategy_kill_switch_off",
        false,
    )
    .await?;
    Ok(Json(json!({"trading_control_state": STATE_NORMAL})))
}

pub async fn activate_soft_close(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<ConfirmBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    if body.confirm != Some(true) {
        return Err(bad("confirmation required"));
    }
    set_strategy_state(
        &state,
        user.id,
        id,
        STATE_SOFT_CLOSE,
        "strategy_soft_close_on",
        true,
    )
    .await?;
    Ok(Json(json!({"trading_control_state": STATE_SOFT_CLOSE})))
}

pub async fn deactivate_soft_close(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<ConfirmBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    if body.confirm != Some(true) {
        return Err(bad("confirmation required"));
    }
    set_strategy_state(
        &state,
        user.id,
        id,
        STATE_NORMAL,
        "strategy_soft_close_off",
        false,
    )
    .await?;
    Ok(Json(json!({"trading_control_state": STATE_NORMAL})))
}

pub async fn resume_trading(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<ConfirmBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    if body.confirm != Some(true) {
        return Err(bad("confirmation required"));
    }
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let current: String = sqlx::query_scalar(
        "SELECT trading_control_state FROM copy_strategies WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?
    .ok_or_else(|| ApiError(axum::http::StatusCode::NOT_FOUND, "strategy not found".into()))?;
    let parsed = TradingControlState::parse(&current).unwrap_or(TradingControlState::Normal);
    if !crate::trading_control::resume_requires_confirmation(parsed) {
        return Err(bad("strategy is already in normal trading mode"));
    }
    if current == STATE_HARD_CLOSE_IN_PROGRESS {
        return Err(bad("wait for hard close to finish or activate kill switch"));
    }
    drop(tx);
    set_strategy_state(
        &state,
        user.id,
        id,
        STATE_NORMAL,
        "strategy_trading_resumed",
        false,
    )
    .await?;
    Ok(Json(json!({"trading_control_state": STATE_NORMAL})))
}

pub async fn hard_close_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let (account_id, _) = load_strategy_account(&mut tx, user.id, id).await?;
    tx.commit().await.map_err(internal)?;
    let preview = preview_hard_close(&state.pool, user.id, account_id, Some(id), &state.polymarket)
        .await
        .map_err(|e| bad(e.to_string()))?;
    Ok(Json(preview))
}

#[derive(Deserialize)]
pub struct HardCloseBody {
    pub confirmation: Option<String>,
}

pub async fn start_hard_close_route(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<HardCloseBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    if body.confirmation.as_deref() != Some("CLOSE STRATEGY") {
        return Err(bad("type CLOSE STRATEGY to confirm hard close"));
    }
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let (account_id, _) = load_strategy_account(&mut tx, user.id, id).await?;
    tx.commit().await.map_err(internal)?;
    let idempotency = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let op_id = start_hard_close(&state.pool, user.id, account_id, Some(id), idempotency)
        .await
        .map_err(|e| bad(e.to_string()))?;
    let _ = process_pending_operations(
        &state.pool,
        &super::account_control::BooksAdapter(&state.polymarket),
        &state.polymarket,
        state.cfg.fee_rate,
        state.cfg.fee_exponent,
    )
    .await;
    Ok(Json(json!({"operation_id": op_id, "strategy_id": id})))
}

pub async fn get_hard_close(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, op_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let (account_id, _) = load_strategy_account(&mut tx, user.id, id).await?;
    tx.commit().await.map_err(internal)?;
    let data = get_operation(&state.pool, user.id, account_id, Some(id), op_id)
        .await
        .map_err(|e| bad(e.to_string()))?;
    Ok(Json(data))
}

pub async fn retry_hard_close(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, op_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let (account_id, _) = load_strategy_account(&mut tx, user.id, id).await?;
    tx.commit().await.map_err(internal)?;
    retry_operation(&state.pool, user.id, account_id, Some(id), op_id)
        .await
        .map_err(|e| bad(e.to_string()))?;
    let _ = process_pending_operations(
        &state.pool,
        &super::account_control::BooksAdapter(&state.polymarket),
        &state.polymarket,
        state.cfg.fee_rate,
        state.cfg.fee_exponent,
    )
    .await;
    Ok(Json(json!({"operation_id": op_id, "retrying": true})))
}
