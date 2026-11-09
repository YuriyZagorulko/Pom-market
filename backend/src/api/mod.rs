use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, Query, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use chrono::Utc;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::auth::{self, AuthUser};
use crate::config::Config;
use crate::email::{Email, MailError};
use crate::db::begin_as;
use crate::engine::{is_eth_address, is_private_key, normalize_address};
use crate::polymarket::http::PublicClient;
use crate::polymarket::live::probe_account;
use crate::security::{credential_aad, hash_password, random_token, sha256_hex, verify_password, SecretBlob, Vault};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub cfg: Config,
    pub vault: Vault,
    pub polymarket: PublicClient,
    pub activity: broadcast::Sender<String>,
    pub login_attempts: std::sync::Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    pub recovery_attempts: std::sync::Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    pub verify_attempts: std::sync::Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    pub mail: Email,
}

#[derive(Serialize)]
struct ErrBody {
    error: String,
}

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.0, Json(ErrBody { error: self.1 })).into_response()
    }
}

fn bad(message: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, message.into())
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/v1/openapi.json", get(openapi))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/refresh", post(refresh))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/auth/me", get(me))
        .route("/api/v1/auth/password/setup", post(password_setup))
        .route("/api/v1/auth/password/change", post(password_change))
        .route("/api/v1/auth/password/reset/request", post(reset_request))
        .route("/api/v1/auth/password/recovery/request", post(forgot_password))
        .route("/api/v1/auth/password/reset/confirm", post(password_setup))
        .route("/api/auth/forgot-password", post(forgot_password))
        .route("/api/auth/reset-password", post(reset_password))
        .route("/api/v1/auth/email/verify/request", post(verify_email_request))
        .route("/api/v1/auth/email/verify/confirm", post(verify_email_confirm))
        .route("/api/v1/admin/overview", get(admin_overview))
        .route("/api/v1/admin/users", get(admin_users).post(admin_create_user))
        .route("/api/v1/admin/users/{id}", get(admin_user).patch(admin_update_user))
        .route("/api/v1/admin/users/{id}/suspend", post(admin_suspend))
        .route("/api/v1/admin/users/{id}/activate", post(admin_activate))
        .route("/api/v1/admin/users/{id}/invite", post(admin_invite))
        .route("/api/v1/admin/health", get(admin_health))
        .route("/api/v1/admin/events", get(admin_events))
        .route("/api/v1/admin/settings", get(admin_settings))
        .route("/api/v1/admin/kill-switch", post(admin_kill))
        .route("/api/v1/dashboard", get(dashboard))
        .route("/api/v1/accounts", get(list_accounts).post(create_account))
        .route("/api/v1/accounts/{id}", get(get_account).patch(patch_account).delete(delete_account))
        .route("/api/v1/accounts/{id}/test", post(test_account))
        .route("/api/v1/accounts/{id}/balance", get(account_balance))
        .route("/api/v1/accounts/{id}/positions", get(account_positions))
        .route("/api/v1/accounts/{id}/mode", post(set_mode))
        .route("/api/v1/accounts/{id}/disconnect", post(disconnect_account))
        .route("/api/v1/accounts/{id}/risk", patch(patch_risk))
        .route("/api/v1/accounts/{id}/credentials/rotate", post(rotate_credentials))
        .route("/api/v1/source-traders", get(list_traders).post(create_trader))
        .route("/api/v1/source-traders/{id}", get(get_trader).patch(patch_trader).delete(delete_trader))
        .route("/api/v1/strategies", get(list_strategies).post(create_strategy))
        .route("/api/v1/strategies/{id}", get(get_strategy).patch(patch_strategy).delete(delete_strategy))
        .route("/api/v1/strategies/{id}/enable", post(enable_strategy))
        .route("/api/v1/activity", get(list_activity))
        .route("/api/v1/source-trades", get(list_source_trades))
        .route("/api/v1/decisions", get(list_decisions))
        .route("/api/v1/orders", get(list_orders))
        .route("/api/v1/fills", get(list_fills))
        .route("/api/v1/positions", get(list_positions))
        .route("/api/v1/closed-positions", get(list_closed))
        .route("/api/v1/trades/{id}", get(trade_detail))
        .route("/api/v1/ws", get(ws_upgrade))
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let db = sqlx::query("SELECT 1").execute(&state.pool).await.is_ok();
    Json(json!({"status": if db { "ok" } else { "degraded" }, "database": db}))
}

async fn openapi() -> impl IntoResponse {
    Json(serde_json::from_str::<Value>(include_str!("openapi.json")).unwrap_or_else(|_| json!({"openapi": "3.0.3"})))
}

#[derive(Deserialize)]
struct LoginBody {
    email: String,
    password: String,
}

async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<LoginBody>,
) -> Result<impl IntoResponse, ApiError> {
    let ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("local")
        .to_string();
    if rate_limited(&state, &ip) {
        return Err(ApiError(StatusCode::TOO_MANY_REQUESTS, "too many login attempts".into()));
    }
    let client_ip = ip.split(',').next().unwrap_or("").trim().to_string();
    let stored_ip = client_ip.parse::<std::net::IpAddr>().ok().map(|_| client_ip);
    let attempt = auth::login(
        &state.pool,
        &state.cfg.jwt_secret,
        &body.email,
        &body.password,
        headers.get("user-agent").and_then(|v| v.to_str().ok()).map(str::to_string),
        stored_ip,
        state.cfg.access_ttl_seconds,
        state.cfg.refresh_ttl_seconds,
    )
    .await;
    if let Some(alert) = attempt.security_alert {
        dispatch_security_lock_email(state.mail.clone(), alert);
    }
    let result = attempt
        .result
        .map_err(|message| ApiError(StatusCode::UNAUTHORIZED, message.into()))?;
    let jar = jar.add(refresh_cookie(&result.refresh_token, state.cfg.refresh_ttl_seconds));
    Ok((jar, Json(json!({
        "access_token": result.access_token,
        "token_type": "Bearer",
        "user": user_json(&result.user)
    }))))
}

async fn refresh(State(state): State<AppState>, jar: CookieJar) -> Result<impl IntoResponse, ApiError> {
    let token = jar.get("pom_refresh").map(|c| c.value().to_string()).ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "missing refresh session".into()))?;
    let result = auth::refresh(&state.pool, &state.cfg.jwt_secret, &token, state.cfg.access_ttl_seconds, state.cfg.refresh_ttl_seconds)
        .await
        .map_err(|_| ApiError(StatusCode::UNAUTHORIZED, "invalid session".into()))?;
    let jar = jar.add(refresh_cookie(&result.refresh_token, state.cfg.refresh_ttl_seconds));
    Ok((jar, Json(json!({"access_token": result.access_token, "user": user_json(&result.user)}))))
}

async fn logout(State(state): State<AppState>, jar: CookieJar, headers: HeaderMap) -> Result<impl IntoResponse, ApiError> {
    if let Some(user) = bearer_user(&state, &headers) {
        let _ = auth::logout(&state.pool, user.sid).await;
    }
    Ok((jar.remove(Cookie::from("pom_refresh")), Json(json!({"ok": true}))))
}

async fn me(State(state): State<AppState>, headers: HeaderMap) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, None)?;
    Ok(Json(user_json(&user)))
}

#[derive(Deserialize)]
struct PasswordBody {
    token: String,
    password: String,
}

