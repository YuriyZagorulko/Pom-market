use std::net::SocketAddr;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{ConnectInfo, Path, Query, State, WebSocketUpgrade};
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
use crate::client_ip::{self, TrustedProxies};
use crate::config::Config;
use crate::copy_preview::{
    market_allowed_list, percent_to_proportion, simulate_copy_preview, CopyPreviewParams,
    PreviewEvent,
};
use crate::db::begin_as;
use crate::email::{Email, MailError};
use crate::engine::{is_eth_address, is_private_key, normalize_address};
use crate::polymarket::http::PublicClient;
use crate::polymarket::live::probe_account;
use crate::rate_limit::{self, RateLimitStore};
use crate::security::{
    credential_aad, hash_password, random_token, sha256_hex, verify_password, SecretBlob, Vault,
};
use crate::strategy_budget::utilization_for_user;

mod account_control;
mod page;
use page::{ListQuery, Paged};
type PageQuery = ListQuery;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub cfg: Config,
    pub vault: Vault,
    pub polymarket: PublicClient,
    pub activity: broadcast::Sender<String>,
    pub trusted_proxies: TrustedProxies,
    pub login_limits: RateLimitStore,
    pub recovery_limits: RateLimitStore,
    pub verify_limits: RateLimitStore,
    pub refresh_limits: RateLimitStore,
    pub mail: Email,
}

const LOGIN_FAIL_MAX: usize = 10;
const LOGIN_FAIL_WINDOW_SECS: u64 = 60;
const REFRESH_MAX: usize = 120;
const REFRESH_WINDOW_SECS: u64 = 60;
const RECOVERY_IP_MAX: usize = 5;
const RECOVERY_IP_WINDOW_SECS: u64 = 60;
const RECOVERY_EMAIL_MAX: usize = 5;
const RECOVERY_EMAIL_WINDOW_SECS: u64 = 3600;
const VERIFY_MAX: usize = 3;
const VERIFY_WINDOW_SECS: u64 = 60;

pub fn new_app_state(
    pool: PgPool,
    cfg: Config,
    vault: Vault,
    polymarket: PublicClient,
    activity: broadcast::Sender<String>,
    mail: Email,
) -> AppState {
    let trusted_proxies = cfg.trusted_proxies_parsed.clone();
    AppState {
        pool,
        cfg,
        vault,
        polymarket,
        activity,
        trusted_proxies,
        login_limits: RateLimitStore::default(),
        recovery_limits: RateLimitStore::default(),
        verify_limits: RateLimitStore::default(),
        refresh_limits: RateLimitStore::default(),
        mail,
    }
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
        .route(
            "/api/v1/auth/password/recovery/request",
            post(forgot_password),
        )
        .route("/api/v1/auth/password/reset/confirm", post(password_setup))
        .route("/api/auth/forgot-password", post(forgot_password))
        .route("/api/auth/reset-password", post(reset_password))
        .route(
            "/api/v1/auth/email/verify/request",
            post(verify_email_request),
        )
        .route(
            "/api/v1/auth/email/verify/confirm",
            post(verify_email_confirm),
        )
        .route("/api/v1/admin/overview", get(admin_overview))
        .route(
            "/api/v1/admin/users",
            get(admin_users).post(admin_create_user),
        )
        .route(
            "/api/v1/admin/users/{id}",
            get(admin_user).patch(admin_update_user),
        )
        .route("/api/v1/admin/users/{id}/suspend", post(admin_suspend))
        .route("/api/v1/admin/users/{id}/activate", post(admin_activate))
        .route("/api/v1/admin/users/{id}/invite", post(admin_invite))
        .route("/api/v1/admin/health", get(admin_health))
        .route("/api/v1/admin/events", get(admin_events))
        .route("/api/v1/admin/settings", get(admin_settings))
        .route("/api/v1/admin/kill-switch", post(admin_kill))
        .route("/api/v1/dashboard", get(dashboard))
        .route("/api/v1/accounts", get(list_accounts).post(create_account))
        .route(
            "/api/v1/accounts/{id}",
            get(get_account).patch(patch_account).delete(delete_account),
        )
        .route("/api/v1/accounts/{id}/test", post(test_account))
        .route("/api/v1/accounts/{id}/balance", get(account_balance))
        .route("/api/v1/accounts/{id}/positions", get(account_positions))
        .route("/api/v1/accounts/{id}/mode", post(set_mode))
        .route("/api/v1/accounts/{id}/disconnect", post(disconnect_account))
        .route("/api/v1/accounts/{id}/risk", patch(patch_risk))
        .route(
            "/api/v1/accounts/{id}/trading-control",
            get(account_control::get_trading_control),
        )
        .route(
            "/api/v1/accounts/{id}/trading-control/kill-switch/activate",
            post(account_control::activate_kill_switch),
        )
        .route(
            "/api/v1/accounts/{id}/trading-control/kill-switch/deactivate",
            post(account_control::deactivate_kill_switch),
        )
        .route(
            "/api/v1/accounts/{id}/trading-control/soft-close/activate",
            post(account_control::activate_soft_close),
        )
        .route(
            "/api/v1/accounts/{id}/trading-control/soft-close/deactivate",
            post(account_control::deactivate_soft_close),
        )
        .route(
            "/api/v1/accounts/{id}/trading-control/resume",
            post(account_control::resume_trading),
        )
        .route(
            "/api/v1/accounts/{id}/hard-close/preview",
            get(account_control::hard_close_preview),
        )
        .route(
            "/api/v1/accounts/{id}/hard-close",
            post(account_control::start_hard_close_route),
        )
        .route(
            "/api/v1/accounts/{id}/hard-close/{op_id}",
            get(account_control::get_hard_close),
        )
        .route(
            "/api/v1/accounts/{id}/hard-close/{op_id}/retry",
            post(account_control::retry_hard_close),
        )
        .route(
            "/api/v1/accounts/{id}/credentials/rotate",
            post(rotate_credentials),
        )
        .route(
            "/api/v1/source-traders",
            get(list_traders).post(create_trader),
        )
        .route(
            "/api/v1/source-traders/{id}",
            get(get_trader).patch(patch_trader).delete(delete_trader),
        )
        .route(
            "/api/v1/strategies",
            get(list_strategies).post(create_strategy),
        )
        .route(
            "/api/v1/strategies/{id}",
            get(get_strategy)
                .patch(patch_strategy)
                .delete(delete_strategy),
        )
        .route("/api/v1/strategies/{id}/enable", post(enable_strategy))
        .route(
            "/api/v1/strategies/{id}/utilization",
            get(strategy_utilization),
        )
        .route(
            "/api/v1/source-traders/{id}/copy-preview",
            post(copy_preview),
        )
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

fn database_name_from_url(url: &str) -> String {
    url.rsplit('/')
        .next()
        .unwrap_or("unknown")
        .split('?')
        .next()
        .unwrap_or("unknown")
        .to_string()
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let db = sqlx::query("SELECT 1").execute(&state.pool).await.is_ok();
    let db_name = database_name_from_url(&state.cfg.app_database_url);
    Json(json!({
        "status": if db { "ok" } else { "degraded" },
        "database": db,
        "database_name": db_name,
    }))
}

async fn openapi() -> impl IntoResponse {
    Json(
        serde_json::from_str::<Value>(include_str!("openapi.json"))
            .unwrap_or_else(|_| json!({"openapi": "3.0.3"})),
    )
}

#[derive(Deserialize)]
struct LoginBody {
    email: String,
    password: String,
}

async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<LoginBody>,
) -> Result<impl IntoResponse, ApiError> {
    let ip = client_ip::resolve_client_ip(peer, &headers, &state.trusted_proxies);
    let email_key = rate_limit::login_email_key(&body.email);
    if state
        .login_limits
        .is_blocked(&ip, LOGIN_FAIL_MAX, LOGIN_FAIL_WINDOW_SECS)
        || state
            .login_limits
            .is_blocked(&email_key, LOGIN_FAIL_MAX, LOGIN_FAIL_WINDOW_SECS)
    {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "too many login attempts".into(),
        ));
    }
    let stored_ip = ip.parse::<std::net::IpAddr>().ok().map(|_| ip.clone());
    let attempt = auth::login(
        &state.pool,
        &state.cfg.jwt_secret,
        &body.email,
        &body.password,
        headers
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        stored_ip,
        state.cfg.access_ttl_seconds,
        state.cfg.refresh_ttl_seconds,
    )
    .await;
    if let Some(alert) = attempt.security_alert {
        dispatch_security_lock_email(state.mail.clone(), alert);
    }
    if attempt.result.is_err() {
        state
            .login_limits
            .record_failure(&ip, LOGIN_FAIL_MAX, LOGIN_FAIL_WINDOW_SECS);
        state
            .login_limits
            .record_failure(&email_key, LOGIN_FAIL_MAX, LOGIN_FAIL_WINDOW_SECS);
    }
    let result = attempt
        .result
        .map_err(|message| ApiError(StatusCode::UNAUTHORIZED, message.into()))?;
    let jar = jar.add(refresh_cookie(
        &result.refresh_token,
        state.cfg.refresh_ttl_seconds,
    ));
    Ok((
        jar,
        Json(json!({
            "access_token": result.access_token,
            "token_type": "Bearer",
            "user": user_json(&result.user)
        })),
    ))
}

