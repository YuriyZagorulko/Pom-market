use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::security::{hash_password, random_token, sha256_hex, verify_password};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccessClaims {
    pub sub: Uuid,
    pub sid: Uuid,
    pub role: String,
    pub exp: i64,
}

#[derive(Clone, Debug)]
pub struct AuthUser {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub status: String,
}

#[derive(Clone, Debug)]
pub struct LoginResult {
    pub user: AuthUser,
    pub access_token: String,
    pub refresh_token: String,
    pub session_id: Uuid,
}

#[derive(Clone, Debug)]
pub struct LoginSecurityAlert {
    pub email: String,
    pub display_name: String,
}

#[derive(Clone, Debug)]
pub struct LoginAttempt {
    pub result: Result<LoginResult, &'static str>,
    pub security_alert: Option<LoginSecurityAlert>,
}

pub fn issue_access(
    secret: &str,
    user: &AuthUser,
    session_id: Uuid,
    ttl_seconds: i64,
) -> anyhow::Result<String> {
    let claims = AccessClaims {
        sub: user.id,
        sid: session_id,
        role: user.role.clone(),
        exp: (Utc::now() + Duration::seconds(ttl_seconds)).timestamp(),
    };
    Ok(encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?)
}

pub fn read_access(secret: &str, token: &str) -> anyhow::Result<AccessClaims> {
    let data = decode::<AccessClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;
    Ok(data.claims)
}

#[allow(clippy::too_many_arguments)] // login audit metadata is explicit per caller
pub async fn login(
    pool: &PgPool,
    secret: &str,
    email: &str,
    password: &str,
    user_agent: Option<String>,
    ip: Option<String>,
    access_ttl: i64,
    refresh_ttl: i64,
) -> LoginAttempt {
    let row = match sqlx::query_as::<_, AuthRow>(
        "SELECT id, email, display_name, password_hash, role, status, failed_login_count, locked_until
         FROM auth_user_by_email($1)",
    )
    .bind(email.trim())
    .fetch_optional(pool)
    .await
    {
        Ok(row) => row,
        Err(_) => {
            return LoginAttempt {
                result: Err("invalid credentials"),
                security_alert: None,
            }
        }
    };
    let Some(row) = row else {
        return LoginAttempt {
            result: Err("invalid credentials"),
            security_alert: None,
        };
    };
    if row.status == "suspended" {
        return LoginAttempt {
            result: Err("account suspended"),
            security_alert: None,
        };
    }
    if row.locked_until.is_some_and(|until| until > Utc::now()) {
        return LoginAttempt {
            result: Err("account temporarily locked"),
            security_alert: None,
        };
    }
    let Some(hash) = row.password_hash.as_deref() else {
        return LoginAttempt {
            result: Err("password is not set"),
            security_alert: None,
        };
    };
    if !verify_password(password, hash) {
        let prior_failures = row.failed_login_count;
        let failure =
            sqlx::query_as::<_, FailureRow>("SELECT * FROM auth_record_login_failure($1)")
                .bind(row.id)
                .fetch_one(pool)
                .await;
        let security_alert = match failure {
            Ok(outcome) => {
                if outcome.failed_login_count >= 8
                    && prior_failures < 8
                    && outcome.locked_until.is_some_and(|until| until > Utc::now())
                {
                    Some(LoginSecurityAlert {
                        email: row.email.clone(),
                        display_name: row.display_name.clone(),
                    })
                } else {
                    None
                }
            }
            Err(_) => None,
        };
        return LoginAttempt {
            result: Err("invalid credentials"),
            security_alert,
        };
    }
    if row.status != "active" {
        return LoginAttempt {
            result: Err("account is not active"),
            security_alert: None,
        };
    }
    if sqlx::query("SELECT auth_record_login_success($1)")
        .bind(row.id)
        .execute(pool)
        .await
        .is_err()
    {
        return LoginAttempt {
            result: Err("invalid credentials"),
            security_alert: None,
        };
    }
    let refresh = random_token();
    let session_id =
        match sqlx::query_scalar::<_, Uuid>("SELECT auth_insert_session($1, $2, $3, $4, $5)")
            .bind(row.id)
            .bind(sha256_hex(&refresh))
            .bind(Utc::now() + Duration::seconds(refresh_ttl))
            .bind(user_agent)
            .bind(ip)
            .fetch_one(pool)
            .await
        {
            Ok(session_id) => session_id,
            Err(_) => {
                return LoginAttempt {
                    result: Err("invalid credentials"),
                    security_alert: None,
                }
            }
        };
    let user = row.into_user();
    let access_token = match issue_access(secret, &user, session_id, access_ttl) {
        Ok(token) => token,
        Err(_) => {
            return LoginAttempt {
                result: Err("invalid credentials"),
                security_alert: None,
            }
        }
    };
    LoginAttempt {
        result: Ok(LoginResult {
            user,
            access_token,
            refresh_token: refresh,
            session_id,
        }),
        security_alert: None,
    }
}

