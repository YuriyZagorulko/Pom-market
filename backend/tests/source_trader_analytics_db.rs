//! Source trader analytics aggregates from cached trades (pommarket_test).

mod test_support;

use chrono::{Duration, Utc};
use pommarket::db::{connect, migrate};
use pommarket::source_trader_analytics::{build_dashboard, ensure_sync_row, resolve_trader_period};
use rust_decimal::Decimal;
use std::str::FromStr;
use test_support::assert_test_database_url;
use uuid::Uuid;

async fn seed_trader(pool: &sqlx::PgPool) -> (Uuid, Uuid) {
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ($1,'STA','x','user','active',now()) RETURNING id",
    )
    .bind(format!("sta-{}@example.com", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .unwrap();
    let trader: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias) VALUES ($1,$2,'Stats') RETURNING id",
    )
    .bind(user)
    .bind("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    .fetch_one(pool)
    .await
    .unwrap();
    ensure_sync_row(pool, user, trader).await.unwrap();
    sqlx::query(
        "UPDATE source_trader_analytics_sync SET status = 'ready', sync_phase = 'complete', profile_stats = $2
         WHERE source_trader_id = $1",
    )
    .bind(trader)
    .bind(serde_json::json!({"data": {"trades": 2, "all_time_pnl": {"volume_usdc": "150"}}}))
    .execute(pool)
    .await
    .unwrap();
    (user, trader)
}

async fn insert_trade(
    pool: &sqlx::PgPool,
    trader: Uuid,
    side: &str,
    notional: &str,
    ts_offset_hours: i64,
    dedup: &str,
) {
    let ts = Utc::now() - Duration::hours(ts_offset_hours);
    sqlx::query(
        "INSERT INTO source_trader_analytics_trades (
            source_trader_id, dedup_key, event_ts, side, notional_usd, price, size_shares
         ) VALUES ($1,$2,$3,$4,$5,0.5,10)
         ON CONFLICT (source_trader_id, dedup_key) DO NOTHING",
    )
    .bind(trader)
    .bind(dedup)
    .bind(ts)
    .bind(side)
    .bind(Decimal::from_str(notional).unwrap())
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn period_metrics_buy_sell_and_dedup() {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&url);
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let (user, trader) = seed_trader(&pool).await;
    insert_trade(&pool, trader, "buy", "25", 2, "k1").await;
    insert_trade(&pool, trader, "buy", "75", 1, "k1").await;
    insert_trade(&pool, trader, "sell", "50", 1, "k2").await;

    let dash = build_dashboard(&pool, user, trader, Some("7D"), None, None)
        .await
        .unwrap();
    let m = &dash.metrics;
    assert_eq!(m.total_trades, 2, "duplicate dedup_key must not double-count");
    assert_eq!(m.buy_count, 1);
    assert_eq!(m.sell_count, 1);
    assert_eq!(m.trading_volume_usd, Decimal::from(125));
    assert_eq!(m.buy_stats.median_usd, Some(Decimal::from(75)));
    assert_eq!(m.sell_stats.median_usd, Some(Decimal::from(50)));
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn custom_period_window() {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&url);
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let (user, trader) = seed_trader(&pool).await;
    insert_trade(&pool, trader, "buy", "10", 48, "old").await;
    insert_trade(&pool, trader, "buy", "20", 1, "new").await;

    let end = Utc::now();
    let start = end - Duration::hours(6);
    let dash = build_dashboard(
        &pool,
        user,
        trader,
        None,
        Some(&start.to_rfc3339()),
        Some(&end.to_rfc3339()),
    )
    .await
    .unwrap();
    assert_eq!(dash.metrics.total_trades, 1);
    assert_eq!(dash.period, "custom");
}

#[test]
fn resolve_90d_label() {
    let (_, _, label) = resolve_trader_period(Some("90D"), None, None).unwrap();
    assert_eq!(label, "90D");
}