async fn refresh(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<impl IntoResponse, ApiError> {
    let ip = client_ip::resolve_client_ip(peer, &headers, &state.trusted_proxies);
    if state
        .refresh_limits
        .is_blocked(&ip, REFRESH_MAX, REFRESH_WINDOW_SECS)
    {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "too many refresh attempts".into(),
        ));
    }
    state
        .refresh_limits
        .record_attempt(&ip, REFRESH_MAX, REFRESH_WINDOW_SECS);
    let token = jar
        .get("pom_refresh")
        .map(|c| c.value().to_string())
        .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "missing refresh session".into()))?;
    let result = auth::refresh(
        &state.pool,
        &state.cfg.jwt_secret,
        &token,
        state.cfg.access_ttl_seconds,
        state.cfg.refresh_ttl_seconds,
    )
    .await
    .map_err(|_| ApiError(StatusCode::UNAUTHORIZED, "invalid session".into()))?;
    let jar = jar.add(refresh_cookie(
        &result.refresh_token,
        state.cfg.refresh_ttl_seconds,
    ));
    Ok((
        jar,
        Json(json!({"access_token": result.access_token, "user": user_json(&result.user)})),
    ))
}

async fn logout(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(user) = bearer_user(&state, &headers) {
        let _ = auth::logout(&state.pool, user.sid).await;
    }
    Ok((
        jar.remove(Cookie::from("pom_refresh")),
        Json(json!({"ok": true})),
    ))
}

async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, None)?;
    Ok(Json(user_json(&user)))
}

#[derive(Deserialize)]
struct PasswordBody {
    token: String,
    password: String,
}

async fn password_setup(
    State(state): State<AppState>,
    Json(body): Json<PasswordBody>,
) -> Result<impl IntoResponse, ApiError> {
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

async fn password_change(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ChangePassword>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, None)?;
    let mut tx = begin_as(&state.pool, Some(user.id), &user.role)
        .await
        .map_err(internal)?;
    let profile: UserProfileRow =
        sqlx::query_as("SELECT email, display_name FROM users WHERE id = $1")
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
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "current password is wrong".into(),
        ));
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

async fn reset_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ResetRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin")
        .await
        .map_err(internal)?;
    let user_id: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM users WHERE lower(email) = lower($1)")
            .bind(body.email.trim())
            .fetch_optional(&mut *tx)
            .await
            .map_err(internal)?;
    let Some(user_id) = user_id else {
        return Ok(Json(json!({"ok": true})));
    };
    let profile: UserProfileRow =
        sqlx::query_as("SELECT email, display_name FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(internal)?;
    let token = issue_reset_token(&mut tx, user_id, state.cfg.password_reset_ttl_minutes).await?;
    audit(
        &mut tx,
        Some(admin.id),
        "password_reset_issued",
        &user_id.to_string(),
    )
    .await;
    tx.commit().await.map_err(internal)?;
    let setup_url = state.mail.reset_password_url(&token);
    dispatch_password_recovery_email(
        state.mail.clone(),
        profile.email,
        profile.display_name,
        token,
    );
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
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<ResetRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if !valid_email(&body.email) {
        return Err(bad("email is invalid"));
    }
    let ip = client_ip::resolve_client_ip(peer, &headers, &state.trusted_proxies);
    let email_key = rate_limit::recovery_email_key(&body.email);
    if state
        .recovery_limits
        .is_blocked(&ip, RECOVERY_IP_MAX, RECOVERY_IP_WINDOW_SECS)
        || state.recovery_limits.is_blocked(
            &email_key,
            RECOVERY_EMAIL_MAX,
            RECOVERY_EMAIL_WINDOW_SECS,
        )
    {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "too many recovery attempts".into(),
        ));
    }
    let token = random_token();
    let issued: Option<UserProfileRow> =
        sqlx::query_as("SELECT email, display_name FROM auth_create_reset_token($1, $2, $3)")
            .bind(body.email.trim())
            .bind(sha256_hex(&token))
            .bind(state.cfg.password_reset_ttl_minutes as i32)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    if let Some(profile) = issued {
        dispatch_password_recovery_email(
            state.mail.clone(),
            profile.email,
            profile.display_name,
            token,
        );
    }
    state
        .recovery_limits
        .record_attempt(&ip, RECOVERY_IP_MAX, RECOVERY_IP_WINDOW_SECS);
    state.recovery_limits.record_attempt(
        &email_key,
        RECOVERY_EMAIL_MAX,
        RECOVERY_EMAIL_WINDOW_SECS,
    );
    Ok(Json(json!({ "message": FORGOT_PASSWORD_MESSAGE })))
}

#[derive(Deserialize)]
struct ResetPasswordBody {
    token: String,
    new_password: String,
}

async fn reset_password(
    State(state): State<AppState>,
    Json(body): Json<ResetPasswordBody>,
) -> Result<impl IntoResponse, ApiError> {
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

async fn verify_email_request(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, None)?;
    let ip = client_ip::resolve_client_ip(peer, &headers, &state.trusted_proxies);
    let key = format!("verify:{}:{ip}", user.id);
    if state
        .verify_limits
        .is_blocked(&key, VERIFY_MAX, VERIFY_WINDOW_SECS)
    {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "too many verification attempts".into(),
        ));
    }
    state
        .verify_limits
        .record_attempt(&key, VERIFY_MAX, VERIFY_WINDOW_SECS);
    let mut tx = begin_as(&state.pool, Some(user.id), &user.role)
        .await
        .map_err(internal)?;
    let profile: VerifyUserRow =
        sqlx::query_as("SELECT email, display_name, email_verified_at FROM users WHERE id = $1")
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
    dispatch_email_verification(
        state.mail.clone(),
        profile.email,
        profile.display_name,
        token,
    );
    Ok(Json(json!({"ok": true, "status": "sent"})))
}

async fn verify_email_confirm(
    State(state): State<AppState>,
    Json(body): Json<TokenOnly>,
) -> Result<impl IntoResponse, ApiError> {
    auth::consume_verification(&state.pool, &body.token)
        .await
        .map_err(|e| bad(e.to_string()))?;
    Ok(Json(json!({"ok": true, "status": "verified"})))
}