async fn password_setup(State(state): State<AppState>, Json(body): Json<PasswordBody>) -> Result<impl IntoResponse, ApiError> {
    auth::consume_invitation(&state.pool, &body.token, &body.password)
        .await
        .map_err(|e| bad(e.to_string()))?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct ChangePassword {
    current_password: String,
    new_password: String,
}

async fn password_change(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<ChangePassword>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, None)?;
    let mut tx = begin_as(&state.pool, Some(user.id), &user.role).await.map_err(internal)?;
    let profile: UserProfileRow = sqlx::query_as("SELECT email, display_name FROM users WHERE id = $1")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let hash: Option<String> = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    if !verify_password(&body.current_password, hash.as_deref().unwrap_or("")) {
        return Err(ApiError(StatusCode::UNAUTHORIZED, "current password is wrong".into()));
    }
    let new_hash = hash_password(&body.new_password).map_err(|e| bad(e.to_string()))?;
    sqlx::query("UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1")
        .bind(user.id)
        .bind(new_hash)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    dispatch_password_changed_email(state.mail.clone(), profile.email, profile.display_name);
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct ResetRequest {
    email: String,
}

async fn reset_request(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<ResetRequest>) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin").await.map_err(internal)?;
    let user_id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE lower(email) = lower($1)")
        .bind(body.email.trim())
        .fetch_optional(&mut *tx)
        .await
        .map_err(internal)?;
    let Some(user_id) = user_id else {
        return Ok(Json(json!({"ok": true})));
    };
    let profile: UserProfileRow = sqlx::query_as("SELECT email, display_name FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let token = issue_reset_token(&mut tx, user_id, state.cfg.password_reset_ttl_minutes).await?;
    audit(&mut tx, Some(admin.id), "password_reset_issued", &user_id.to_string()).await;
    tx.commit().await.map_err(internal)?;
    let setup_url = state.mail.reset_password_url(&token);
    dispatch_password_recovery_email(state.mail.clone(), profile.email, profile.display_name, token);
    Ok(Json(json!({
        "ok": true,
        "setup_url": setup_url
    })))
}

pub const FORGOT_PASSWORD_MESSAGE: &str =
    "If an account with this email exists, you will receive a password reset link.";

fn valid_email(email: &str) -> bool {
    let email = email.trim();
    if email.len() < 5 || email.len() > 254 || email.chars().any(char::is_whitespace) {
        return false;
    }
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

async fn forgot_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ResetRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if !valid_email(&body.email) {
        return Err(bad("email is invalid"));
    }
    let ip = client_ip(&headers);
    let email_key = format!("email:{}", body.email.trim().to_lowercase());
    if rate_limited_map(&state.recovery_attempts, &ip, 5, 60)
        || rate_limited_map(&state.recovery_attempts, &email_key, 5, 3600)
    {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "too many recovery attempts".into(),
        ));
    }
    let token = random_token();
    let issued: Option<UserProfileRow> = sqlx::query_as(
        "SELECT email, display_name FROM auth_create_reset_token($1, $2, $3)",
    )
    .bind(body.email.trim())
    .bind(sha256_hex(&token))
    .bind(state.cfg.password_reset_ttl_minutes as i32)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    if let Some(profile) = issued {
        dispatch_password_recovery_email(state.mail.clone(), profile.email, profile.display_name, token);
    }
    Ok(Json(json!({ "message": FORGOT_PASSWORD_MESSAGE })))
}

#[derive(Deserialize)]
struct ResetPasswordBody {
    token: String,
    new_password: String,
}

async fn reset_password(State(state): State<AppState>, Json(body): Json<ResetPasswordBody>) -> Result<impl IntoResponse, ApiError> {
    if body.token.trim().is_empty() {
        return Err(bad("reset token is required"));
    }
    let email = auth::consume_password_reset(&state.pool, body.token.trim(), &body.new_password)
        .await
        .map_err(reset_password_error)?;
    dispatch_password_changed_email(state.mail.clone(), email.clone(), email);
    Ok(Json(json!({"ok": true})))
}

fn reset_password_error(err: anyhow::Error) -> ApiError {
    let message = err.to_string();
    if message.contains("at least 10 characters") {
        return bad(message);
    }
  bad(message)
}

#[derive(Deserialize)]
struct TokenOnly {
    token: String,
}

async fn verify_email_request(State(state): State<AppState>, headers: HeaderMap) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, None)?;
    let ip = client_ip(&headers);
    if rate_limited_map(&state.verify_attempts, &format!("{}:{ip}", user.id), 3, 60) {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "too many verification attempts".into(),
        ));
    }
    let mut tx = begin_as(&state.pool, Some(user.id), &user.role).await.map_err(internal)?;
    let profile: VerifyUserRow = sqlx::query_as(
        "SELECT email, display_name, email_verified_at FROM users WHERE id = $1",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    if profile.email_verified_at.is_some() {
        return Ok(Json(json!({"ok": true, "status": "verified"})));
    }
    let token = random_token();
    sqlx::query(
        "INSERT INTO invitation_tokens (user_id, token_hash, purpose, expires_at) VALUES ($1,$2,'verify', now() + make_interval(hours => $3::int))",
    )
    .bind(user.id)
    .bind(sha256_hex(&token))
    .bind(state.cfg.invite_ttl_hours)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    dispatch_email_verification(state.mail.clone(), profile.email, profile.display_name, token);
    Ok(Json(json!({"ok": true, "status": "sent"})))
}

async fn verify_email_confirm(State(state): State<AppState>, Json(body): Json<TokenOnly>) -> Result<impl IntoResponse, ApiError> {
    auth::consume_verification(&state.pool, &body.token)
        .await
        .map_err(|e| bad(e.to_string()))?;
    Ok(Json(json!({"ok": true, "status": "verified"})))
}

async fn admin_overview(State(state): State<AppState>, headers: HeaderMap) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin").await.map_err(internal)?;
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users").fetch_one(&mut *tx).await.map_err(internal)?;
    let accounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM trading_accounts").fetch_one(&mut *tx).await.map_err(internal)?;
    let traders: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM source_traders").fetch_one(&mut *tx).await.map_err(internal)?;
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM source_events").fetch_one(&mut *tx).await.map_err(internal)?;
    let errors: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM system_events WHERE severity = 'error' AND created_at > now() - interval '24 hours'")
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let kill: Value = sqlx::query_scalar("SELECT value FROM system_settings WHERE key = 'global_kill_switch'")
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    Ok(Json(json!({"users": users, "accounts": accounts, "source_traders": traders, "source_events": events, "errors_24h": errors, "kill_switch": kill})))
}

#[derive(Deserialize)]
struct CreateUser {
    email: String,
    display_name: String,
    role: Option<String>,
}

#[derive(sqlx::FromRow)]
struct UserByEmail {
    id: Uuid,
    email: String,
    display_name: String,
    role: String,
    status: String,
}

async fn admin_create_user(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<CreateUser>) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    let role = body.role.unwrap_or_else(|| "user".into());
    if role != "user" && role != "admin" {
        return Err(bad("role must be user or admin"));
    }
    let email = body.email.trim().to_lowercase();
    if !email.contains('@') {
        return Err(bad("email is invalid"));
    }
    let display_name = body.display_name.trim().to_string();
    if display_name.is_empty() {
        return Err(bad("display name is required"));
    }
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin").await.map_err(internal)?;
    if let Some(existing) = sqlx::query_as::<_, UserByEmail>(
        "SELECT id, email, display_name, role, status FROM users WHERE email = $1",
    )
    .bind(&email)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?
    {
        let mut payload = json!({
            "id": existing.id,
            "email": existing.email,
            "display_name": existing.display_name,
            "role": existing.role,
            "status": existing.status,
            "already_exists": true,
        });
        if existing.status == "invited" {
            let token = random_token();
            sqlx::query("INSERT INTO invitation_tokens (user_id, token_hash, purpose, expires_at) VALUES ($1,$2,'invite', now() + make_interval(hours => $3::int))")
                .bind(existing.id)
                .bind(sha256_hex(&token))
                .bind(state.cfg.invite_ttl_hours)
                .execute(&mut *tx)
                .await
                .map_err(internal)?;
            audit(&mut tx, Some(admin.id), "user_reinvited", &existing.id.to_string()).await;
            tx.commit().await.map_err(internal)?;
            let setup_url = state.mail.setup_url(&token);
            let email_delivery = invitation_delivery(&state.mail, &existing.email, &existing.display_name, &token).await;
            payload["setup_url"] = json!(setup_url);
            payload["email_delivery"] = email_delivery;
        } else {
            tx.rollback().await.map_err(internal)?;
        }
        return Ok((StatusCode::OK, Json(payload)));
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, role, status) VALUES ($1, $2, $3, 'invited') RETURNING id",
    )
    .bind(&email)
    .bind(&display_name)
    .bind(&role)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| bad("could not create user"))?;
    let token = random_token();
    sqlx::query("INSERT INTO invitation_tokens (user_id, token_hash, purpose, expires_at) VALUES ($1,$2,'invite', now() + make_interval(hours => $3::int))")
        .bind(id)
        .bind(sha256_hex(&token))
        .bind(state.cfg.invite_ttl_hours)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    audit(&mut tx, Some(admin.id), "user_created", &id.to_string()).await;
    tx.commit().await.map_err(internal)?;
    let setup_url = state.mail.setup_url(&token);
    let email_delivery = invitation_delivery(&state.mail, &email, &display_name, &token).await;
    Ok((StatusCode::CREATED, Json(json!({
        "id": id,
        "email": email,
        "display_name": display_name,
        "role": role,
        "status": "invited",
        "setup_url": setup_url,
        "email_delivery": email_delivery,
    }))))
}

