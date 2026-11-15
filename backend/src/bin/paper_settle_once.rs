//! One-shot PAPER resolution settlement for dev/diagnostics (run locally).

use pommarket::config::Config;
use pommarket::db::{connect, migrate};
use pommarket::paper_settlement;
use pommarket::polymarket::http::PublicClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = Config::from_env()?;
    pommarket::init_tracing(&cfg.log_format);
    migrate(&cfg.migration_database_url).await?;
    let pool = connect(&cfg.worker_database_url).await?;
    let client = PublicClient::new(
        cfg.data_api_host.clone(),
        cfg.clob_host.clone(),
        cfg.gamma_host.clone(),
    );
    let limit: i64 = std::env::var("PAPER_SETTLE_LIMIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    let results = paper_settlement::process_open_positions(&pool, &client, limit).await?;
    for r in &results {
        println!(
            "{} {} payout={} cost={} pnl={} {}",
            r.position_id, r.status, r.payout_usd, r.cost_released, r.pnl_usd, r.detail
        );
    }
    println!("settled_or_checked={}", results.len());
    Ok(())
}