async fn admin_overview(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin")
        .await
        .map_err(internal)?;
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let accounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM trading_accounts")
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let traders: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM source_traders")
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM source_events")
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let errors: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM system_events WHERE severity = 'error' AND created_at > now() - interval '24 hours'")
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let kill: Value =
        sqlx::query_scalar("SELECT value FROM system_settings WHERE key = 'global_kill_switch'")
            .fetch_one(&mut *tx)
            .await
            .map_err(internal)?;
    Ok(Json(
        json!({"users": users, "accounts": accounts, "source_traders": traders, "source_events": events, "errors_24h": errors, "kill_switch": kill}),
    ))
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

async fn admin_create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateUser>,
) -> Result<impl IntoResponse, ApiError> {
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
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin")
        .await
        .map_err(internal)?;
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
            audit(
                &mut tx,
                Some(admin.id),
                "user_reinvited",
                &existing.id.to_string(),
            )
            .await;
            tx.commit().await.map_err(internal)?;
            let setup_url = state.mail.setup_url(&token);
            let email_delivery =
                invitation_delivery(&state.mail, &existing.email, &existing.display_name, &token)
                    .await;
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
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "email": email,
            "display_name": display_name,
            "role": role,
            "status": "invited",
            "setup_url": setup_url,
            "email_delivery": email_delivery,
        })),
    ))
}

async fn admin_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin")
        .await
        .map_err(internal)?;
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

async fn admin_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin")
        .await
        .map_err(internal)?;
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

async fn admin_update_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateUser>,
) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    if let Some(role) = &body.role {
        if role != "user" && role != "admin" {
            return Err(bad("role must be user or admin"));
        }
    }
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin")
        .await
        .map_err(internal)?;
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

async fn admin_suspend(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    set_status(&state, &headers, id, "suspended").await
}

async fn admin_activate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    set_status(&state, &headers, id, "active").await
}

async fn set_status(
    state: &AppState,
    headers: &HeaderMap,
    id: Uuid,
    status: &str,
) -> Result<Json<Value>, ApiError> {
    let admin = require_user(state, headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin")
        .await
        .map_err(internal)?;
    let updated = sqlx::query("UPDATE users SET status = $2, updated_at = now() WHERE id = $1 AND password_hash IS NOT NULL")
        .bind(id)
        .bind(status)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    if updated.rows_affected() == 0 {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "user not found or password is not set".into(),
        ));
    }
    audit(&mut tx, Some(admin.id), status, &id.to_string()).await;
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"ok": true, "status": status})))
}

async fn admin_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin")
        .await
        .map_err(internal)?;
    let profile: UserProfileRow =
        sqlx::query_as("SELECT email, display_name FROM users WHERE id = $1")
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
    let email_delivery =
        invitation_delivery(&state.mail, &profile.email, &profile.display_name, &token).await;
    Ok(Json(json!({
        "setup_url": setup_url,
        "email_delivery": email_delivery,
    })))
}

async fn admin_health(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin")
        .await
        .map_err(internal)?;
    let settings = sqlx::query_as::<_, SettingRow>(
        "SELECT key, value, updated_at FROM system_settings ORDER BY key",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(settings))
}

async fn admin_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin")
        .await
        .map_err(internal)?;
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

async fn admin_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    admin_health(State(state), headers).await
}

#[derive(Deserialize)]
struct KillBody {
    enabled: bool,
}

async fn admin_kill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<KillBody>,
) -> Result<impl IntoResponse, ApiError> {
    let admin = require_user(&state, &headers, Some("admin"))?;
    let mut tx = begin_as(&state.pool, Some(admin.id), "admin")
        .await
        .map_err(internal)?;
    sqlx::query("UPDATE system_settings SET value = $1, updated_at = now() WHERE key = 'global_kill_switch'")
        .bind(json!({"enabled": body.enabled}))
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    audit(
        &mut tx,
        Some(admin.id),
        "kill_switch",
        if body.enabled { "on" } else { "off" },
    )
    .await;
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"enabled": body.enabled})))
}

async fn dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let accounts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM trading_accounts WHERE user_id = $1")
            .bind(user.id)
            .fetch_one(&mut *tx)
            .await
            .map_err(internal)?;
    let traders: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM source_traders WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let detected: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM source_events e WHERE EXISTS (SELECT 1 FROM source_traders t WHERE t.user_id = $1 AND lower(t.wallet_address) = lower(e.source_wallet))",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    let copied: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM copy_decisions WHERE user_id = $1 AND status = 'approved'",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    let rejected: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM copy_decisions WHERE user_id = $1 AND status IN ('rejected', 'skipped')")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let open_positions: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM positions WHERE user_id = $1 AND open_shares > 0")
            .bind(user.id)
            .fetch_one(&mut *tx)
            .await
            .map_err(internal)?;
    let pnl: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(realized_pnl_usd), 0) FROM positions WHERE user_id = $1",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    let account_rows = sqlx::query_as::<_, AccountCard>(
        "SELECT id, name, signer_address, funder_address, signature_type, operating_mode, status, emergency_stop, trading_control_state, last_exchange_balance_usd, last_balance_checked_at, last_connection_error
         FROM trading_accounts WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user.id)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    let collector: Value =
        sqlx::query_scalar("SELECT value FROM system_settings WHERE key = 'collector_status'")
            .fetch_optional(&mut *tx)
            .await
            .map_err(internal)?
            .unwrap_or(json!({}));
    let heartbeat: Value =
        sqlx::query_scalar("SELECT value FROM system_settings WHERE key = 'worker_heartbeat'")
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

async fn create_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AccountBody>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    validate_signature(&body.signature_type)?;
    if !is_eth_address(&body.signer_address) {
        return Err(bad("signer address is invalid"));
    }
    let funder = body
        .funder_address
        .clone()
        .unwrap_or_else(|| body.signer_address.clone());
    if !is_eth_address(&funder) {
        return Err(bad("funder address is invalid"));
    }
    if let Some(key) = &body.private_key {
        if !is_private_key(key) {
            return Err(bad(
                "private key must be a 32-byte hex key, not a seed phrase",
            ));
        }
        let derived = crate::polymarket::live::address_of(key).map_err(bad)?;
        if derived != normalize_address(&body.signer_address) {
            return Err(bad("private key does not match signer address"));
        }
    }
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
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
        store_key(&state.vault, &mut tx, user.id, id, key)
            .await
            .map_err(internal)?;
    }
    audit(&mut tx, Some(user.id), "account_created", &id.to_string()).await;
    tx.commit().await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

async fn list_accounts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let rows = sqlx::query_as::<_, AccountCard>(
        "SELECT id, name, signer_address, funder_address, signature_type, operating_mode, status, emergency_stop, trading_control_state, last_exchange_balance_usd, last_balance_checked_at, last_connection_error
         FROM trading_accounts WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user.id)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

async fn get_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let row = load_account(&mut tx, user.id, id).await?;
    let risk = sqlx::query_as::<_, RiskRow>(
        "SELECT * FROM account_risk_limits WHERE trading_account_id = $1",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(
        json!({"account": row, "risk": risk, "credentials_stored": credential_exists(&mut tx, id).await?}),
    ))
}

#[derive(Deserialize)]
struct PatchAccount {
    name: Option<String>,
}

async fn patch_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchAccount>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
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

async fn delete_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
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

async fn test_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let clob_ok = state.polymarket.clob_ok().await.unwrap_or(false);
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
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
        let key = secret
            .get("private_key")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let probed = probe_account(
            &state.cfg.clob_host,
            key,
            &account.signature_type,
            Some(&account.funder_address),
        )
        .await;
        report["balance_usd"] = serde_json::to_value(probed.balance_usd).unwrap_or(Value::Null);
        report["api_credentials_created"] = json!(probed.api_credentials_created);
        report["signer_matches"] = json!(probed.signer_address == account.signer_address);
        if let Some(error) = &probed.error {
            report["error"] = json!(error);
        }
        let mut tx = begin_as(&state.pool, Some(user.id), "user")
            .await
            .map_err(internal)?;
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
        let mut tx = begin_as(&state.pool, Some(user.id), "user")
            .await
            .map_err(internal)?;
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

