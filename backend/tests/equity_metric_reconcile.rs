//! Independent equity_metric check vs strategy analytics snapshot builder.

mod test_support;

use std::env;

use pommarket::data_api_ingest::independent_equity_metric;
use pommarket::db::{begin_as, connect, migrate};
use rust_decimal::Decimal;
use test_support::{bootstrap_paper_capital_for_account, truncate_integration_tables};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn equity_metric_matches_capital_plus_positions_mtm() {
    test_support::assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    pommarket::bootstrap_admin(&migration, &pommarket::config::Config::from_env().unwrap())
        .await
        .unwrap();

    let cfg = pommarket::config::Config::from_env().unwrap();
    let worker = connect(&cfg.worker_database_url).await.unwrap();

    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ('equity@example.com','E','x','user','active',now()) RETURNING id",
    )
    .fetch_one(&migration)
    .await
    .unwrap();
    let account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signer_address, funder_address, signature_type, operating_mode, status)
         VALUES ($1,'Paper','0xeq00000000000000000000000000000000001','0xeq00000000000000000000000000000000001','eoa','paper_trading','connected') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_risk_limits (trading_account_id, user_id, max_order_usd, max_daily_loss_usd, max_exposure_usd,
         max_slippage_bps, max_orders_per_interval, interval_seconds, max_stale_data_seconds, paper_cash_usd)
         VALUES ($1,$2,500,2000,5000,500,100,60,3600,50000)",
    )
    .bind(account_id)
    .bind(user_id)
    .execute(&migration)
    .await
    .unwrap();
    let trader_id: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias, monitoring_enabled)
         VALUES ($1,'0xleader00000000000000000000000000000001','Leader',true) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&migration)
    .await
    .unwrap();
    let strategy_id: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (
            user_id, trading_account_id, source_trader_id, enabled, sizing_mode, fixed_amount_usd,
            strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions, max_slippage_bps, min_order_usd, copy_buy, copy_sell
         ) VALUES ($1,$2,$3,true,'fixed_amount',10,1000,500,5000,5,500,1,true,true) RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(trader_id)
    .fetch_one(&migration)
    .await
    .unwrap();

    bootstrap_paper_capital_for_account(&worker, account_id).await;

    let mut tx = begin_as(&worker, Some(user_id), "user").await.unwrap();
    sqlx::query(
        "INSERT INTO positions (
            user_id, trading_account_id, strategy_id, source_wallet, token_id, condition_id, outcome,
            market_title, operating_mode, open_shares, avg_price, cost_basis_usd, source_observed_shares
         ) VALUES ($1,$2,$3,'0xleader00000000000000000000000000000001','tok-eq','0xc','Yes','EQ','paper_trading',100,0.5,50,100)",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(strategy_id)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let metrics =
        pommarket::strategy_analytics::compute_metrics(&worker, user_id, strategy_id, &cfg)
            .await
            .unwrap();

    let independent = independent_equity_metric(
        metrics.available_capital_usd,
        metrics.reserved_capital_usd,
        metrics.positions_market_value_usd,
    );
    assert_eq!(
        independent,
        metrics.equity.strategy_equity_usd,
        "equity_metric should equal available + reserved + positions MTM"
    );
    assert!(
        metrics.mark_quality == "cost_basis_estimate" || metrics.positions_market_value_usd > Decimal::ZERO
    );
}
