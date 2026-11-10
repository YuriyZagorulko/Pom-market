use std::net::SocketAddr;

use axum::http::{HeaderValue, Method};
use sqlx::postgres::PgListener;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

use pommarket::api::{new_app_state, router};
use pommarket::config::Config;
use pommarket::db::{connect, migrate};
use pommarket::polymarket::http::PublicClient;
use pommarket::security::Vault;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = Config::from_env()?;
    pommarket::init_tracing(&cfg.log_format);
    migrate(&cfg.migration_database_url).await?;
    let migration = connect(&cfg.migration_database_url).await?;
    let app_password = std::env::var("APP_DB_PASSWORD")
        .map_err(|_| anyhow::anyhow!("missing required environment variable APP_DB_PASSWORD"))?;
    let worker_password = std::env::var("WORKER_DB_PASSWORD")
        .map_err(|_| anyhow::anyhow!("missing required environment variable WORKER_DB_PASSWORD"))?;
    pommarket::ensure_login_roles(&migration, &app_password, &worker_password).await?;
    pommarket::bootstrap_admin(&migration, &cfg).await?;
    drop(migration);
    if std::env::args().any(|arg| arg == "--rotate-keys") {
        let pool = connect(&cfg.worker_database_url).await?;
        let vault = Vault::new(
            &cfg.encryption_master_key,
            &cfg.encryption_key_id,
            cfg.previous_encryption_key.as_deref(),
            cfg.previous_encryption_key_id.as_deref(),
        )?;
        let count = pommarket::rotate_encryption_keys(&pool, &vault).await?;
        println!("rotated {count} credential records to {}", vault.key_id());
        return Ok(());
    }
    let pool = connect(&cfg.app_database_url).await?;
    let (activity, _) = broadcast::channel(256);
    let listener_url = cfg.app_database_url.clone();
    let activity_tx = activity.clone();
    tokio::spawn(async move {
        if let Err(err) = listen(listener_url, activity_tx).await {
            tracing::error!(error = %pommarket::security::redact(&err.to_string()), "activity listener stopped");
        }
    });
    let mail = pommarket::email::Email::from_config(&cfg)?;
    let state = new_app_state(
        pool,
        cfg.clone(),
        Vault::new(
            &cfg.encryption_master_key,
            &cfg.encryption_key_id,
            cfg.previous_encryption_key.as_deref(),
            cfg.previous_encryption_key_id.as_deref(),
        )?,
        PublicClient::new(
            cfg.data_api_host.clone(),
            cfg.clob_host.clone(),
            cfg.gamma_host.clone(),
        ),
        activity,
        mail,
    );
    let mut cors = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
        ])
        .allow_credentials(true);
    let mut origins = Vec::new();
    for origin in cfg.cors_origins() {
        origins.push(HeaderValue::from_str(&origin)?);
    }
    cors = cors.allow_origin(origins);
    let app = router(state)
        .layer(TraceLayer::new_for_http())
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(cors);
    let listener = tokio::net::TcpListener::bind(&cfg.api_bind).await?;
    tracing::info!(bind = %cfg.api_bind, "api listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn listen(url: String, tx: broadcast::Sender<String>) -> anyhow::Result<()> {
    let mut listener = PgListener::connect(&url).await?;
    listener.listen("pom_activity").await?;
    loop {
        let note = listener.recv().await?;
        let _ = tx.send(note.payload().to_string());
    }
}