async fn account_balance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let account = load_account(&mut tx, user.id, id).await?;
    let paper: Decimal = sqlx::query_scalar(
        "SELECT paper_cash_usd FROM account_risk_limits WHERE trading_account_id = $1",
    )
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

async fn account_positions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
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

async fn set_mode(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<ModeBody>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    if !matches!(
        body.mode.as_str(),
        "monitoring" | "paper_trading" | "live_trading"
    ) {
        return Err(bad("unknown mode"));
    }
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let account = load_account(&mut tx, user.id, id).await?;
    if body.mode == "live_trading" {
        if body.confirm_live != Some(true)
            || body.confirmation.as_deref() != Some("ENABLE LIVE TRADING")
        {
            return Err(bad("type ENABLE LIVE TRADING and set confirm_live=true"));
        }
        if !state.cfg.confirm_live_trading {
            return Err(ApiError(
                StatusCode::FORBIDDEN,
                "server CONFIRM_LIVE_TRADING is not enabled".into(),
            ));
        }
        let has_key = credential_exists(&mut tx, id).await?;
        if !has_key || account.status != "connected" || account.last_exchange_balance_usd.is_none()
        {
            return Err(bad(
                "connect the account and refresh its balance before live trading",
            ));
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

async fn disconnect_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    sqlx::query("UPDATE trading_accounts SET status = 'disconnected', operating_mode = 'monitoring', emergency_stop = true, trading_control_state = 'kill_switch', updated_at = now() WHERE id = $1 AND user_id = $2")
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

async fn patch_risk(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<RiskBody>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
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

async fn rotate_credentials(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<AccountBody>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let Some(key) = body.private_key.as_deref() else {
        return Err(bad("private_key is required"));
    };
    if !is_private_key(key) {
        return Err(bad("private key must be a 32-byte hex key"));
    }
    let derived = crate::polymarket::live::address_of(key).map_err(bad)?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let account = load_account(&mut tx, user.id, id).await?;
    if derived != account.signer_address && body.signer_address.is_empty() {
        return Err(bad("private key does not match the stored signer"));
    }
    store_key(&state.vault, &mut tx, user.id, id, key)
        .await
        .map_err(internal)?;
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

async fn create_trader(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<TraderBody>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    if !is_eth_address(&body.wallet_address) {
        return Err(bad("wallet address is invalid"));
    }
    let wallet = normalize_address(&body.wallet_address);
    let profile = state.polymarket.profile_for_registration(&wallet).await;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
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
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "profile": profile})),
    ))
}

async fn list_traders(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let rows = sqlx::query_as::<_, TraderRow>(
        "SELECT t.id, t.wallet_address, t.alias, t.monitoring_enabled, t.last_trade_at, t.observed_trade_count,
                t.copied_trade_count, t.rejected_trade_count, t.profile,
                COALESCE((SELECT json_agg(json_build_object(
                            'account_id', s.trading_account_id,
                            'strategy_id', s.id,
                            'enabled', s.enabled,
                            'account_name', a.name
                          ))
                          FROM copy_strategies s
                          JOIN trading_accounts a ON a.id = s.trading_account_id
                          WHERE s.source_trader_id = t.id), '[]'::json) AS links
         FROM source_traders t WHERE t.user_id = $1 ORDER BY t.created_at DESC",
    )
    .bind(user.id)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

async fn get_trader(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let row = sqlx::query_as::<_, TraderRow>(
        "SELECT t.id, t.wallet_address, t.alias, t.monitoring_enabled, t.last_trade_at, t.observed_trade_count,
                t.copied_trade_count, t.rejected_trade_count, t.profile,
                COALESCE((SELECT json_agg(json_build_object(
                            'account_id', s.trading_account_id,
                            'strategy_id', s.id,
                            'enabled', s.enabled,
                            'account_name', a.name
                          ))
                          FROM copy_strategies s
                          JOIN trading_accounts a ON a.id = s.trading_account_id
                          WHERE s.source_trader_id = t.id), '[]'::json) AS links
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

async fn patch_trader(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchTrader>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
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

async fn delete_trader(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
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
    /// UI percent 0–100; converted to `proportion` when sizing_mode is proportional.
    copy_percent: Option<Decimal>,
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

async fn create_strategy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<StrategyBody>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mode = body.sizing_mode.unwrap_or_else(|| "fixed_amount".into());
    if !matches!(
        mode.as_str(),
        "fixed_amount" | "proportional" | "multiplier"
    ) {
        return Err(bad("unknown sizing mode"));
    }
    let budget = body.strategy_budget_usd.unwrap_or(Decimal::from(100));
    if budget <= Decimal::ZERO || budget > Decimal::from(10_000_000) {
        return Err(bad("strategy budget must be between 0 and 10000000 USD"));
    }
    let mut proportion = body.proportion;
    if mode == "proportional" {
        if let Some(pct) = body.copy_percent {
            proportion = Some(
                percent_to_proportion(pct)
                    .ok_or_else(|| bad("copy_percent must be greater than 0 and at most 100"))?,
            );
        }
        if proportion.is_none() {
            return Err(bad(
                "copy_percent or proportion is required for proportional sizing",
            ));
        }
    }
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let _ = load_account(&mut tx, user.id, body.trading_account_id).await?;
    let owns_trader: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM source_traders WHERE id = $1 AND user_id = $2)",
    )
    .bind(body.source_trader_id)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    if !owns_trader {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "source trader not found".into(),
        ));
    }
    let monitoring: bool =
        sqlx::query_scalar("SELECT monitoring_enabled FROM source_traders WHERE id = $1")
            .bind(body.source_trader_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(internal)?;
    if !monitoring {
        return Err(bad(
            "enable monitoring on the source account before creating a copy strategy",
        ));
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
    .bind(proportion)
    .bind(body.multiplier)
    .bind(budget)
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
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "enabled": false})),
    ))
}

async fn list_strategies(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<StrategyFilter>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
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

async fn get_strategy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let row = one_strategy(&mut tx, user.id, id).await?;
    Ok(Json(row))
}

async fn patch_strategy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<StrategyBody>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let _ = one_strategy(&mut tx, user.id, id).await?;
    let mut proportion = body.proportion;
    if let Some(pct) = body.copy_percent {
        proportion = Some(
            percent_to_proportion(pct)
                .ok_or_else(|| bad("copy_percent must be greater than 0 and at most 100"))?,
        );
    }
    if let Some(b) = body.strategy_budget_usd {
        if b <= Decimal::ZERO || b > Decimal::from(10_000_000) {
            return Err(bad("strategy budget must be between 0 and 10000000 USD"));
        }
    }
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
    .bind(proportion)
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