pub async fn refresh(
    pool: &PgPool,
    secret: &str,
    refresh_token: &str,
    access_ttl: i64,
    refresh_ttl: i64,
) -> anyhow::Result<LoginResult> {
    let row = sqlx::query_as::<_, SessionRow>(
        "SELECT session_id, user_id, expires_at, revoked_at, role, status, email, display_name
         FROM auth_session_by_hash($1)",
    )
    .bind(sha256_hex(refresh_token))
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        anyhow::bail!("invalid session");
    };
    if row.revoked_at.is_some() || row.expires_at < Utc::now() || row.status != "active" {
        anyhow::bail!("invalid session");
    }
    sqlx::query("SELECT auth_revoke_session($1)")
        .bind(row.session_id)
        .execute(pool)
        .await?;
    let user = AuthUser {
        id: row.user_id,
        email: row.email,
        display_name: row.display_name,
        role: row.role,
        status: row.status,
    };
    let refresh_token = random_token();
    let session_id: Uuid = sqlx::query_scalar("SELECT auth_insert_session($1, $2, $3, $4, $5)")
        .bind(user.id)
        .bind(sha256_hex(&refresh_token))
        .bind(Utc::now() + Duration::seconds(refresh_ttl))
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .fetch_one(pool)
        .await?;
    let access_token = issue_access(secret, &user, session_id, access_ttl)?;
    Ok(LoginResult {
        user,
        access_token,
        refresh_token,
        session_id,
    })
}

pub async fn logout(pool: &PgPool, session_id: Uuid) -> anyhow::Result<()> {
    sqlx::query("SELECT auth_revoke_session($1)")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn consume_invitation(
    pool: &PgPool,
    token: &str,
    password: &str,
) -> anyhow::Result<String> {
    let row = sqlx::query_as::<_, InviteRow>(
        "SELECT token_id, user_id, purpose, expires_at, used_at, email, status FROM auth_invitation_by_hash($1)",
    )
    .bind(sha256_hex(token))
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        anyhow::bail!("invitation not found");
    };
    if row.purpose == "verify" {
        anyhow::bail!("invitation not found");
    }
    if row.used_at.is_some() || row.expires_at < Utc::now() {
        anyhow::bail!("invitation is no longer valid");
    }
    let password_hash = hash_password(password)?;
    sqlx::query("SELECT auth_consume_invitation($1, $2, $3)")
        .bind(row.token_id)
        .bind(row.user_id)
        .bind(password_hash)
        .execute(pool)
        .await?;
    Ok(row.email)
}

pub async fn consume_password_reset(
    pool: &PgPool,
    token: &str,
    password: &str,
) -> anyhow::Result<String> {
    let row = sqlx::query_as::<_, InviteRow>(
        "SELECT token_id, user_id, purpose, expires_at, used_at, email, status FROM auth_invitation_by_hash($1)",
    )
    .bind(sha256_hex(token))
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        anyhow::bail!("reset token not found");
    };
    if row.purpose != "reset" {
        anyhow::bail!("reset token not found");
    }
    if row.used_at.is_some() {
        anyhow::bail!("reset token has already been used");
    }
    if row.expires_at < Utc::now() {
        anyhow::bail!("reset token is no longer valid");
    }
    if row.status != "active" {
        anyhow::bail!("reset token is no longer valid");
    }
    let password_hash = hash_password(password)?;
    sqlx::query("SELECT auth_consume_password_reset($1, $2, $3)")
        .bind(row.token_id)
        .bind(row.user_id)
        .bind(password_hash)
        .execute(pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("reset token is no longer valid") {
                anyhow::anyhow!("reset token is no longer valid")
            } else {
                e.into()
            }
        })?;
    Ok(row.email)
}

pub async fn consume_verification(pool: &PgPool, token: &str) -> anyhow::Result<String> {
    let row = sqlx::query_as::<_, InviteRow>(
        "SELECT token_id, user_id, purpose, expires_at, used_at, email, status FROM auth_invitation_by_hash($1)",
    )
    .bind(sha256_hex(token))
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        anyhow::bail!("verification token not found");
    };
    if row.purpose != "verify" {
        anyhow::bail!("verification token not found");
    }
    if row.used_at.is_some() || row.expires_at < Utc::now() {
        anyhow::bail!("verification token is no longer valid");
    }
    sqlx::query("SELECT auth_consume_verification($1, $2)")
        .bind(row.token_id)
        .bind(row.user_id)
        .execute(pool)
        .await?;
    Ok(row.email)
}

#[derive(sqlx::FromRow)]
struct AuthRow {
    id: Uuid,
    email: String,
    display_name: String,
    password_hash: Option<String>,
    role: String,
    status: String,
    failed_login_count: i32,
    locked_until: Option<chrono::DateTime<Utc>>,
}

impl AuthRow {
    fn into_user(self) -> AuthUser {
        AuthUser {
            id: self.id,
            email: self.email,
            display_name: self.display_name,
            role: self.role,
            status: self.status,
        }
    }
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    session_id: Uuid,
    user_id: Uuid,
    expires_at: chrono::DateTime<Utc>,
    revoked_at: Option<chrono::DateTime<Utc>>,
    role: String,
    status: String,
    email: String,
    display_name: String,
}

#[derive(sqlx::FromRow)]
struct FailureRow {
    failed_login_count: i32,
    locked_until: Option<chrono::DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct InviteRow {
    token_id: Uuid,
    user_id: Uuid,
    purpose: String,
    expires_at: chrono::DateTime<Utc>,
    used_at: Option<chrono::DateTime<Utc>>,
    email: String,
    status: String,
}
