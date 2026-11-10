use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use super::{bad, internal, require_user, ApiError, AppState};
use crate::strategy_analytics::{
    build_overview, compute_performance, equity_series, list_activity, list_positions,
    resolve_period,
};

#[derive(Deserialize)]
pub struct PeriodQuery {
    pub period: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Deserialize)]
pub struct PositionsQuery {
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Deserialize)]
pub struct ActivityQuery {
    pub side: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub status: Option<String>,
}

pub async fn overview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let body = build_overview(&state.pool, user.id, id, &state.cfg)
        .await
        .map_err(|e| {
            if e.to_string().contains("not found") {
                bad("strategy not found")
            } else {
                internal(e)
            }
        })?;
    Ok(Json(
        serde_json::to_value(body).unwrap_or(serde_json::json!({})),
    ))
}

pub async fn equity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<PeriodQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let (start, end, label) =
        resolve_period(q.period.as_deref(), q.from.as_deref(), q.to.as_deref()).map_err(bad)?;
    let series = equity_series(
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
            bad("strategy not found")
        } else {
            internal(e)
        }
    })?;
    Ok(Json(serde_json::json!({
        "period": label,
        "from": start,
        "to": end,
        "equity_label": crate::strategy_analytics::PARTIAL_EQUITY_LABEL,
        "equity_note": crate::strategy_analytics::EQUITY_MODEL_NOTE,
        "points": series,
        "empty_note": if series.is_empty() {
            "Historical points appear after worker snapshots (every 15 minutes). No prior history is fabricated."
        } else {
            ""
        },
    })))
}

pub async fn positions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<PositionsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let status = q.status.as_deref().unwrap_or("open");
    let limit = q.limit.unwrap_or(25).clamp(1, 100);
    let offset = q.offset.unwrap_or(0).max(0);
    let (items, total) = list_positions(&state.pool, user.id, id, status, limit, offset)
        .await
        .map_err(|e| {
            if e.to_string().contains("not found") {
                bad("strategy not found")
            } else {
                internal(e)
            }
        })?;
    Ok(Json(serde_json::json!({
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
    })))
}

pub async fn activity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<ActivityQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let limit = q.limit.unwrap_or(25).clamp(1, 100);
    let offset = q.offset.unwrap_or(0).max(0);
    let from = q
        .from
        .as_deref()
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|d| d.with_timezone(&chrono::Utc))
                .map_err(|_| "invalid from")
        })
        .transpose()
        .map_err(bad)?;
    let to =
        q.to.as_deref()
            .map(|s| {
                chrono::DateTime::parse_from_rfc3339(s)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .map_err(|_| "invalid to")
            })
            .transpose()
            .map_err(bad)?;
    let side = q
        .side
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let (items, total) = list_activity(
        &state.pool,
        user.id,
        id,
        crate::strategy_analytics::ActivityListParams {
            side: side.as_deref(),
            from,
            to,
            limit,
            offset,
        },
    )
    .await
    .map_err(|e| {
        if e.to_string().contains("not found") {
            bad("strategy not found")
        } else {
            internal(e)
        }
    })?;
    let mut items = items;
    if let Some(st) = q.status.as_deref() {
        items.retain(|r| r.execution_status.eq_ignore_ascii_case(st));
    }
    Ok(Json(serde_json::json!({
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
    })))
}

pub async fn performance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let body = compute_performance(&state.pool, user.id, id, &state.cfg)
        .await
        .map_err(|e| {
            if e.to_string().contains("not found") {
                bad("strategy not found")
            } else {
                internal(e)
            }
        })?;
    Ok(Json(
        serde_json::to_value(body).unwrap_or(serde_json::json!({})),
    ))
}