async fn strategy_utilization(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let util = utilization_for_user(&state.pool, user.id, id)
        .await
        .map_err(|_| ApiError(StatusCode::NOT_FOUND, "strategy not found".into()))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let summary: StrategyActivitySummary = sqlx::query_as(
        "SELECT
            s.enabled,
            a.operating_mode,
            st.monitoring_enabled,
            (SELECT COUNT(*) FROM copy_decisions d WHERE d.strategy_id = s.id AND d.status = 'approved') AS copied_count,
            (SELECT COUNT(*) FROM copy_decisions d WHERE d.strategy_id = s.id AND d.status = 'rejected') AS rejected_count,
            (SELECT MAX(e.event_ts) FROM source_events e
                JOIN source_traders t ON lower(t.wallet_address) = lower(e.source_wallet)
                WHERE t.id = s.source_trader_id) AS last_source_event_at,
            (SELECT MAX(o.updated_at) FROM orders o
                JOIN copy_decisions d ON d.id = o.decision_id
                WHERE d.strategy_id = s.id AND d.status = 'approved') AS last_copied_at
         FROM copy_strategies s
         JOIN trading_accounts a ON a.id = s.trading_account_id
         JOIN source_traders st ON st.id = s.source_trader_id
         WHERE s.id = $1 AND s.user_id = $2",
    )
    .bind(id)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| ApiError(StatusCode::NOT_FOUND, "strategy not found".into()))?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({
        "allocated_budget_usd": util.allocated_budget_usd,
        "deployed_capital_usd": util.deployed_capital_usd,
        "reserved_capital_usd": util.reserved_capital_usd,
        "available_budget_usd": util.available_budget_usd,
        "open_positions_count": util.open_positions_count,
        "updated_at": util.updated_at,
        "enabled": summary.enabled,
        "operating_mode": summary.operating_mode,
        "monitoring_enabled": summary.monitoring_enabled,
        "copied_trades_count": summary.copied_count,
        "rejected_trades_count": summary.rejected_count,
        "last_source_event_at": summary.last_source_event_at,
        "last_copied_at": summary.last_copied_at,
    })))
}

#[derive(sqlx::FromRow)]
struct StrategyActivitySummary {
    enabled: bool,
    operating_mode: String,
    monitoring_enabled: bool,
    copied_count: i64,
    rejected_count: i64,
    last_source_event_at: Option<chrono::DateTime<Utc>>,
    last_copied_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Deserialize)]
struct CopyPreviewBody {
    trading_account_id: Uuid,
    sizing_mode: String,
    copy_percent: Option<Decimal>,
    proportion: Option<Decimal>,
    fixed_amount_usd: Option<Decimal>,
    multiplier: Option<Decimal>,
    strategy_budget_usd: Decimal,
    max_position_usd: Option<Decimal>,
    max_exposure_usd: Option<Decimal>,
    min_order_usd: Option<Decimal>,
    copy_buy: Option<bool>,
    copy_sell: Option<bool>,
    allowed_markets: Option<Value>,
    limit: Option<i32>,
}

async fn copy_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<CopyPreviewBody>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    if body.strategy_budget_usd <= Decimal::ZERO {
        return Err(bad("strategy budget must be positive"));
    }
    let limit = body.limit.unwrap_or(20).clamp(1, 50) as i64;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let wallet: String = sqlx::query_scalar(
        "SELECT wallet_address FROM source_traders WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?
    .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "trader not found".into()))?;
    let _ = load_account(&mut tx, user.id, body.trading_account_id).await?;
    let risk = sqlx::query_as::<_, AccountRiskRow>(
        "SELECT max_order_usd, max_exposure_usd, min_order_usd FROM account_risk_limits WHERE trading_account_id = $1",
    )
    .bind(body.trading_account_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;

    let mut proportion = body.proportion;
    if body.sizing_mode == "proportional" {
        if let Some(pct) = body.copy_percent {
            proportion = Some(
                percent_to_proportion(pct)
                    .ok_or_else(|| bad("copy_percent must be greater than 0 and at most 100"))?,
            );
        }
        if proportion.is_none() {
            return Err(bad("copy_percent or proportion is required"));
        }
    }

    let rows = sqlx::query_as::<_, PreviewSourceRow>(
        "SELECT id, token_id, condition_id, outcome, market_title, market_icon, market_slug,
                side, price, size_shares, notional_usd, event_ts
         FROM source_events
         WHERE lower(source_wallet) = lower($1)
         ORDER BY event_ts DESC
         LIMIT $2",
    )
    .bind(&wallet)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;

    let allowed = body.allowed_markets.unwrap_or_else(|| json!([]));
    let params = CopyPreviewParams {
        sizing_mode: body.sizing_mode,
        fixed_amount_usd: body.fixed_amount_usd,
        proportion,
        multiplier: body.multiplier,
        strategy_budget_usd: body.strategy_budget_usd,
        max_position_usd: body.max_position_usd.unwrap_or(Decimal::from(50)),
        max_exposure_usd: body.max_exposure_usd.unwrap_or(body.strategy_budget_usd),
        account_max_exposure: risk.max_exposure_usd,
        max_order_usd: risk.max_order_usd,
        min_order_usd: body
            .min_order_usd
            .unwrap_or(risk.min_order_usd)
            .max(Decimal::ONE),
        copy_buy: body.copy_buy.unwrap_or(true),
        copy_sell: body.copy_sell.unwrap_or(true),
    };

    let events: Vec<PreviewEvent> = rows
        .iter()
        .map(|r| {
            let pe = PreviewEvent {
                id: r.id,
                token_id: r.token_id.clone(),
                side: r.side.clone(),
                price: r.price,
                size_shares: r.size_shares,
                notional_usd: r.notional_usd,
                event_ts: r.event_ts,
                market_title: r.market_title.clone(),
                market_icon: r.market_icon.clone(),
                outcome: r.outcome.clone(),
                condition_id: r.condition_id.clone(),
                market_slug: r.market_slug.clone(),
                allowed_check: true,
            };
            PreviewEvent {
                allowed_check: market_allowed_list(&allowed, &pe),
                ..pe
            }
        })
        .collect();

    let preview_rows = simulate_copy_preview(&events, &params);
    Ok(Json(json!({
        "disclaimer": "Estimates use recent source trades and current settings. Live execution may differ due to book, slippage, fees, and latency.",
        "rows": preview_rows,
    })))
}

#[derive(sqlx::FromRow)]
struct PreviewSourceRow {
    id: Uuid,
    token_id: String,
    condition_id: Option<String>,
    outcome: Option<String>,
    market_title: Option<String>,
    market_icon: Option<String>,
    market_slug: Option<String>,
    side: String,
    price: Decimal,
    size_shares: Decimal,
    notional_usd: Decimal,
    event_ts: chrono::DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct AccountRiskRow {
    max_order_usd: Decimal,
    max_exposure_usd: Decimal,
    min_order_usd: Decimal,
}

#[derive(Deserialize)]
struct EnableBody {
    enabled: bool,
}

async fn enable_strategy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<EnableBody>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    if body.enabled {
        #[derive(sqlx::FromRow)]
        struct EnableCheck {
            monitoring_enabled: bool,
            operating_mode: String,
            account_status: String,
            emergency_stop: bool,
            strategy_budget_usd: Decimal,
            min_order_usd: Decimal,
            sizing_mode: String,
            proportion: Option<Decimal>,
            fixed_amount_usd: Option<Decimal>,
            multiplier: Option<Decimal>,
            paper_cash_usd: Decimal,
        }
        let row = sqlx::query_as::<_, EnableCheck>(
            "SELECT st.monitoring_enabled, a.operating_mode, a.status AS account_status, a.emergency_stop,
                    s.strategy_budget_usd, s.min_order_usd, s.sizing_mode, s.proportion, s.fixed_amount_usd, s.multiplier,
                    r.paper_cash_usd
             FROM copy_strategies s
             JOIN source_traders st ON st.id = s.source_trader_id
             JOIN trading_accounts a ON a.id = s.trading_account_id
             JOIN account_risk_limits r ON r.trading_account_id = a.id
             WHERE s.id = $1 AND s.user_id = $2",
        )
        .bind(id)
        .bind(user.id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "strategy not found".into()))?;
        if !row.monitoring_enabled {
            return Err(bad("enable monitoring on the source account first"));
        }
        if row.account_status == "disconnected" {
            return Err(bad(
                "connect the trading account before enabling copy trading",
            ));
        }
        if row.emergency_stop {
            return Err(bad("clear emergency stop on the trading account first"));
        }
        if row.operating_mode == "monitoring" {
            return Err(bad(
                "set the trading account to paper_trading or live_trading",
            ));
        }
        if row.operating_mode == "live_trading" {
            return Err(bad(
                "live copy trading is not enabled in this release; use paper_trading",
            ));
        }
        if row.strategy_budget_usd < row.min_order_usd {
            return Err(bad(
                "strategy budget is smaller than the minimum order size",
            ));
        }
        match row.sizing_mode.as_str() {
            "proportional" if row.proportion.is_none() => {
                return Err(bad("set copy percentage before enabling the strategy"));
            }
            "fixed_amount" if row.fixed_amount_usd.is_none() => {
                return Err(bad("set fixed amount before enabling the strategy"));
            }
            "multiplier" if row.multiplier.is_none() => {
                return Err(bad("set multiplier before enabling the strategy"));
            }
            _ => {}
        }
        if row.operating_mode == "paper_trading" && row.paper_cash_usd < row.min_order_usd {
            return Err(bad("increase paper cash on the trading account"));
        }
    }
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