async fn admin_users(State(state): State<AppState>, headers: HeaderMap) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin").await.map_err(internal)?;
    let rows = sqlx::query_as::<_, UserList>(
        "SELECT u.id, u.email, u.display_name, u.role, u.status, u.created_at, u.last_login_at,
                (SELECT COUNT(*) FROM trading_accounts a WHERE a.user_id = u.id) AS accounts,
                (SELECT COUNT(*) FROM source_traders t WHERE t.user_id = u.id) AS traders
         FROM users u ORDER BY u.created_at DESC",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

async fn admin_user(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin").await.map_err(internal)?;
    let row = sqlx::query_as::<_, UserList>(
        "SELECT u.id, u.email, u.display_name, u.role, u.status, u.created_at, u.last_login_at,
                (SELECT COUNT(*) FROM trading_accounts a WHERE a.user_id = u.id) AS accounts,
                (SELECT COUNT(*) FROM source_traders t WHERE t.user_id = u.id) AS traders
         FROM users u WHERE u.id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?
    .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "user not found".into()))?;
    Ok(Json(row))
}

#[derive(Deserialize)]
struct UpdateUser {
    display_name: Option<String>,
    role: Option<String>,
}

async fn admin_update_user(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>, Json(body): Json<UpdateUser>) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    if let Some(role) = &body.role {
        if role != "user" && role != "admin" {
            return Err(bad("role must be user or admin"));
        }
    }
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin").await.map_err(internal)?;
    sqlx::query(
        "UPDATE users SET display_name = COALESCE($2, display_name), role = COALESCE($3, role), updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(body.display_name)
    .bind(body.role)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    audit(&mut tx, Some(admin.id), "user_updated", &id.to_string()).await;
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

async fn admin_suspend(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    set_status(&state, &headers, id, "suspended").await
}

async fn admin_activate(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    set_status(&state, &headers, id, "active").await
}

async fn set_status(state: &AppState, headers: &HeaderMap, id: Uuid, status: &str) -> Result<Json<Value>, ApiError> {
    let admin = require_user(state, headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin").await.map_err(internal)?;
    let updated = sqlx::query("UPDATE users SET status = $2, updated_at = now() WHERE id = $1 AND password_hash IS NOT NULL")
        .bind(id)
        .bind(status)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    if updated.rows_affected() == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, "user not found or password is not set".into()));
    }
    audit(&mut tx, Some(admin.id), status, &id.to_string()).await;
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"ok": true, "status": status})))
}

async fn admin_invite(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin").await.map_err(internal)?;
    let profile: UserProfileRow = sqlx::query_as("SELECT email, display_name FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "user not found".into()))?;
    let token = random_token();
    sqlx::query("INSERT INTO invitation_tokens (user_id, token_hash, purpose, expires_at) VALUES ($1,$2,'invite', now() + make_interval(hours => $3::int))")
        .bind(id)
        .bind(sha256_hex(&token))
        .bind(state.cfg.invite_ttl_hours)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let setup_url = state.mail.setup_url(&token);
    let email_delivery = invitation_delivery(&state.mail, &profile.email, &profile.display_name, &token).await;
    Ok(Json(json!({
        "setup_url": setup_url,
        "email_delivery": email_delivery,
    })))
}

async fn admin_health(State(state): State<AppState>, headers: HeaderMap) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin").await.map_err(internal)?;
    let settings = sqlx::query_as::<_, SettingRow>("SELECT key, value, updated_at FROM system_settings ORDER BY key")
        .fetch_all(&mut *tx)
        .await
        .map_err(internal)?;
    Ok(Json(settings))
}

async fn admin_events(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<PageQuery>) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin").await.map_err(internal)?;
    let rows = sqlx::query_as::<_, SystemEventRow>(
        "SELECT id, severity, component, message, created_at FROM system_events ORDER BY created_at DESC LIMIT $1 OFFSET $2",
    )
    .bind(q.limit())
    .bind(q.offset.unwrap_or(0))
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

async fn admin_settings(State(state): State<AppState>, headers: HeaderMap) -> Result<impl IntoResponse, ApiError> {
    admin_health(State(state), headers).await
}

#[derive(Deserialize)]
struct KillBody {
    enabled: bool,
}

async fn admin_kill(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<KillBody>) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin").await.map_err(internal)?;
    sqlx::query("UPDATE system_settings SET value = $1, updated_at = now() WHERE key = 'global_kill_switch'")
        .bind(json!({"enabled": body.enabled}))
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    audit(&mut tx, Some(admin.id), "kill_switch", if body.enabled { "on" } else { "off" }).await;
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"enabled": body.enabled})))
}

async fn dashboard(State(state): State<AppState>, headers: HeaderMap) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let accounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM trading_accounts WHERE user_id = $1").bind(user.id).fetch_one(&mut *tx).await.map_err(internal)?;
    let traders: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM source_traders WHERE user_id = $1").bind(user.id).fetch_one(&mut *tx).await.map_err(internal)?;
    let detected: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM source_events e WHERE EXISTS (SELECT 1 FROM source_traders t WHERE t.user_id = $1 AND lower(t.wallet_address) = lower(e.source_wallet))",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    let copied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM copy_decisions WHERE user_id = $1 AND status = 'approved'")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let rejected: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM copy_decisions WHERE user_id = $1 AND status IN ('rejected', 'skipped')")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let open_positions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM positions WHERE user_id = $1 AND open_shares > 0")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let pnl: Decimal = sqlx::query_scalar("SELECT COALESCE(SUM(realized_pnl_usd), 0) FROM positions WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let account_rows = sqlx::query_as::<_, AccountCard>(
        "SELECT id, name, signer_address, funder_address, signature_type, operating_mode, status, emergency_stop, last_exchange_balance_usd, last_balance_checked_at, last_connection_error
         FROM trading_accounts WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user.id)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    let collector: Value = sqlx::query_scalar("SELECT value FROM system_settings WHERE key = 'collector_status'")
        .fetch_optional(&mut *tx)
        .await
        .map_err(internal)?
        .unwrap_or(json!({}));
    let heartbeat: Value = sqlx::query_scalar("SELECT value FROM system_settings WHERE key = 'worker_heartbeat'")
        .fetch_optional(&mut *tx)
        .await
        .map_err(internal)?
        .unwrap_or(json!({}));
    Ok(Json(json!({
        "accounts": accounts,
        "source_traders": traders,
        "detected_trades": detected,
        "copied_trades": copied,
        "skipped_or_rejected": rejected,
        "open_positions": open_positions,
        "realized_pnl_usd": pnl,
        "account_cards": account_rows,
        "collector": collector,
        "worker": heartbeat
    })))
}

#[derive(Deserialize)]
struct AccountBody {
    name: String,
    signature_type: String,
    signer_address: String,
    funder_address: Option<String>,
    private_key: Option<String>,
}

async fn create_account(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<AccountBody>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    validate_signature(&body.signature_type)?;
    if !is_eth_address(&body.signer_address) {
        return Err(bad("signer address is invalid"));
    }
    let funder = body.funder_address.clone().unwrap_or_else(|| body.signer_address.clone());
    if !is_eth_address(&funder) {
        return Err(bad("funder address is invalid"));
    }
    if let Some(key) = &body.private_key {
        if !is_private_key(key) {
            return Err(bad("private key must be a 32-byte hex key, not a seed phrase"));
        }
        let derived = crate::polymarket::live::address_of(key).map_err(|e| bad(e))?;
        if derived != normalize_address(&body.signer_address) {
            return Err(bad("private key does not match signer address"));
        }
    }
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signature_type, signer_address, funder_address, operating_mode, status)
         VALUES ($1,$2,$3,$4,$5,'monitoring','pending') RETURNING id",
    )
    .bind(user.id)
    .bind(body.name.trim())
    .bind(&body.signature_type)
    .bind(normalize_address(&body.signer_address))
    .bind(normalize_address(&funder))
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    sqlx::query(
        "INSERT INTO account_risk_limits (trading_account_id, user_id, max_order_usd, max_daily_loss_usd, max_exposure_usd, max_slippage_bps, max_orders_per_interval, interval_seconds, max_stale_data_seconds, paper_cash_usd)
         VALUES ($1,$2,50,100,500,150,20,60,120,0)",
    )
    .bind(id)
    .bind(user.id)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    if let Some(key) = body.private_key.as_deref() {
        store_key(&state.vault, &mut tx, user.id, id, key).await.map_err(internal)?;
    }
    audit(&mut tx, Some(user.id), "account_created", &id.to_string()).await;
    tx.commit().await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

async fn list_accounts(State(state): State<AppState>, headers: HeaderMap) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let rows = sqlx::query_as::<_, AccountCard>(
        "SELECT id, name, signer_address, funder_address, signature_type, operating_mode, status, emergency_stop, last_exchange_balance_usd, last_balance_checked_at, last_connection_error
         FROM trading_accounts WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user.id)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

