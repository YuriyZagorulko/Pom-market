use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use super::{bad, internal, require_user, ApiError, AppState};
use crate::source_trader_analytics::{
    build_dashboard, list_trades, request_refresh, TradesListQuery,
};
use crate::source_trader_copy_calculate::{fetch_calculate_cache_meta, run_calculate, CalculateRequest};

#[derive(Deserialize)]
pub struct PeriodQuery {
    pub period: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

pub async fn dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<PeriodQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let body = build_dashboard(
        &state.pool,
        user.id,
        id,
        q.period.as_deref(),
        q.from.as_deref(),
        q.to.as_deref(),
    )
    .await
    .map_err(|e| {
        if e.to_string().contains("not found") {
            ApiError(StatusCode::NOT_FOUND, e.to_string())
        } else {
            bad(&e.to_string())
        }
    })?;
    Ok(Json(serde_json::to_value(body).unwrap_or(serde_json::json!({}))))
}

pub async fn trades(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<TradesListQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let (items, total) = list_trades(&state.pool, user.id, id, &q)
        .await
        .map_err(|e| {
            if e.to_string().contains("not found") {
                ApiError(StatusCode::NOT_FOUND, e.to_string())
            } else {
                internal(&e)
            }
        })?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    Ok(Json(serde_json::json!({
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
    })))
}

pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    request_refresh(&state.pool, user.id, id)
        .await
        .map_err(|e| {
            if e.to_string().contains("not found") {
                ApiError(StatusCode::NOT_FOUND, e.to_string())
            } else {
                bad(&e.to_string())
            }
        })?;
    Ok(Json(serde_json::json!({"ok": true, "queued": true})))
}

pub async fn calculate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<CalculateRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let out = run_calculate(&state.pool, user.id, id, &body)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("not found") {
                ApiError(StatusCode::NOT_FOUND, msg)
            } else if msg.contains("too many trades") {
                bad(&msg)
            } else {
                internal(&msg)
            }
        })?;
    let body = serde_json::to_value(out).map_err(|e| internal(&e.to_string()))?;
    Ok(Json(body))
}

pub async fn calculate_cache_meta(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<PeriodQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let out = fetch_calculate_cache_meta(
        &state.pool,
        user.id,
        id,
        q.period.as_deref(),
        q.from.as_deref(),
        q.to.as_deref(),
    )
    .await
    .map_err(|e| {
        if e.to_string().contains("not found") {
            ApiError(StatusCode::NOT_FOUND, e.to_string())
        } else {
            bad(&e.to_string())
        }
    })?;
    let body = serde_json::to_value(out).map_err(|e| internal(&e.to_string()))?;
    Ok(Json(body))
}