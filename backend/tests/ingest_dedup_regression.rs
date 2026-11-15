//! Regression: source event dedup across Data API / RTDS (scenarios A–E).
//!
//! Run: `cargo test --test ingest_dedup_regression -- --ignored --test-threads=1`
//! (shared `pommarket_test`; parallel tests would race on truncate).
mod test_support;

use std::env;

use pommarket::db::{begin_as, connect, migrate};
use pommarket::ensure_login_roles;
use pommarket::pipeline::ingest;
use pommarket::polymarket::parse_activity_payload;
use rust_decimal::Decimal;
use serde_json::json;
use test_support::{assert_all_test_database_urls, truncate_integration_tables};

fn trade_json(
    tx: &str,
    side: &str,
    size: &str,
    price: &str,
    token: &str,
    ts: i64,
) -> serde_json::Value {
    json!({
        "proxyWallet": "0x2222222222222222222222222222222222222222",
        "transactionHash": tx,
        "token_id": token,
        "side": side,
        "price": price,
        "size": size,
        "timestamp": ts,
        "title": "Dedup regression",
        "usdcSize": (size.parse::<f64>().unwrap() * price.parse::<f64>().unwrap())
    })
}

fn parse_one(payload: &serde_json::Value, origin: &str) -> pommarket::polymarket::RawTrade {
    parse_activity_payload(payload, origin)
        .into_iter()
        .next()
        .expect("one trade")
}

async fn setup_worker() -> sqlx::PgPool {
    assert_all_test_database_urls();
    let migration_url = env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&migration_url).await.unwrap();
    let migration = connect(&migration_url).await.unwrap();
    ensure_login_roles(
        &migration,
        &env::var("APP_DB_PASSWORD").unwrap(),
        &env::var("WORKER_DB_PASSWORD").unwrap(),
    )
    .await
    .unwrap();
    truncate_integration_tables(&migration, &migration_url).await;
    connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap()
}

async fn event_metrics(pool: &sqlx::PgPool, event_id: uuid::Uuid) -> (Decimal, Decimal, i32, i64) {
    let mut tx = begin_as(pool, None, "worker").await.unwrap();
    let row: (Decimal, Option<Decimal>, i32) = sqlx::query_as(
        "SELECT size_shares, usdc_size, fragment_count FROM source_events WHERE id = $1",
    )
    .bind(event_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let fragments: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM source_event_fragments WHERE source_event_id = $1",
    )
    .bind(event_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    (
        row.0,
        row.1.unwrap_or(Decimal::ZERO),
        row.2,
        fragments,
    )
}

/// Scenario A — one trade from Data API and RTDS.
#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn scenario_a_data_api_and_rtds_single_trade() {
    let worker = setup_worker().await;
    let payload = trade_json("0xaaaa", "BUY", "1000", "0.89", "token-a", 1_700_000_000);
    let api = parse_one(&payload, "data_api");
    let rtds = parse_one(&payload, "rtds");
    assert_eq!(api.dedup_key, rtds.dedup_key);
    assert_eq!(api.fragment_key, rtds.fragment_key);

    let first = ingest(&worker, &api).await.unwrap();
    assert!(first.created);
    let second = ingest(&worker, &rtds).await.unwrap();
    assert_eq!(second.event_id, first.event_id);
    assert!(!second.created);
    assert!(second.duplicate_fragment);

    let (shares, usdc, frag_count, frag_rows) =
        event_metrics(&worker, first.event_id).await;
    assert_eq!(shares, Decimal::from(1000));
    assert_eq!(usdc, Decimal::new(890, 0));
    assert_eq!(frag_count, 1);
    assert_eq!(frag_rows, 1);
}

/// Scenario B — two distinct txs with identical price/size/timestamp.
#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn scenario_b_two_real_trades_same_price_size() {
    let worker = setup_worker().await;
    let ts = 1_700_000_111_i64;
    let t1 = parse_one(&trade_json("0xbbbb1", "buy", "500", "0.69", "tok-b", ts), "data_api");
    let t2 = parse_one(&trade_json("0xbbbb2", "buy", "500", "0.69", "tok-b", ts), "data_api");
    assert_ne!(t1.dedup_key, t2.dedup_key);

    let o1 = ingest(&worker, &t1).await.unwrap();
    let o2 = ingest(&worker, &t2).await.unwrap();
    assert!(o1.created);
    assert!(o2.created);
    assert_ne!(o1.event_id, o2.event_id);

    let mut tx = begin_as(&worker, None, "worker").await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM source_events")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(count, 2);
}

/// Scenario C — repeated delivery idempotent.
#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn scenario_c_redelivery_idempotent() {
    let worker = setup_worker().await;
    let trade = parse_one(
        &trade_json("0xcccc", "buy", "7", "0.5", "tok-c", 1_700_000_222),
        "rtds",
    );
    let a = ingest(&worker, &trade).await.unwrap();
    for _ in 0..5 {
        let again = ingest(&worker, &trade).await.unwrap();
        assert_eq!(again.event_id, a.event_id);
        assert!(again.duplicate_fragment);
    }
    let (shares, _, _, frags) = event_metrics(&worker, a.event_id).await;
    assert_eq!(shares, Decimal::from(7));
    assert_eq!(frags, 1);
}

/// Scenario D — one tx, two real partial fills (different size at same price).
#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn scenario_d_partial_fragments_aggregate() {
    let worker = setup_worker().await;
    let tx = "0xdddd";
    let price = "0.42";
    let f1 = parse_one(&trade_json(tx, "buy", "10", price, "tok-d", 1_700_000_333), "data_api");
    let f2 = parse_one(&trade_json(tx, "buy", "3", price, "tok-d", 1_700_000_333), "data_api");
    assert_eq!(f1.dedup_key, f2.dedup_key);
    assert_ne!(f1.fragment_key, f2.fragment_key);

    let first = ingest(&worker, &f1).await.unwrap();
    let second = ingest(&worker, &f2).await.unwrap();
    assert_eq!(second.event_id, first.event_id);
    assert!(!second.duplicate_fragment);

    let (shares, _, frag_count, frag_rows) = event_metrics(&worker, first.event_id).await;
    assert_eq!(shares, Decimal::from(13));
    assert_eq!(frag_count, 2);
    assert_eq!(frag_rows, 2);

    let dup = ingest(&worker, &f2).await.unwrap();
    assert!(dup.duplicate_fragment);
    let (shares2, _, _, frags2) = event_metrics(&worker, first.event_id).await;
    assert_eq!(shares2, shares);
    assert_eq!(frags2, 2);
}

/// Scenario E — BUY and SELL never merge.
#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn scenario_e_buy_and_sell_separate_events() {
    let worker = setup_worker().await;
    let tx = "0xeeee";
    let buy = parse_one(&trade_json(tx, "buy", "100", "0.55", "tok-e", 1_700_000_444), "data_api");
    let sell = parse_one(&trade_json(tx, "sell", "100", "0.55", "tok-e", 1_700_000_444), "data_api");
    assert_ne!(buy.dedup_key, sell.dedup_key);

    let ob = ingest(&worker, &buy).await.unwrap();
    let os = ingest(&worker, &sell).await.unwrap();
    assert_ne!(ob.event_id, os.event_id);
}
