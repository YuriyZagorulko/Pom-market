use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use super::{bad, internal, load_account, require_user, ApiError, AppState};
use crate::db::begin_as;
use crate::hard_close::{
    get_operation, pause_operations_for_kill_switch, preview_hard_close,
    process_pending_operations, retry_operation, start_hard_close,
};
use crate::pipeline::cancel_open_buy_orders;
use crate::trading_control::{
    TradingControlState, STATE_HARD_CLOSE_IN_PROGRESS, STATE_KILL_SWITCH, STATE_NORMAL,
    STATE_SOFT_CLOSE,
};

#[derive(Deserialize)]
pub struct ConfirmBody {
    pub confirm: Option<bool>,
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
    let account = load_account(&mut tx, user.id, id).await?;
    tx.commit().await.map_err(internal)?;
    let parsed = TradingControlState::parse(&account.trading_control_state)
        .unwrap_or(TradingControlState::Normal);
    Ok(Json(json!({
        "trading_control_state": account.trading_control_state,
        "emergency_stop": account.emergency_stop,
        "blocks_buy": parsed.blocks_buy(),
        "blocks_all_trading": parsed.blocks_all_trading(),
        "allows_copy_sell": parsed.allows_copy_sell(),
        "resume_requires_confirmation": crate::trading_control::resume_requires_confirmation(parsed),
    })))
}

async fn set_account_state(
    state: &AppState,
    user_id: Uuid,
    account_id: Uuid,
    new_state: &str,
    action: &str,
    cancel_buys: bool,
) -> Result<(), ApiError> {
    let mut tx = begin_as(&state.pool, Some(user_id), "user")
        .await
        .map_err(internal)?;
    let prev: (String, String) = sqlx::query_as(
        "SELECT trading_control_state, operating_mode FROM trading_accounts WHERE id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(account_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    if cancel_buys {
        cancel_open_buy_orders(&mut tx, account_id, &prev.1)
            .await
            .map_err(internal)?;
    }
    let emergency = TradingControlState::parse(new_state)
        .map(|s| s.emergency_stop_flag())
        .unwrap_or(false);
    sqlx::query(
        "UPDATE trading_accounts SET trading_control_state = $3, emergency_stop = $4, updated_at = now() WHERE id = $1 AND user_id = $2",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(new_state)
    .bind(emergency)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    sqlx::query(
        "INSERT INTO audit_log (actor_user_id, action, target, metadata) VALUES ($1,$2,$3,$4)",
    )
    .bind(user_id)
    .bind(action)
    .bind(account_id.to_string())
    .bind(json!({
        "previous_state": prev.0,
        "new_state": new_state,
    }))
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(())
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
    pause_operations_for_kill_switch(&state.pool, id)
        .await
        .map_err(internal)?;
    set_account_state(
        &state,
        user.id,
        id,
        STATE_KILL_SWITCH,
        "account_kill_switch_on",
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
    set_account_state(
        &state,
        user.id,
        id,
        STATE_NORMAL,
        "account_kill_switch_off",
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
    set_account_state(
        &state,
        user.id,
        id,
        STATE_SOFT_CLOSE,
        "account_soft_close_on",
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
    set_account_state(
        &state,
        user.id,
        id,
        STATE_NORMAL,
        "account_soft_close_off",
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
        "SELECT trading_control_state FROM trading_accounts WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    let parsed = TradingControlState::parse(&current).unwrap_or(TradingControlState::Normal);
    if !crate::trading_control::resume_requires_confirmation(parsed) {
        return Err(bad("account is already in normal trading mode"));
    }
    if current == STATE_HARD_CLOSE_IN_PROGRESS {
        return Err(bad("wait for hard close to finish or activate kill switch"));
    }
    drop(tx);
    set_account_state(
        &state,
        user.id,
        id,
        STATE_NORMAL,
        "account_trading_resumed",
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
    let preview = preview_hard_close(&state.pool, user.id, id, &state.polymarket)
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
    if body.confirmation.as_deref() != Some("CLOSE ALL") {
        return Err(bad("type CLOSE ALL to confirm hard close"));
    }
    let idempotency = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let op_id = start_hard_close(&state.pool, user.id, id, idempotency)
        .await
        .map_err(|e| bad(e.to_string()))?;
    let _ = process_pending_operations(
        &state.pool,
        &BooksAdapter(&state.polymarket),
        state.cfg.fee_rate,
        state.cfg.fee_exponent,
    )
    .await;
    Ok(Json(json!({"operation_id": op_id})))
}

pub async fn get_hard_close(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, op_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let data = get_operation(&state.pool, user.id, id, op_id)
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
    retry_operation(&state.pool, user.id, id, op_id)
        .await
        .map_err(|e| bad(e.to_string()))?;
    let _ = process_pending_operations(
        &state.pool,
        &BooksAdapter(&state.polymarket),
        state.cfg.fee_rate,
        state.cfg.fee_exponent,
    )
    .await;
    Ok(Json(json!({"operation_id": op_id, "retrying": true})))
}

struct BooksAdapter<'a>(&'a crate::polymarket::http::PublicClient);

#[async_trait::async_trait]
impl crate::pipeline::Books for BooksAdapter<'_> {
    async fn book(&self, token_id: &str) -> Result<crate::engine::OrderBook, String> {
        self.0.order_book(token_id).await
    }
    async fn fee(&self, token_id: &str) -> Option<crate::engine::FeeSchedule> {
        self.0.fee_schedule(token_id).await
    }
}
