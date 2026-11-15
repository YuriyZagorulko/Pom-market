use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use super::page::ListQuery;
use axum::http::StatusCode;

use super::{bad, internal, require_user, ApiError, AppState};
use crate::strategy_analytics::{
    build_overview, equity_series, list_activity, list_positions, resolve_period,
};
use crate::strategy_performance::{
    build_performance_dashboard, compute_execution_stats, list_key_events, list_performance_positions,
    position_history, PerformancePositionQuery,
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

/// Strategy activity list filters (same query shape as other paginated feeds).
pub type ActivityQuery = ListQuery;

pub async fn overview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let body = build_overview(&state.pool, user.id, id, &state.cfg)
        .await
        .map_err(|e| map_analytics_error(e))?;
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
    .map_err(map_analytics_error)?;
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
        .map_err(map_analytics_error)?;
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
    let limit = q.limit();
    let offset = q.offset();
    let page = q.page.unwrap_or(1).max(1);
    let from = q.parse_from();
    let to = q.parse_to_exclusive();
    let activity_type = q
        .filter_activity_type()
        .or_else(|| q.filter_side());
    let search = q.filter_q();
    let status = q.filter_status();
    let (items, total) = list_activity(
        &state.pool,
        user.id,
        id,
        crate::strategy_analytics::ActivityListParams {
            activity_type: activity_type.as_deref(),
            side: None,
            status: status.as_deref(),
            search: search.as_deref(),
            from,
            to,
            limit,
            offset,
        },
    )
    .await
    .map_err(map_analytics_error)?;
    let total_pages = if total == 0 {
        0
    } else {
        (total + limit - 1) / limit
    };
    Ok(Json(serde_json::json!({
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
        "page": page,
        "page_size": limit,
        "total_pages": total_pages,
    })))
}

#[derive(Deserialize)]
pub struct PerformancePositionsQuery {
    #[serde(flatten)]
    pub list: ListQuery,
    pub status: Option<String>,
    pub sort: Option<String>,
    pub pnl_filter: Option<String>,
}

pub async fn performance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(period): Query<PeriodQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let body = build_performance_dashboard(
        &state.pool,
        user.id,
        id,
        &state.cfg,
        period.period.as_deref(),
        period.from.as_deref(),
        period.to.as_deref(),
    )
    .await
        .map_err(map_analytics_error)?;
    Ok(Json(
        serde_json::to_value(body).unwrap_or(serde_json::json!({})),
    ))
}

pub async fn performance_positions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<PerformancePositionsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let status = q.status.as_deref().unwrap_or("all");
    let sort = q.sort.as_deref().unwrap_or("profit_desc");
    let pnl_filter = match q.pnl_filter.as_deref() {
        Some("profit") => Some("profit"),
        Some("loss") => Some("loss"),
        _ => None,
    };
    let limit = q.list.limit();
    let offset = q.list.offset();
    let (items, total, truncated) = list_performance_positions(
        &state.pool,
        user.id,
        id,
        PerformancePositionQuery {
            status: if status == "open" {
                "open"
            } else if status == "closed" {
                "closed"
            } else {
                "all"
            },
            sort: match sort {
                "profit_asc" => "profit_asc",
                "roi_desc" => "roi_desc",
                "invested_desc" => "invested_desc",
                "closed_recent" => "closed_recent",
                _ => "profit_desc",
            },
            pnl_filter,
            search: q.list.filter_q(),
            limit,
            offset,
        },
    )
    .await
    .map_err(map_analytics_error)?;
    Ok(Json(serde_json::json!({
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
        "truncated": truncated,
        "truncation_note": if truncated {
            Some(format!("Only the first {} positions are ranked; narrow filters or status for full accuracy.", crate::strategy_performance::MAX_POSITIONS_MATERIALIZE))
        } else {
            None
        },
    })))
}

pub async fn performance_key_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(period): Query<PeriodQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let limit = 80i64;
    let items = list_key_events(
        &state.pool,
        user.id,
        id,
        period.period.as_deref(),
        period.from.as_deref(),
        period.to.as_deref(),
        limit,
    )
    .await
    .map_err(map_analytics_error)?;
    Ok(Json(serde_json::json!({ "items": items, "period": period.period })))
}

pub async fn performance_execution(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(period): Query<PeriodQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let body = compute_execution_stats(
        &state.pool,
        user.id,
        id,
        period.period.as_deref(),
        period.from.as_deref(),
        period.to.as_deref(),
    )
    .await
    .map_err(map_analytics_error)?;
    Ok(Json(serde_json::to_value(body).unwrap_or(serde_json::json!({}))))
}

pub async fn performance_position_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, token_id)): Path<(Uuid, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let items = position_history(&state.pool, user.id, id, &token_id)
        .await
        .map_err(map_analytics_error)?;
    Ok(Json(serde_json::json!({ "items": items, "token_id": token_id })))
}

fn map_analytics_error(err: impl std::fmt::Display) -> ApiError {
    let msg = err.to_string();
    if msg.contains("strategy not found") {
        ApiError(StatusCode::NOT_FOUND, "strategy not found".into())
    } else if msg.contains("capital account missing") {
        ApiError(StatusCode::NOT_FOUND, "strategy not found".into())
    } else {
        internal(err)
    }
}

#[cfg(test)]
mod query_tests {
    use super::ActivityQuery;
    use serde_urlencoded;

    #[test]
    fn activity_query_deserializes_pagination() {
        let q: ActivityQuery =
            serde_urlencoded::from_str("limit=25&page=2&activity_type=buy&status=skipped")
                .expect("query string should deserialize");
        assert_eq!(q.limit(), 25);
        assert_eq!(q.offset(), 25);
        assert_eq!(q.filter_activity_type().as_deref(), Some("buy"));
        assert_eq!(q.filter_status().as_deref(), Some("skipped"));
    }
}