async fn delete_strategy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    sqlx::query("DELETE FROM copy_strategies WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

async fn list_activity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM timeline_events
         WHERE user_id = $1
           AND ($2::text IS NULL OR kind = $2)
           AND ($3::text IS NULL OR message ILIKE '%' || $3 || '%')
           AND ($4::timestamptz IS NULL OR at >= $4)
           AND ($5::timestamptz IS NULL OR at < $5)",
    )
    .bind(user.id)
    .bind(q.filter_kind())
    .bind(q.filter_q())
    .bind(q.parse_from())
    .bind(q.parse_to_exclusive())
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    let rows = sqlx::query_as::<_, ActivityRow>(
        "SELECT id, kind, message, at, source_event_id, decision_id, order_id
         FROM timeline_events
         WHERE user_id = $1
           AND ($2::text IS NULL OR kind = $2)
           AND ($3::text IS NULL OR message ILIKE '%' || $3 || '%')
           AND ($4::timestamptz IS NULL OR at >= $4)
           AND ($5::timestamptz IS NULL OR at < $5)
         ORDER BY at DESC, id DESC
         LIMIT $6 OFFSET $7",
    )
    .bind(user.id)
    .bind(q.filter_kind())
    .bind(q.filter_q())
    .bind(q.parse_from())
    .bind(q.parse_to_exclusive())
    .bind(q.limit())
    .bind(q.offset())
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(Paged {
        items: rows,
        total,
        limit: q.limit(),
        offset: q.offset(),
    }))
}

async fn list_source_trades(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let wallet_ids = q.parse_wallets();
    let wallet_ids = if wallet_ids.is_empty() {
        None
    } else {
        Some(wallet_ids)
    };
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM source_events e
         WHERE EXISTS (
            SELECT 1 FROM source_traders t
            WHERE t.user_id = $1 AND lower(t.wallet_address) = lower(e.source_wallet)
         )
         AND ($2::text IS NULL OR e.side = $2)
         AND ($3::text IS NULL OR e.source_wallet ILIKE '%' || $3 || '%' OR e.token_id ILIKE '%' || $3 || '%'
              OR e.tx_hash ILIKE '%' || $3 || '%' OR e.market_title ILIKE '%' || $3 || '%'
              OR e.market_slug ILIKE '%' || $3 || '%' OR e.outcome ILIKE '%' || $3 || '%'
              OR EXISTS (SELECT 1 FROM source_traders st WHERE st.user_id = $1 AND lower(st.wallet_address) = lower(e.source_wallet) AND st.alias ILIKE '%' || $3 || '%'))
         AND ($4::text IS NULL OR e.outcome ILIKE $4)
         AND ($5::text IS NULL OR e.activity_type = $5)
         AND ($6::text IS NULL OR e.status = $6)
         AND ($7::timestamptz IS NULL OR e.event_ts >= $7)
         AND ($8::timestamptz IS NULL OR e.event_ts < $8)
         AND ($9::numeric IS NULL OR COALESCE(e.usdc_size, e.notional_usd) >= $9)
         AND ($10::numeric IS NULL OR COALESCE(e.usdc_size, e.notional_usd) <= $10)
         AND ($11::numeric IS NULL OR e.price >= $11)
         AND ($12::numeric IS NULL OR e.price <= $12)
         AND ($13::uuid[] IS NULL OR EXISTS (
              SELECT 1 FROM source_traders st
              WHERE st.user_id = $1 AND st.id = ANY($13) AND lower(st.wallet_address) = lower(e.source_wallet)
         ))",
    )
    .bind(user.id)
    .bind(q.filter_side())
    .bind(q.filter_q())
    .bind(q.filter_outcome())
    .bind(q.filter_activity_type())
    .bind(q.filter_status())
    .bind(q.parse_from())
    .bind(q.parse_to_exclusive())
    .bind(q.min_amount())
    .bind(q.max_amount())
    .bind(q.min_price())
    .bind(q.max_price())
    .bind(wallet_ids.clone())
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    let rows = sqlx::query_as::<_, TradeRow>(
        "SELECT e.id, e.source_wallet, e.tx_hash, e.token_id, e.condition_id, e.outcome, e.market_title, e.market_slug,
                e.event_title, e.event_slug, e.market_icon, e.activity_type, e.usdc_size,
                e.side, e.price, e.size_shares, e.notional_usd, e.event_ts, e.origin, e.status
         FROM source_events e
         WHERE EXISTS (
            SELECT 1 FROM source_traders t
            WHERE t.user_id = $1 AND lower(t.wallet_address) = lower(e.source_wallet)
         )
         AND ($2::text IS NULL OR e.side = $2)
         AND ($3::text IS NULL OR e.source_wallet ILIKE '%' || $3 || '%' OR e.token_id ILIKE '%' || $3 || '%'
              OR e.tx_hash ILIKE '%' || $3 || '%' OR e.market_title ILIKE '%' || $3 || '%'
              OR e.market_slug ILIKE '%' || $3 || '%' OR e.outcome ILIKE '%' || $3 || '%'
              OR EXISTS (SELECT 1 FROM source_traders st WHERE st.user_id = $1 AND lower(st.wallet_address) = lower(e.source_wallet) AND st.alias ILIKE '%' || $3 || '%'))
         AND ($4::text IS NULL OR e.outcome ILIKE $4)
         AND ($5::text IS NULL OR e.activity_type = $5)
         AND ($6::text IS NULL OR e.status = $6)
         AND ($7::timestamptz IS NULL OR e.event_ts >= $7)
         AND ($8::timestamptz IS NULL OR e.event_ts < $8)
         AND ($9::numeric IS NULL OR COALESCE(e.usdc_size, e.notional_usd) >= $9)
         AND ($10::numeric IS NULL OR COALESCE(e.usdc_size, e.notional_usd) <= $10)
         AND ($11::numeric IS NULL OR e.price >= $11)
         AND ($12::numeric IS NULL OR e.price <= $12)
         AND ($13::uuid[] IS NULL OR EXISTS (
              SELECT 1 FROM source_traders st
              WHERE st.user_id = $1 AND st.id = ANY($13) AND lower(st.wallet_address) = lower(e.source_wallet)
         ))
         ORDER BY e.event_ts DESC, e.id DESC
         LIMIT $14 OFFSET $15",
    )
    .bind(user.id)
    .bind(q.filter_side())
    .bind(q.filter_q())
    .bind(q.filter_outcome())
    .bind(q.filter_activity_type())
    .bind(q.filter_status())
    .bind(q.parse_from())
    .bind(q.parse_to_exclusive())
    .bind(q.min_amount())
    .bind(q.max_amount())
    .bind(q.min_price())
    .bind(q.max_price())
    .bind(wallet_ids)
    .bind(q.limit())
    .bind(q.offset())
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(Paged {
        items: rows,
        total,
        limit: q.limit(),
        offset: q.offset(),
    }))
}