async fn get_account(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let row = load_account(&mut tx, user.id, id).await?;
    let risk = sqlx::query_as::<_, RiskRow>("SELECT * FROM account_risk_limits WHERE trading_account_id = $1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    Ok(Json(json!({"account": row, "risk": risk, "credentials_stored": credential_exists(&mut tx, id).await?})))
}

#[derive(Deserialize)]
struct PatchAccount {
    name: Option<String>,
}

async fn patch_account(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>, Json(body): Json<PatchAccount>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let updated = sqlx::query("UPDATE trading_accounts SET name = COALESCE($3, name), updated_at = now() WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .bind(body.name)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    if updated.rows_affected() == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, "account not found".into()));
    }
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

async fn delete_account(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let updated = sqlx::query("DELETE FROM trading_accounts WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    if updated.rows_affected() == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, "account not found".into()));
    }
    audit(&mut tx, Some(user.id), "account_deleted", &id.to_string()).await;
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

async fn test_account(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let clob_ok = state.polymarket.clob_ok().await.unwrap_or(false);
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let account = load_account(&mut tx, user.id, id).await?;
    let secret = load_secret(&state.vault, &mut tx, user.id, id).await?;
    tx.commit().await.map_err(internal)?;
    let mut report = json!({
        "address_valid": is_eth_address(&account.signer_address),
        "signature_type": account.signature_type,
        "clob_reachable": clob_ok,
        "balance_usd": Value::Null,
        "error": Value::Null
    });
    if let Some(secret) = secret {
        let key = secret.get("private_key").and_then(|v| v.as_str()).unwrap_or("");
        let probed = probe_account(&state.cfg.clob_host, key, &account.signature_type, Some(&account.funder_address)).await;
        report["balance_usd"] = serde_json::to_value(probed.balance_usd).unwrap_or(Value::Null);
        report["api_credentials_created"] = json!(probed.api_credentials_created);
        report["signer_matches"] = json!(probed.signer_address == account.signer_address);
        if let Some(error) = &probed.error {
            report["error"] = json!(error);
        }
        let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
        sqlx::query(
            "UPDATE trading_accounts SET status = $3, last_exchange_balance_usd = $4, last_balance_checked_at = now(), last_connection_error = $5, updated_at = now() WHERE id = $1 AND user_id = $2",
        )
        .bind(id)
        .bind(user.id)
        .bind(if probed.error.is_none() && clob_ok { "connected" } else { "error" })
        .bind(probed.balance_usd)
        .bind(&probed.error)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
        tx.commit().await.map_err(internal)?;
    } else if clob_ok {
        let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
        sqlx::query("UPDATE trading_accounts SET status = 'connected', last_connection_error = NULL, updated_at = now() WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user.id)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
        tx.commit().await.map_err(internal)?;
    } else {
        report["error"] = json!("Polymarket CLOB is not reachable and no trading key is stored");
    }
    Ok(Json(report))
}

async fn account_balance(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let account = load_account(&mut tx, user.id, id).await?;
    let paper: Decimal = sqlx::query_scalar("SELECT paper_cash_usd FROM account_risk_limits WHERE trading_account_id = $1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    Ok(Json(json!({
        "exchange_balance_usd": account.last_exchange_balance_usd,
        "exchange_balance_checked_at": account.last_balance_checked_at,
        "paper_cash_usd": if account.operating_mode == "paper_trading" { Some(paper) } else { None },
        "connection_error": account.last_connection_error
    })))
}

async fn account_positions(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let _ = load_account(&mut tx, user.id, id).await?;
    let rows = sqlx::query_as::<_, PositionView>(
        "SELECT id, source_wallet, token_id, market_title, outcome, operating_mode, open_shares, avg_price, cost_basis_usd, realized_pnl_usd, updated_at
         FROM positions WHERE trading_account_id = $1 AND user_id = $2 ORDER BY updated_at DESC",
    )
    .bind(id)
    .bind(user.id)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
struct ModeBody {
    mode: String,
    confirm_live: Option<bool>,
    confirmation: Option<String>,
}

async fn set_mode(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>, Json(body): Json<ModeBody>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    if !matches!(body.mode.as_str(), "monitoring" | "paper_trading" | "live_trading") {
        return Err(bad("unknown mode"));
    }
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let account = load_account(&mut tx, user.id, id).await?;
    if body.mode == "live_trading" {
        if body.confirm_live != Some(true) || body.confirmation.as_deref() != Some("ENABLE LIVE TRADING") {
            return Err(bad("type ENABLE LIVE TRADING and set confirm_live=true"));
        }
        if !state.cfg.confirm_live_trading {
            return Err(ApiError(StatusCode::FORBIDDEN, "server CONFIRM_LIVE_TRADING is not enabled".into()));
        }
        let has_key = credential_exists(&mut tx, id).await?;
        if !has_key || account.status != "connected" || account.last_exchange_balance_usd.is_none() {
            return Err(bad("connect the account and refresh its balance before live trading"));
        }
        sqlx::query("UPDATE trading_accounts SET operating_mode = 'live_trading', live_confirmed_at = now(), updated_at = now() WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user.id)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
    } else {
        sqlx::query("UPDATE trading_accounts SET operating_mode = $3, updated_at = now() WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user.id)
            .bind(&body.mode)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
    }
    audit(&mut tx, Some(user.id), "mode_changed", &body.mode).await;
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"mode": body.mode})))
}

async fn disconnect_account(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    sqlx::query("UPDATE trading_accounts SET status = 'disconnected', operating_mode = 'monitoring', emergency_stop = true, updated_at = now() WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    sqlx::query("UPDATE copy_strategies SET enabled = false, updated_at = now() WHERE trading_account_id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct RiskBody {
    max_order_usd: Option<Decimal>,
    max_daily_loss_usd: Option<Decimal>,
    max_exposure_usd: Option<Decimal>,
    max_slippage_bps: Option<i32>,
    max_orders_per_interval: Option<i32>,
    interval_seconds: Option<i32>,
    max_stale_data_seconds: Option<i32>,
    paper_cash_usd: Option<Decimal>,
    emergency_stop: Option<bool>,
}

async fn patch_risk(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>, Json(body): Json<RiskBody>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let _ = load_account(&mut tx, user.id, id).await?;
    sqlx::query(
        "UPDATE account_risk_limits SET
            max_order_usd = COALESCE($3, max_order_usd),
            max_daily_loss_usd = COALESCE($4, max_daily_loss_usd),
            max_exposure_usd = COALESCE($5, max_exposure_usd),
            max_slippage_bps = COALESCE($6, max_slippage_bps),
            max_orders_per_interval = COALESCE($7, max_orders_per_interval),
            interval_seconds = COALESCE($8, interval_seconds),
            max_stale_data_seconds = COALESCE($9, max_stale_data_seconds),
            paper_cash_usd = COALESCE($10, paper_cash_usd),
            updated_at = now()
         WHERE trading_account_id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user.id)
    .bind(body.max_order_usd)
    .bind(body.max_daily_loss_usd)
    .bind(body.max_exposure_usd)
    .bind(body.max_slippage_bps)
    .bind(body.max_orders_per_interval)
    .bind(body.interval_seconds)
    .bind(body.max_stale_data_seconds)
    .bind(body.paper_cash_usd)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    if let Some(stop) = body.emergency_stop {
        sqlx::query("UPDATE trading_accounts SET emergency_stop = $3, updated_at = now() WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user.id)
            .bind(stop)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
    }
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

async fn rotate_credentials(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>, Json(body): Json<AccountBody>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let Some(key) = body.private_key.as_deref() else {
        return Err(bad("private_key is required"));
    };
    if !is_private_key(key) {
        return Err(bad("private key must be a 32-byte hex key"));
    }
    let derived = crate::polymarket::live::address_of(key).map_err(|e| bad(e))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let account = load_account(&mut tx, user.id, id).await?;
    if derived != account.signer_address && body.signer_address.is_empty() {
        return Err(bad("private key does not match the stored signer"));
    }
    store_key(&state.vault, &mut tx, user.id, id, key).await.map_err(internal)?;
    if !body.signer_address.is_empty() {
        sqlx::query("UPDATE trading_accounts SET signer_address = $3, funder_address = COALESCE($4, funder_address), signature_type = $5, updated_at = now() WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user.id)
            .bind(normalize_address(&body.signer_address))
            .bind(body.funder_address.map(|v| normalize_address(&v)))
            .bind(&body.signature_type)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
    }
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct TraderBody {
    wallet_address: String,
    alias: String,
}

async fn create_trader(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<TraderBody>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    if !is_eth_address(&body.wallet_address) {
        return Err(bad("wallet address is invalid"));
    }
    let wallet = normalize_address(&body.wallet_address);
    let profile = state.polymarket.profile(&wallet).await;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias, profile, last_trade_at)
         VALUES ($1,$2,$3,$4,$5)
         ON CONFLICT (user_id, wallet_address) DO UPDATE SET alias = EXCLUDED.alias, profile = EXCLUDED.profile, updated_at = now()
         RETURNING id",
    )
    .bind(user.id)
    .bind(&wallet)
    .bind(body.alias.trim())
    .bind(serde_json::to_value(&profile).unwrap_or(json!({})))
    .bind(profile.last_trade_at)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(json!({"id": id, "profile": profile}))))
}

async fn list_traders(State(state): State<AppState>, headers: HeaderMap) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let rows = sqlx::query_as::<_, TraderRow>(
        "SELECT t.id, t.wallet_address, t.alias, t.monitoring_enabled, t.last_trade_at, t.observed_trade_count,
                t.copied_trade_count, t.rejected_trade_count, t.profile,
                COALESCE((SELECT json_agg(json_build_object('account_id', s.trading_account_id, 'strategy_id', s.id, 'enabled', s.enabled))
                          FROM copy_strategies s WHERE s.source_trader_id = t.id), '[]'::json) AS links
         FROM source_traders t WHERE t.user_id = $1 ORDER BY t.created_at DESC",
    )
    .bind(user.id)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

async fn get_trader(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let row = sqlx::query_as::<_, TraderRow>(
        "SELECT t.id, t.wallet_address, t.alias, t.monitoring_enabled, t.last_trade_at, t.observed_trade_count,
                t.copied_trade_count, t.rejected_trade_count, t.profile,
                COALESCE((SELECT json_agg(json_build_object('account_id', s.trading_account_id, 'strategy_id', s.id, 'enabled', s.enabled))
                          FROM copy_strategies s WHERE s.source_trader_id = t.id), '[]'::json) AS links
         FROM source_traders t WHERE t.id = $1 AND t.user_id = $2",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?
    .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "trader not found".into()))?;
    Ok(Json(row))
}

#[derive(Deserialize)]
struct PatchTrader {
    alias: Option<String>,
    monitoring_enabled: Option<bool>,
}

async fn patch_trader(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>, Json(body): Json<PatchTrader>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let updated = sqlx::query(
        "UPDATE source_traders SET alias = COALESCE($3, alias), monitoring_enabled = COALESCE($4, monitoring_enabled), updated_at = now() WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user.id)
    .bind(body.alias)
    .bind(body.monitoring_enabled)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    if updated.rows_affected() == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, "trader not found".into()));
    }
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

