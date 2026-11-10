pub mod api;
pub mod auth;
pub mod client_ip;
pub mod config;
pub mod copy_preview;
pub mod db;
pub mod email;
pub mod engine;
pub mod hard_close;
pub mod pipeline;
pub mod polymarket;
pub mod rate_limit;
pub mod security;
pub mod strategy_budget;
pub mod trading_control;

use chrono::Utc;
use sqlx::PgPool;

pub fn init_tracing(format: &str) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,sqlx=warn".into());
    if format == "json" {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
}

pub async fn ensure_login_roles(
    pool: &PgPool,
    app_password: &str,
    worker_password: &str,
) -> anyhow::Result<()> {
    for attempt in 0..5 {
        match alter_login_roles(pool, app_password, worker_password).await {
            Ok(()) => return Ok(()),
            Err(err) if attempt < 4 && err.to_string().contains("tuple concurrently updated") => {
                tokio::time::sleep(std::time::Duration::from_millis(200 * (attempt + 1))).await;
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

async fn alter_login_roles(
    pool: &PgPool,
    app_password: &str,
    worker_password: &str,
) -> anyhow::Result<()> {
    for (role, password) in [
        ("pommarket_app", app_password),
        ("pommarket_worker", worker_password),
    ] {
        let statement: String = sqlx::query_scalar(
            "SELECT format('ALTER ROLE %I LOGIN PASSWORD %L', $1::text, $2::text)",
        )
        .bind(role)
        .bind(password)
        .fetch_one(pool)
        .await?;
        sqlx::query(&statement).execute(pool).await?;
    }
    Ok(())
}

pub async fn bootstrap_admin(pool: &PgPool, cfg: &config::Config) -> anyhow::Result<()> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE role = 'admin')")
            .fetch_one(pool)
            .await?;
    if exists {
        return Ok(());
    }
    let (Some(email), Some(password)) = (&cfg.bootstrap_admin_email, &cfg.bootstrap_admin_password)
    else {
        tracing::warn!("no administrator exists and BOOTSTRAP_ADMIN_EMAIL / BOOTSTRAP_ADMIN_PASSWORD are unset");
        return Ok(());
    };
    let hash = security::hash_password(password)?;
    sqlx::query(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES (lower($1), $2, $3, 'admin', 'active', now())",
    )
    .bind(email.trim())
    .bind(&cfg.bootstrap_admin_name)
    .bind(hash)
    .execute(pool)
    .await?;
    tracing::info!(email = %email.trim().to_lowercase(), "bootstrap administrator created");
    Ok(())
}

pub async fn rotate_encryption_keys(pool: &PgPool, vault: &security::Vault) -> anyhow::Result<u64> {
    let mut tx = db::begin_as(pool, None, "worker").await?;
    let rows = sqlx::query_as::<_, Cred>("SELECT trading_account_id, user_id, ciphertext, nonce, key_id, aad FROM trading_credentials")
        .fetch_all(&mut *tx)
        .await?;
    let mut rotated = 0u64;
    for row in rows {
        if row.key_id == vault.key_id() {
            continue;
        }
        let plain = vault.decrypt(&security::SecretBlob {
            ciphertext: row.ciphertext,
            nonce: row.nonce,
            key_id: row.key_id,
            aad: row.aad.clone(),
        })?;
        let blob = vault.encrypt(&plain, &row.aad)?;
        sqlx::query("UPDATE trading_credentials SET ciphertext = $2, nonce = $3, key_id = $4, updated_at = now() WHERE trading_account_id = $1")
            .bind(row.trading_account_id)
            .bind(blob.ciphertext)
            .bind(blob.nonce)
            .bind(blob.key_id)
            .execute(&mut *tx)
            .await?;
        rotated += 1;
    }
    tx.commit().await?;
    let _ = Utc::now();
    Ok(rotated)
}

#[derive(sqlx::FromRow)]
struct Cred {
    trading_account_id: uuid::Uuid,
    #[allow(dead_code)] // selected for credential_aad and future audit fields
    user_id: uuid::Uuid,
    ciphertext: Vec<u8>,
    nonce: Vec<u8>,
    key_id: String,
    aad: String,
}