async fn list_decisions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let rows = sqlx::query_as::<_, DecisionRow>(
        "SELECT id, trading_account_id, source_event_id, strategy_id, status, reason, operating_mode, side,
                target_notional_usd, target_shares, limit_price, book_price, available_balance_usd, decided_at
         FROM copy_decisions
         WHERE user_id = $1
           AND ($2::uuid IS NULL OR trading_account_id = $2)
           AND ($3::text IS NULL OR status = $3)
           AND ($4::text IS NULL OR side = $4)
           AND ($5::text IS NULL OR operating_mode = $5)
         ORDER BY decided_at DESC, id DESC LIMIT $6 OFFSET $7",
    )
    .bind(user.id)
    .bind(q.account_id)
    .bind(q.filter_status())
    .bind(q.filter_side())
    .bind(ListQuery::filter_text(&q.mode))
    .bind(q.limit())
    .bind(q.offset())
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM copy_decisions WHERE user_id = $1
           AND ($2::uuid IS NULL OR trading_account_id = $2)
           AND ($3::text IS NULL OR status = $3)
           AND ($4::text IS NULL OR side = $4)
           AND ($5::text IS NULL OR operating_mode = $5)",
    )
    .bind(user.id)
    .bind(q.account_id)
    .bind(q.filter_status())
    .bind(q.filter_side())
    .bind(ListQuery::filter_text(&q.mode))
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(Paged {
        items: rows,
        total,
        limit: q.limit(),
        offset: q.offset(),
    }))
}

async fn list_orders(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders o
         LEFT JOIN source_events se ON se.id = o.source_event_id
         WHERE o.user_id = $1
           AND ($2::uuid IS NULL OR o.trading_account_id = $2)
           AND ($3::text IS NULL OR o.status = $3)
           AND ($4::text IS NULL OR o.side = $4)
           AND ($5::text IS NULL OR o.operating_mode = $5)
           AND ($6::text IS NULL OR o.token_id ILIKE '%' || $6 || '%' OR o.exchange_order_id ILIKE '%' || $6 || '%'
                OR se.market_title ILIKE '%' || $6 || '%')
           AND ($7::timestamptz IS NULL OR o.created_at >= $7)
           AND ($8::timestamptz IS NULL OR o.created_at < $8)
           AND ($9::numeric IS NULL OR o.limit_price >= $9)
           AND ($10::numeric IS NULL OR o.limit_price <= $10)",
    )
    .bind(user.id)
    .bind(q.account_id)
    .bind(q.filter_status())
    .bind(q.filter_side())
    .bind(ListQuery::filter_text(&q.mode))
    .bind(q.filter_q())
    .bind(q.parse_from())
    .bind(q.parse_to_exclusive())
    .bind(q.min_price())
    .bind(q.max_price())
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    let rows = sqlx::query_as::<_, OrderListRow>(
        "SELECT o.id, o.trading_account_id, o.decision_id, o.source_event_id, o.exchange_order_id, o.operating_mode, o.order_type, o.side,
                o.token_id, o.limit_price, o.size_shares, o.status, o.error, o.created_at, o.submitted_at, o.updated_at,
                se.market_title, se.outcome, se.market_icon, se.market_slug
         FROM orders o
         LEFT JOIN source_events se ON se.id = o.source_event_id
         WHERE o.user_id = $1
           AND ($2::uuid IS NULL OR o.trading_account_id = $2)
           AND ($3::text IS NULL OR o.status = $3)
           AND ($4::text IS NULL OR o.side = $4)
           AND ($5::text IS NULL OR o.operating_mode = $5)
           AND ($6::text IS NULL OR o.token_id ILIKE '%' || $6 || '%' OR o.exchange_order_id ILIKE '%' || $6 || '%'
                OR se.market_title ILIKE '%' || $6 || '%')
           AND ($7::timestamptz IS NULL OR o.created_at >= $7)
           AND ($8::timestamptz IS NULL OR o.created_at < $8)
           AND ($9::numeric IS NULL OR o.limit_price >= $9)
           AND ($10::numeric IS NULL OR o.limit_price <= $10)
         ORDER BY o.created_at DESC, o.id DESC
         LIMIT $11 OFFSET $12",
    )
    .bind(user.id)
    .bind(q.account_id)
    .bind(q.filter_status())
    .bind(q.filter_side())
    .bind(ListQuery::filter_text(&q.mode))
    .bind(q.filter_q())
    .bind(q.parse_from())
    .bind(q.parse_to_exclusive())
    .bind(q.min_price())
    .bind(q.max_price())
    .bind(q.limit())
    .bind(q.offset())
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(Paged {
        items: rows,
        total,
        limit: q.limit(),
        offset: q.offset(),
    }))
}

async fn list_fills(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fills f
         LEFT JOIN orders o ON o.id = f.order_id
         LEFT JOIN source_events se ON se.id = o.source_event_id
         WHERE f.user_id = $1
           AND ($2::uuid IS NULL OR f.trading_account_id = $2)
           AND ($3::text IS NULL OR o.side = $3)
           AND ($4::text IS NULL OR se.outcome ILIKE $4 OR se.market_title ILIKE '%' || $4 || '%')
           AND ($5::text IS NULL OR f.tx_hash ILIKE '%' || $5 || '%' OR f.order_id::text ILIKE '%' || $5 || '%'
                OR se.market_title ILIKE '%' || $5 || '%')
           AND ($6::timestamptz IS NULL OR f.filled_at >= $6)
           AND ($7::timestamptz IS NULL OR f.filled_at < $7)
           AND ($8::numeric IS NULL OR f.price >= $8)
           AND ($9::numeric IS NULL OR f.price <= $9)",
    )
    .bind(user.id)
    .bind(q.account_id)
    .bind(q.filter_side())
    .bind(q.filter_outcome())
    .bind(q.filter_q())
    .bind(q.parse_from())
    .bind(q.parse_to_exclusive())
    .bind(q.min_price())
    .bind(q.max_price())
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    let rows = sqlx::query_as::<_, FillListRow>(
        "SELECT f.id, f.order_id, f.trading_account_id, f.price, f.shares, f.notional_usd, f.fee_usd, f.fee_status, f.tx_hash,
                f.slippage_vs_source_bps, f.slippage_vs_limit_bps, f.filled_at,
                se.market_title, se.outcome, se.market_icon, o.side AS order_side
         FROM fills f
         LEFT JOIN orders o ON o.id = f.order_id
         LEFT JOIN source_events se ON se.id = o.source_event_id
         WHERE f.user_id = $1
           AND ($2::uuid IS NULL OR f.trading_account_id = $2)
           AND ($3::text IS NULL OR o.side = $3)
           AND ($4::text IS NULL OR se.outcome ILIKE $4 OR se.market_title ILIKE '%' || $4 || '%')
           AND ($5::text IS NULL OR f.tx_hash ILIKE '%' || $5 || '%' OR f.order_id::text ILIKE '%' || $5 || '%'
                OR se.market_title ILIKE '%' || $5 || '%')
           AND ($6::timestamptz IS NULL OR f.filled_at >= $6)
           AND ($7::timestamptz IS NULL OR f.filled_at < $7)
           AND ($8::numeric IS NULL OR f.price >= $8)
           AND ($9::numeric IS NULL OR f.price <= $9)
         ORDER BY f.filled_at DESC, f.id DESC
         LIMIT $10 OFFSET $11",
    )
    .bind(user.id)
    .bind(q.account_id)
    .bind(q.filter_side())
    .bind(q.filter_outcome())
    .bind(q.filter_q())
    .bind(q.parse_from())
    .bind(q.parse_to_exclusive())
    .bind(q.min_price())
    .bind(q.max_price())
    .bind(q.limit())
    .bind(q.offset())
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(Paged {
        items: rows,
        total,
        limit: q.limit(),
        offset: q.offset(),
    }))
}