async fn delete_trader(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let updated = sqlx::query("DELETE FROM source_traders WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    if updated.rows_affected() == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, "trader not found".into()));
    }
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct StrategyBody {
    trading_account_id: Uuid,
    source_trader_id: Uuid,
    sizing_mode: Option<String>,
    fixed_amount_usd: Option<Decimal>,
    proportion: Option<Decimal>,
    multiplier: Option<Decimal>,
    strategy_budget_usd: Option<Decimal>,
    max_position_usd: Option<Decimal>,
    max_exposure_usd: Option<Decimal>,
    max_open_positions: Option<i32>,
    max_slippage_bps: Option<i32>,
    min_order_usd: Option<Decimal>,
    copy_buy: Option<bool>,
    copy_sell: Option<bool>,
    cooldown_seconds: Option<i32>,
    allowed_markets: Option<Value>,
    enabled: Option<bool>,
}

async fn create_strategy(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<StrategyBody>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mode = body.sizing_mode.unwrap_or_else(|| "fixed_amount".into());
    if !matches!(mode.as_str(), "fixed_amount" | "proportional" | "multiplier") {
        return Err(bad("unknown sizing mode"));
    }
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let _ = load_account(&mut tx, user.id, body.trading_account_id).await?;
    let owns_trader: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM source_traders WHERE id = $1 AND user_id = $2)")
        .bind(body.source_trader_id)
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    if !owns_trader {
        return Err(ApiError(StatusCode::NOT_FOUND, "source trader not found".into()));
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (
            user_id, trading_account_id, source_trader_id, enabled, sizing_mode, fixed_amount_usd, proportion, multiplier,
            strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions, max_slippage_bps, min_order_usd,
            copy_buy, copy_sell, cooldown_seconds, allowed_markets
         ) VALUES ($1,$2,$3,false,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
         RETURNING id",
    )
    .bind(user.id)
    .bind(body.trading_account_id)
    .bind(body.source_trader_id)
    .bind(&mode)
    .bind(body.fixed_amount_usd.or(Some(Decimal::from(10))))
    .bind(body.proportion)
    .bind(body.multiplier)
    .bind(body.strategy_budget_usd.unwrap_or(Decimal::from(100)))
    .bind(body.max_position_usd.unwrap_or(Decimal::from(50)))
    .bind(body.max_exposure_usd.unwrap_or(Decimal::from(100)))
    .bind(body.max_open_positions.unwrap_or(10))
    .bind(body.max_slippage_bps.unwrap_or(150))
    .bind(body.min_order_usd.unwrap_or(Decimal::ONE))
    .bind(body.copy_buy.unwrap_or(true))
    .bind(body.copy_sell.unwrap_or(true))
    .bind(body.cooldown_seconds.unwrap_or(0))
    .bind(body.allowed_markets.unwrap_or_else(|| json!([])))
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| bad("strategy already exists for this account and trader"))?;
    tx.commit().await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(json!({"id": id, "enabled": false}))))
}