async fn list_positions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM positions
         WHERE user_id = $1 AND open_shares > 0
           AND ($2::uuid IS NULL OR trading_account_id = $2)
           AND ($3::text IS NULL OR market_title ILIKE '%' || $3 || '%' OR outcome ILIKE '%' || $3 || '%' OR token_id ILIKE '%' || $3 || '%')",
    )
    .bind(user.id)
    .bind(q.account_id)
    .bind(q.filter_q())
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    let rows = sqlx::query_as::<_, PositionView>(
        "SELECT id, source_wallet, token_id, market_title, outcome, operating_mode, open_shares, avg_price, cost_basis_usd, realized_pnl_usd, updated_at
         FROM positions WHERE user_id = $1 AND open_shares > 0
           AND ($2::uuid IS NULL OR trading_account_id = $2)
           AND ($3::text IS NULL OR market_title ILIKE '%' || $3 || '%' OR outcome ILIKE '%' || $3 || '%' OR token_id ILIKE '%' || $3 || '%')
         ORDER BY updated_at DESC, id DESC
         LIMIT $4 OFFSET $5",
    )
    .bind(user.id)
    .bind(q.account_id)
    .bind(q.filter_q())
    .bind(q.limit())
    .bind(q.offset())
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(Paged {
        items: rows,
        total,
        limit: q.limit(),
        offset: q.offset(),
    }))
}

async fn list_closed(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM closed_positions WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let rows = sqlx::query_as::<_, ClosedRow>(
        "SELECT id, trading_account_id, source_wallet, token_id, market_title, operating_mode, closed_shares, realized_pnl_usd, closed_at
         FROM closed_positions WHERE user_id = $1 ORDER BY closed_at DESC, id DESC LIMIT $2 OFFSET $3",
    )
    .bind(user.id)
    .bind(q.limit())
    .bind(q.offset())
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    Ok(Json(Paged {
        items: rows,
        total,
        limit: q.limit(),
        offset: q.offset(),
    }))
}

async fn trade_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_user(&state, &headers, Some("user"))?;
    let mut tx = begin_as(&state.pool, Some(user.id), "user")
        .await
        .map_err(internal)?;
    let trade = sqlx::query_as::<_, TradeRow>(
        "SELECT e.id, e.source_wallet, e.tx_hash, e.token_id, e.condition_id, e.outcome, e.market_title, e.market_slug,
                e.event_title, e.event_slug, e.market_icon, e.activity_type, e.usdc_size,
                e.side, e.price, e.size_shares, e.notional_usd, e.event_ts, e.origin, e.status
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
    let polygonscan = trade
        .tx_hash
        .as_ref()
        .map(|tx| format!("https://polygonscan.com/tx/{tx}"));
    let market_url = crate::polymarket::polymarket_event_url(
        trade.event_slug.as_deref(),
        trade.market_slug.as_deref(),
    );
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

fn sizing_breakdown(
    decisions: &[DecisionRow],
    orders: &[OrderRow],
    fills: &[FillRow],
) -> Vec<serde_json::Value> {
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

async fn ws_upgrade(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
    Query(q): Query<WsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let claims = auth::read_access(&state.cfg.jwt_secret, &q.token)
        .map_err(|_| ApiError(StatusCode::UNAUTHORIZED, "invalid token".into()))?;
    let mut rx = state.activity.subscribe();
    Ok(ws.on_upgrade(move |socket| async move {
        let _ = pump(socket, &mut rx, claims.sub).await;
    }))
}

async fn pump(mut socket: WebSocket, rx: &mut broadcast::Receiver<String>, user_id: Uuid) {
    loop {
        match rx.recv().await {
            Ok(payload) => {
                let Ok(value) = serde_json::from_str::<Value>(&payload) else {
                    continue;
                };
                if value.get("user_id").and_then(|v| v.as_str())
                    == Some(user_id.to_string().as_str())
                    && socket.send(Message::Text(payload.into())).await.is_err()
                {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
}

fn require_user(
    state: &AppState,
    headers: &HeaderMap,
    role: Option<&str>,
) -> Result<AuthUser, ApiError> {
    let claims = bearer_user(state, headers)
        .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "authentication required".into()))?;
    if let Some(role) = role {
        if claims.role != role {
            return Err(ApiError(
                StatusCode::FORBIDDEN,
                "wrong role for this application".into(),
            ));
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

async fn invitation_delivery(
    mail: &Email,
    to_email: &str,
    display_name: &str,
    token: &str,
) -> Value {
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

fn dispatch_password_recovery_email(
    mail: Email,
    email: String,
    display_name: String,
    token: String,
) {
    tokio::spawn(async move {
        if let Err(err) = mail
            .send_password_recovery(&email, &display_name, &token)
            .await
        {
            Email::log_failure(&err);
        }
    });
}

fn dispatch_email_verification(mail: Email, email: String, display_name: String, token: String) {
    tokio::spawn(async move {
        if let Err(err) = mail
            .send_email_verification(&email, &display_name, &token)
            .await
        {
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
    if matches!(
        value,
        "eoa" | "poly_proxy" | "poly_gnosis_safe" | "poly_1271"
    ) {
        Ok(())
    } else {
        Err(bad(
            "signature_type must be eoa, poly_proxy, poly_gnosis_safe, or poly_1271",
        ))
    }
}

async fn store_key(
    vault: &Vault,
    tx: &mut crate::db::Tx<'_>,
    user_id: Uuid,
    account_id: Uuid,
    private_key: &str,
) -> anyhow::Result<()> {
    let aad = credential_aad(user_id, account_id);
    let payload = json!({"private_key": private_key}).to_string();
    let blob = vault.encrypt(payload.as_bytes(), &aad)?;
    let hint = private_key
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
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

async fn load_secret(
    vault: &Vault,
    tx: &mut crate::db::Tx<'_>,
    user_id: Uuid,
    account_id: Uuid,
) -> Result<Option<Value>, ApiError> {
    let row = sqlx::query_as::<_, CredRow>("SELECT ciphertext, nonce, key_id, aad FROM trading_credentials WHERE trading_account_id = $1 AND user_id = $2")
        .bind(account_id)
        .bind(user_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(internal)?;
    let Some(row) = row else { return Ok(None) };
    let plain = vault
        .decrypt(&SecretBlob {
            ciphertext: row.ciphertext,
            nonce: row.nonce,
            key_id: row.key_id,
            aad: row.aad,
        })
        .map_err(|_| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "stored credentials could not be decrypted".into(),
            )
        })?;
    Ok(Some(serde_json::from_slice(&plain).unwrap_or(json!({}))))
}

async fn credential_exists(tx: &mut crate::db::Tx<'_>, account_id: Uuid) -> Result<bool, ApiError> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM trading_credentials WHERE trading_account_id = $1)",
    )
    .bind(account_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(internal)
}

async fn load_account(
    tx: &mut crate::db::Tx<'_>,
    user_id: Uuid,
    id: Uuid,
) -> Result<AccountCard, ApiError> {
    sqlx::query_as::<_, AccountCard>(
        "SELECT id, name, signer_address, funder_address, signature_type, operating_mode, status, emergency_stop, trading_control_state, last_exchange_balance_usd, last_balance_checked_at, last_connection_error
         FROM trading_accounts WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(internal)?
    .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "account not found".into()))
}

async fn one_strategy(
    tx: &mut crate::db::Tx<'_>,
    user_id: Uuid,
    id: Uuid,
) -> Result<StrategyView, ApiError> {
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
    trading_control_state: String,
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
    event_slug: Option<String>,
    market_icon: Option<String>,
    activity_type: String,
    usdc_size: Option<Decimal>,
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
struct OrderListRow {
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
    market_title: Option<String>,
    outcome: Option<String>,
    market_icon: Option<String>,
    market_slug: Option<String>,
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
struct FillListRow {
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
    market_title: Option<String>,
    outcome: Option<String>,
    market_icon: Option<String>,
    order_side: Option<String>,
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