async fn list_strategies(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<StrategyFilter>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let rows = sqlx::query_as::<_, StrategyView>(
        "SELECT s.id, s.trading_account_id, s.source_trader_id, t.alias, t.wallet_address, a.name AS account_name,
                s.enabled, s.sizing_mode, s.fixed_amount_usd, s.proportion, s.multiplier, s.strategy_budget_usd,
                s.max_position_usd, s.max_exposure_usd, s.max_open_positions, s.max_slippage_bps, s.min_order_usd,
                s.copy_buy, s.copy_sell, s.cooldown_seconds, s.allowed_markets
         FROM copy_strategies s
         JOIN source_traders t ON t.id = s.source_trader_id
         JOIN trading_accounts a ON a.id = s.trading_account_id
         WHERE s.user_id = $1
           AND ($2::uuid IS NULL OR s.trading_account_id = $2)
           AND ($3::uuid IS NULL OR s.source_trader_id = $3)
         ORDER BY s.created_at DESC",
    )
    .bind(user.id)
    .bind(q.account_id)
    .bind(q.trader_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

async fn get_strategy(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let row = one_strategy(&mut tx, user.id, id).await?;
    Ok(Json(row))
}

async fn patch_strategy(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>, Json(body): Json<StrategyBody>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let _ = one_strategy(&mut tx, user.id, id).await?;
    sqlx::query(
        "UPDATE copy_strategies SET
            sizing_mode = COALESCE($3, sizing_mode),
            fixed_amount_usd = COALESCE($4, fixed_amount_usd),
            proportion = COALESCE($5, proportion),
            multiplier = COALESCE($6, multiplier),
            strategy_budget_usd = COALESCE($7, strategy_budget_usd),
            max_position_usd = COALESCE($8, max_position_usd),
            max_exposure_usd = COALESCE($9, max_exposure_usd),
            max_open_positions = COALESCE($10, max_open_positions),
            max_slippage_bps = COALESCE($11, max_slippage_bps),
            min_order_usd = COALESCE($12, min_order_usd),
            copy_buy = COALESCE($13, copy_buy),
            copy_sell = COALESCE($14, copy_sell),
            cooldown_seconds = COALESCE($15, cooldown_seconds),
            allowed_markets = COALESCE($16, allowed_markets),
            enabled = COALESCE($17, enabled),
            updated_at = now()
         WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user.id)
    .bind(body.sizing_mode)
    .bind(body.fixed_amount_usd)
    .bind(body.proportion)
    .bind(body.multiplier)
    .bind(body.strategy_budget_usd)
    .bind(body.max_position_usd)
    .bind(body.max_exposure_usd)
    .bind(body.max_open_positions)
    .bind(body.max_slippage_bps)
    .bind(body.min_order_usd)
    .bind(body.copy_buy)
    .bind(body.copy_sell)
    .bind(body.cooldown_seconds)
    .bind(body.allowed_markets)
    .bind(body.enabled)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct EnableBody {
    enabled: bool,
}

async fn enable_strategy(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>, Json(body): Json<EnableBody>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    sqlx::query("UPDATE copy_strategies SET enabled = $3, updated_at = now() WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .bind(body.enabled)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"enabled": body.enabled})))
}

async fn delete_strategy(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    sqlx::query("DELETE FROM copy_strategies WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

async fn list_activity(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<PageQuery>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let rows = sqlx::query_as::<_, ActivityRow>(
        "SELECT id, kind, message, at, source_event_id, decision_id, order_id
         FROM timeline_events WHERE user_id = $1 ORDER BY at DESC LIMIT $2 OFFSET $3",
    )
    .bind(user.id)
    .bind(q.limit())
    .bind(q.offset.unwrap_or(0))
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

async fn list_source_trades(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<PageQuery>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let rows = sqlx::query_as::<_, TradeRow>(
        "SELECT e.id, e.source_wallet, e.tx_hash, e.token_id, e.condition_id, e.outcome, e.market_title, e.market_slug,
                e.event_title, e.side, e.price, e.size_shares, e.notional_usd, e.event_ts, e.origin, e.status
         FROM source_events e
         WHERE EXISTS (
            SELECT 1 FROM source_traders t
            WHERE t.user_id = $1 AND lower(t.wallet_address) = lower(e.source_wallet)
         )
         AND ($2::text IS NULL OR e.side = $2)
         AND ($3::text IS NULL OR e.source_wallet = $3 OR e.token_id = $3 OR e.tx_hash = $3 OR e.market_title ILIKE '%' || $3 || '%')
         ORDER BY e.event_ts DESC
         LIMIT $4 OFFSET $5",
    )
    .bind(user.id)
    .bind(q.side.as_deref().map(|s| s.to_ascii_lowercase()))
    .bind(q.q.as_ref().map(|s| s.to_ascii_lowercase()))
    .bind(q.limit())
    .bind(q.offset.unwrap_or(0))
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

async fn list_decisions(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<PageQuery>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let rows = sqlx::query_as::<_, DecisionRow>(
        "SELECT id, trading_account_id, source_event_id, strategy_id, status, reason, operating_mode, side,
                target_notional_usd, target_shares, limit_price, book_price, available_balance_usd, decided_at
         FROM copy_decisions
         WHERE user_id = $1
           AND ($2::uuid IS NULL OR trading_account_id = $2)
           AND ($3::text IS NULL OR status = $3)
           AND ($4::text IS NULL OR side = $4)
           AND ($5::text IS NULL OR operating_mode = $5)
         ORDER BY decided_at DESC LIMIT $6 OFFSET $7",
    )
    .bind(user.id)
    .bind(q.account_id)
    .bind(q.status.clone())
    .bind(q.side.as_deref().map(|s| s.to_ascii_lowercase()))
    .bind(q.mode.clone())
    .bind(q.limit())
    .bind(q.offset.unwrap_or(0))
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

async fn list_orders(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<PageQuery>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let rows = sqlx::query_as::<_, OrderRow>(
        "SELECT id, trading_account_id, decision_id, source_event_id, exchange_order_id, operating_mode, order_type, side,
                token_id, limit_price, size_shares, status, error, created_at, submitted_at, updated_at
         FROM orders WHERE user_id = $1
           AND ($2::uuid IS NULL OR trading_account_id = $2)
           AND ($3::text IS NULL OR status = $3)
           AND ($4::text IS NULL OR side = $4)
           AND ($5::text IS NULL OR operating_mode = $5)
         ORDER BY created_at DESC LIMIT $6 OFFSET $7",
    )
    .bind(user.id)
    .bind(q.account_id)
    .bind(q.status.clone())
    .bind(q.side.as_deref().map(|s| s.to_ascii_lowercase()))
    .bind(q.mode.clone())
    .bind(q.limit())
    .bind(q.offset.unwrap_or(0))
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

async fn list_fills(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<PageQuery>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let rows = sqlx::query_as::<_, FillRow>(
        "SELECT id, order_id, trading_account_id, price, shares, notional_usd, fee_usd, fee_status, tx_hash,
                slippage_vs_source_bps, slippage_vs_limit_bps, filled_at
         FROM fills WHERE user_id = $1 AND ($2::uuid IS NULL OR trading_account_id = $2)
         ORDER BY filled_at DESC LIMIT $3 OFFSET $4",
    )
    .bind(user.id)
    .bind(q.account_id)
    .bind(q.limit())
    .bind(q.offset.unwrap_or(0))
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

async fn list_positions(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<PageQuery>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let rows = sqlx::query_as::<_, PositionView>(
        "SELECT id, source_wallet, token_id, market_title, outcome, operating_mode, open_shares, avg_price, cost_basis_usd, realized_pnl_usd, updated_at
         FROM positions WHERE user_id = $1 AND open_shares > 0 AND ($2::uuid IS NULL OR trading_account_id = $2)
         ORDER BY updated_at DESC LIMIT $3 OFFSET $4",
    )
    .bind(user.id)
    .bind(q.account_id)
    .bind(q.limit())
    .bind(q.offset.unwrap_or(0))
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

async fn list_closed(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<PageQuery>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let rows = sqlx::query_as::<_, ClosedRow>(
        "SELECT id, trading_account_id, source_wallet, token_id, market_title, operating_mode, closed_shares, realized_pnl_usd, closed_at
         FROM closed_positions WHERE user_id = $1 ORDER BY closed_at DESC LIMIT $2 OFFSET $3",
    )
    .bind(user.id)
    .bind(q.limit())
    .bind(q.offset.unwrap_or(0))
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

async fn trade_detail(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user").await.map_err(internal)?;
    let trade = sqlx::query_as::<_, TradeRow>(
        "SELECT e.id, e.source_wallet, e.tx_hash, e.token_id, e.condition_id, e.outcome, e.market_title, e.market_slug,
                e.event_title, e.side, e.price, e.size_shares, e.notional_usd, e.event_ts, e.origin, e.status
         FROM source_events e
         WHERE e.id = $1 AND EXISTS (SELECT 1 FROM source_traders t WHERE t.user_id = $2 AND lower(t.wallet_address) = lower(e.source_wallet))",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?
    .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "trade not found".into()))?;
    let decisions = sqlx::query_as::<_, DecisionRow>(
        "SELECT id, trading_account_id, source_event_id, strategy_id, status, reason, operating_mode, side,
                target_notional_usd, target_shares, limit_price, book_price, available_balance_usd, decided_at
         FROM copy_decisions WHERE source_event_id = $1 AND user_id = $2 ORDER BY decided_at",
    )
    .bind(id)
    .bind(user.id)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    let orders = sqlx::query_as::<_, OrderRow>(
        "SELECT id, trading_account_id, decision_id, source_event_id, exchange_order_id, operating_mode, order_type, side,
                token_id, limit_price, size_shares, status, error, created_at, submitted_at, updated_at
         FROM orders WHERE source_event_id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user.id)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    let fills = sqlx::query_as::<_, FillRow>(
        "SELECT f.id, f.order_id, f.trading_account_id, f.price, f.shares, f.notional_usd, f.fee_usd, f.fee_status, f.tx_hash,
                f.slippage_vs_source_bps, f.slippage_vs_limit_bps, f.filled_at
         FROM fills f WHERE f.user_id = $1 AND f.order_id IN (SELECT id FROM orders WHERE source_event_id = $2 AND user_id = $1)",
    )
    .bind(user.id)
    .bind(id)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    let timeline = sqlx::query_as::<_, ActivityRow>(
        "SELECT id, kind, message, at, source_event_id, decision_id, order_id FROM timeline_events
         WHERE user_id = $1 AND source_event_id = $2 ORDER BY at",
    )
    .bind(user.id)
    .bind(id)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    let polygonscan = trade.tx_hash.as_ref().map(|tx| format!("https://polygonscan.com/tx/{tx}"));
    let market_url = trade.market_slug.as_ref().map(|slug| format!("https://polymarket.com/market/{slug}"));
    let sizing = sizing_breakdown(&decisions, &orders, &fills);
    Ok(Json(json!({
        "trade": trade,
        "decisions": decisions,
        "orders": orders,
        "fills": fills,
        "sizing": sizing,
        "timeline": timeline,
        "links": {"polygonscan": polygonscan, "polymarket": market_url}
    })))
}

fn sizing_breakdown(decisions: &[DecisionRow], orders: &[OrderRow], fills: &[FillRow]) -> Vec<serde_json::Value> {
    decisions.iter().map(|decision| {
        let order = orders.iter().find(|order| order.decision_id == decision.id);
        let fill = order.and_then(|order| fills.iter().find(|fill| fill.order_id == order.id));
        let requested = decision.target_notional_usd;
        let actual = fill.map(|fill| fill.notional_usd);
        let reserved = match (decision.target_shares, decision.limit_price) {
            (Some(shares), Some(limit)) => Some(shares * limit),
            _ => None,
        };
        json!({
            "decision_id": decision.id,
            "trading_account_id": decision.trading_account_id,
            "requested_notional_usd": requested,
            "reserved_notional_usd": reserved,
            "actual_notional_usd": actual,
            "notional_difference_usd": match (requested, actual) {
                (Some(requested), Some(actual)) => Some(requested - actual),
                _ => None,
            },
            "estimated_fee_usd": fill.and_then(|fill| if fill.fee_status == "estimated" { fill.fee_usd } else { None }),
            "actual_fee_usd": fill.and_then(|fill| if fill.fee_status == "actual" { fill.fee_usd } else { None }),
            "fee_status": fill.map(|fill| fill.fee_status.clone())
        })
    }).collect()
}

async fn ws_upgrade(State(state): State<AppState>, ws: WebSocketUpgrade, Query(q): Query<WsQuery>) -> Result<impl IntoResponse, ApiError> {
    let claims = auth::read_access(&state.cfg.jwt_secret, &q.token).map_err(|_| ApiError(StatusCode::UNAUTHORIZED, "invalid token".into()))?;
    let mut rx = state.activity.subscribe();
    Ok(ws.on_upgrade(move |socket| async move {
        let _ = pump(socket, &mut rx, claims.sub).await;
    }))
}

async fn pump(mut socket: WebSocket, rx: &mut broadcast::Receiver<String>, user_id: Uuid) {
    loop {
        match rx.recv().await {
            Ok(payload) => {
                let Ok(value) = serde_json::from_str::<Value>(&payload) else { continue };
                if value.get("user_id").and_then(|v| v.as_str()) == Some(user_id.to_string().as_str()) {
                    if socket.send(Message::Text(payload.into())).await.is_err() {
                        break;
                    }
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
}

fn require_user(state: &AppState, headers: &HeaderMap, role: Option<&str>) -> Result<AuthUser, ApiError> {
    let claims = bearer_user(state, headers).ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "authentication required".into()))?;
    if let Some(role) = role {
        if claims.role != role {
            return Err(ApiError(StatusCode::FORBIDDEN, "wrong role for this application".into()));
        }
    }
    Ok(AuthUser {
        id: claims.sub,
        email: String::new(),
        display_name: String::new(),
        role: claims.role,
        status: "active".into(),
    })
}

fn bearer_user(state: &AppState, headers: &HeaderMap) -> Option<crate::auth::AccessClaims> {
    let header = headers.get("authorization")?.to_str().ok()?;
    let token = header.strip_prefix("Bearer ")?;
    auth::read_access(&state.cfg.jwt_secret, token).ok()
}

fn user_json(user: &AuthUser) -> Value {
    json!({
        "id": user.id,
        "email": user.email,
        "display_name": user.display_name,
        "role": user.role,
        "status": user.status
    })
}

fn refresh_cookie(token: &str, _ttl: i64) -> Cookie<'static> {
    Cookie::build(("pom_refresh", token.to_string()))
        .http_only(true)
        .path("/")
        .same_site(axum_extra::extract::cookie::SameSite::Lax)
        .build()
}

fn rate_limited(state: &AppState, ip: &str) -> bool {
    rate_limited_map(&state.login_attempts, ip, 10, 60)
}

fn rate_limited_map(
    store: &std::sync::Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    key: &str,
    max: usize,
    window_secs: u64,
) -> bool {
    let mut map = store.lock().expect("rate limit lock");
    let now = Instant::now();
    let entry = map.entry(key.to_string()).or_default();
    entry.retain(|at| now.duration_since(*at) < Duration::from_secs(window_secs));
    if entry.len() >= max {
        return true;
    }
    entry.push(now);
    false
}

fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("local")
        .split(',')
        .next()
        .unwrap_or("local")
        .trim()
        .to_string()
}

async fn issue_reset_token(
    tx: &mut crate::db::Tx<'_>,
    user_id: Uuid,
    ttl_minutes: i64,
) -> Result<String, ApiError> {
    let token = random_token();
    sqlx::query(
        "INSERT INTO invitation_tokens (user_id, token_hash, purpose, expires_at) VALUES ($1,$2,'reset', now() + ($3::int * interval '1 minute'))",
    )
    .bind(user_id)
    .bind(sha256_hex(&token))
    .bind(ttl_minutes as i32)
    .execute(&mut **tx)
    .await
    .map_err(internal)?;
    Ok(token)
}

fn dispatch_invitation_email(mail: Email, email: String, display_name: String, token: String) {
    tokio::spawn(async move {
        if let Err(err) = mail.send_invitation(&email, &display_name, &token).await {
            Email::log_failure(&err);
        }
    });
}

async fn invitation_delivery(mail: &Email, to_email: &str, display_name: &str, token: &str) -> Value {
    if !mail.enabled() {
        return json!({
            "configured": false,
            "sent": false,
            "detail": MailError::NotConfigured.public_message(),
        });
    }
    match mail.send_invitation(to_email, display_name, token).await {
        Ok(()) => {
            tracing::info!(to = %to_email, "invitation email sent");
            json!({
                "configured": true,
                "sent": true,
                "detail": format!("Invitation email sent to {to_email}."),
            })
        }
        Err(err) => {
            Email::log_failure(&err);
            json!({
                "configured": true,
                "sent": false,
                "detail": err.public_message(),
            })
        }
    }
}

fn dispatch_password_recovery_email(mail: Email, email: String, display_name: String, token: String) {
    tokio::spawn(async move {
        if let Err(err) = mail.send_password_recovery(&email, &display_name, &token).await {
            Email::log_failure(&err);
        }
    });
}

fn dispatch_email_verification(mail: Email, email: String, display_name: String, token: String) {
    tokio::spawn(async move {
        if let Err(err) = mail.send_email_verification(&email, &display_name, &token).await {
            Email::log_failure(&err);
        }
    });
}

fn dispatch_password_changed_email(mail: Email, email: String, display_name: String) {
    tokio::spawn(async move {
        if let Err(err) = mail.send_password_changed(&email, &display_name).await {
            Email::log_failure(&err);
        }
    });
}

fn dispatch_security_lock_email(mail: Email, alert: auth::LoginSecurityAlert) {
    tokio::spawn(async move {
        if let Err(err) = mail
            .send_security_alert(
                &alert.email,
                &alert.display_name,
                "Your account was temporarily locked after several failed sign-in attempts.",
            )
            .await
        {
            Email::log_failure(&err);
        }
    });
}

fn validate_signature(value: &str) -> Result<(), ApiError> {
    if matches!(value, "eoa" | "poly_proxy" | "poly_gnosis_safe" | "poly_1271") {
        Ok(())
    } else {
        Err(bad("signature_type must be eoa, poly_proxy, poly_gnosis_safe, or poly_1271"))
    }
}

async fn store_key(vault: &Vault, tx: &mut crate::db::Tx<'_>, user_id: Uuid, account_id: Uuid, private_key: &str) -> anyhow::Result<()> {
    let aad = credential_aad(user_id, account_id);
    let payload = json!({"private_key": private_key}).to_string();
    let blob = vault.encrypt(payload.as_bytes(), &aad)?;
    let hint = private_key.chars().rev().take(4).collect::<String>().chars().rev().collect::<String>();
    sqlx::query(
        "INSERT INTO trading_credentials (trading_account_id, user_id, ciphertext, nonce, key_id, aad, api_key_hint)
         VALUES ($1,$2,$3,$4,$5,$6,$7)
         ON CONFLICT (trading_account_id) DO UPDATE SET ciphertext = EXCLUDED.ciphertext, nonce = EXCLUDED.nonce, key_id = EXCLUDED.key_id, aad = EXCLUDED.aad, api_key_hint = EXCLUDED.api_key_hint, updated_at = now()",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(blob.ciphertext)
    .bind(blob.nonce)
    .bind(blob.key_id)
    .bind(blob.aad)
    .bind(hint)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn load_secret(vault: &Vault, tx: &mut crate::db::Tx<'_>, user_id: Uuid, account_id: Uuid) -> Result<Option<Value>, ApiError> {
    let row = sqlx::query_as::<_, CredRow>("SELECT ciphertext, nonce, key_id, aad FROM trading_credentials WHERE trading_account_id = $1 AND user_id = $2")
        .bind(account_id)
        .bind(user_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(internal)?;
    let Some(row) = row else { return Ok(None) };
    let plain = vault
        .decrypt(&SecretBlob { ciphertext: row.ciphertext, nonce: row.nonce, key_id: row.key_id, aad: row.aad })
        .map_err(|_| ApiError(StatusCode::INTERNAL_SERVER_ERROR, "stored credentials could not be decrypted".into()))?;
    Ok(Some(serde_json::from_slice(&plain).unwrap_or(json!({}))))
}

async fn credential_exists(tx: &mut crate::db::Tx<'_>, account_id: Uuid) -> Result<bool, ApiError> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM trading_credentials WHERE trading_account_id = $1)")
        .bind(account_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(internal)
}

async fn load_account(tx: &mut crate::db::Tx<'_>, user_id: Uuid, id: Uuid) -> Result<AccountCard, ApiError> {
    sqlx::query_as::<_, AccountCard>(
        "SELECT id, name, signer_address, funder_address, signature_type, operating_mode, status, emergency_stop, last_exchange_balance_usd, last_balance_checked_at, last_connection_error
         FROM trading_accounts WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(internal)?
    .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "account not found".into()))
}

async fn one_strategy(tx: &mut crate::db::Tx<'_>, user_id: Uuid, id: Uuid) -> Result<StrategyView, ApiError> {
    sqlx::query_as::<_, StrategyView>(
        "SELECT s.id, s.trading_account_id, s.source_trader_id, t.alias, t.wallet_address, a.name AS account_name,
                s.enabled, s.sizing_mode, s.fixed_amount_usd, s.proportion, s.multiplier, s.strategy_budget_usd,
                s.max_position_usd, s.max_exposure_usd, s.max_open_positions, s.max_slippage_bps, s.min_order_usd,
                s.copy_buy, s.copy_sell, s.cooldown_seconds, s.allowed_markets
         FROM copy_strategies s
         JOIN source_traders t ON t.id = s.source_trader_id
         JOIN trading_accounts a ON a.id = s.trading_account_id
         WHERE s.id = $1 AND s.user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(internal)?
    .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "strategy not found".into()))
}

async fn audit(tx: &mut crate::db::Tx<'_>, actor: Option<Uuid>, action: &str, target: &str) {
    let _ = sqlx::query("INSERT INTO audit_log (actor_user_id, action, target) VALUES ($1,$2,$3)")
        .bind(actor)
        .bind(action)
        .bind(target)
        .execute(&mut **tx)
        .await;
}

fn internal(err: impl std::fmt::Display) -> ApiError {
    tracing::error!(error = %crate::security::redact(&err.to_string()), "request failed");
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, "request failed".into())
}

#[derive(Deserialize)]
struct PageQuery {
    limit: Option<i64>,
    offset: Option<i64>,
    q: Option<String>,
    side: Option<String>,
    status: Option<String>,
    mode: Option<String>,
    account_id: Option<Uuid>,
}

impl PageQuery {
    fn limit(&self) -> i64 {
        self.limit.unwrap_or(50).clamp(1, 200)
    }
}

#[derive(Deserialize)]
struct StrategyFilter {
    account_id: Option<Uuid>,
    trader_id: Option<Uuid>,
}

#[derive(Deserialize)]
struct WsQuery {
    token: String,
}

#[derive(sqlx::FromRow, Serialize)]
struct UserList {
    id: Uuid,
    email: String,
    display_name: String,
    role: String,
    status: String,
    created_at: chrono::DateTime<Utc>,
    last_login_at: Option<chrono::DateTime<Utc>>,
    accounts: i64,
    traders: i64,
}

#[derive(sqlx::FromRow, Serialize)]
struct SettingRow {
    key: String,
    value: Value,
    updated_at: chrono::DateTime<Utc>,
}

#[derive(sqlx::FromRow, Serialize)]
struct SystemEventRow {
    id: Uuid,
    severity: String,
    component: String,
    message: String,
    created_at: chrono::DateTime<Utc>,
}

#[derive(sqlx::FromRow, Serialize)]
struct AccountCard {
    id: Uuid,
    name: String,
    signer_address: String,
    funder_address: String,
    signature_type: String,
    operating_mode: String,
    status: String,
    emergency_stop: bool,
    last_exchange_balance_usd: Option<Decimal>,
    last_balance_checked_at: Option<chrono::DateTime<Utc>>,
    last_connection_error: Option<String>,
}

#[derive(sqlx::FromRow, Serialize)]
struct RiskRow {
    trading_account_id: Uuid,
    user_id: Uuid,
    max_order_usd: Decimal,
    max_daily_loss_usd: Decimal,
    max_exposure_usd: Decimal,
    max_slippage_bps: i32,
    max_orders_per_interval: i32,
    interval_seconds: i32,
    max_stale_data_seconds: i32,
    paper_cash_usd: Decimal,
}

#[derive(sqlx::FromRow, Serialize)]
struct TraderRow {
    id: Uuid,
    wallet_address: String,
    alias: String,
    monitoring_enabled: bool,
    last_trade_at: Option<chrono::DateTime<Utc>>,
    observed_trade_count: i32,
    copied_trade_count: i32,
    rejected_trade_count: i32,
    profile: Value,
    links: Value,
}

#[derive(sqlx::FromRow, Serialize)]
struct StrategyView {
    id: Uuid,
    trading_account_id: Uuid,
    source_trader_id: Uuid,
    alias: String,
    wallet_address: String,
    account_name: String,
    enabled: bool,
    sizing_mode: String,
    fixed_amount_usd: Option<Decimal>,
    proportion: Option<Decimal>,
    multiplier: Option<Decimal>,
    strategy_budget_usd: Decimal,
    max_position_usd: Decimal,
    max_exposure_usd: Decimal,
    max_open_positions: i32,
    max_slippage_bps: i32,
    min_order_usd: Decimal,
    copy_buy: bool,
    copy_sell: bool,
    cooldown_seconds: i32,
    allowed_markets: Value,
}

#[derive(sqlx::FromRow, Serialize)]
struct ActivityRow {
    id: Uuid,
    kind: String,
    message: String,
    at: chrono::DateTime<Utc>,
    source_event_id: Option<Uuid>,
    decision_id: Option<Uuid>,
    order_id: Option<Uuid>,
}

#[derive(sqlx::FromRow, Serialize)]
struct TradeRow {
    id: Uuid,
    source_wallet: String,
    tx_hash: Option<String>,
    token_id: String,
    condition_id: Option<String>,
    outcome: Option<String>,
    market_title: Option<String>,
    market_slug: Option<String>,
    event_title: Option<String>,
    side: String,
    price: Decimal,
    size_shares: Decimal,
    notional_usd: Decimal,
    event_ts: chrono::DateTime<Utc>,
    origin: String,
    status: String,
}

#[derive(sqlx::FromRow, Serialize)]
struct DecisionRow {
    id: Uuid,
    trading_account_id: Uuid,
    source_event_id: Uuid,
    strategy_id: Uuid,
    status: String,
    reason: String,
    operating_mode: String,
    side: String,
    target_notional_usd: Option<Decimal>,
    target_shares: Option<Decimal>,
    limit_price: Option<Decimal>,
    book_price: Option<Decimal>,
    available_balance_usd: Option<Decimal>,
    decided_at: chrono::DateTime<Utc>,
}

#[derive(sqlx::FromRow, Serialize)]
struct OrderRow {
    id: Uuid,
    trading_account_id: Uuid,
    decision_id: Uuid,
    source_event_id: Uuid,
    exchange_order_id: Option<String>,
    operating_mode: String,
    order_type: String,
    side: String,
    token_id: String,
    limit_price: Decimal,
    size_shares: Decimal,
    status: String,
    error: Option<String>,
    created_at: chrono::DateTime<Utc>,
    submitted_at: Option<chrono::DateTime<Utc>>,
    updated_at: chrono::DateTime<Utc>,
}

#[derive(sqlx::FromRow, Serialize)]
struct FillRow {
    id: Uuid,
    order_id: Uuid,
    trading_account_id: Uuid,
    price: Decimal,
    shares: Decimal,
    notional_usd: Decimal,
    fee_usd: Option<Decimal>,
    fee_status: String,
    tx_hash: Option<String>,
    slippage_vs_source_bps: Option<i32>,
    slippage_vs_limit_bps: Option<i32>,
    filled_at: chrono::DateTime<Utc>,
}

#[derive(sqlx::FromRow, Serialize)]
struct PositionView {
    id: Uuid,
    source_wallet: String,
    token_id: String,
    market_title: Option<String>,
    outcome: Option<String>,
    operating_mode: String,
    open_shares: Decimal,
    avg_price: Decimal,
    cost_basis_usd: Decimal,
    realized_pnl_usd: Decimal,
    updated_at: chrono::DateTime<Utc>,
}

#[derive(sqlx::FromRow, Serialize)]
struct ClosedRow {
    id: Uuid,
    trading_account_id: Uuid,
    source_wallet: String,
    token_id: String,
    market_title: Option<String>,
    operating_mode: String,
    closed_shares: Decimal,
    realized_pnl_usd: Decimal,
    closed_at: chrono::DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct UserProfileRow {
    email: String,
    display_name: String,
}

#[derive(sqlx::FromRow)]
struct VerifyUserRow {
    email: String,
    display_name: String,
    email_verified_at: Option<chrono::DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct CredRow {
    ciphertext: Vec<u8>,
    nonce: Vec<u8>,
    key_id: String,
    aad: String,
}
