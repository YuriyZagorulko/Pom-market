mod test_support;

use std::env;

use chrono::Utc;
use pommarket::db::{begin_as, connect, migrate};
use pommarket::ensure_login_roles;
use pommarket::pipeline::ingest;
use pommarket::polymarket::RawTrade;
use rust_decimal::Decimal;
use serde_json::json;
use test_support::{assert_all_test_database_urls, truncate_integration_tables};

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn ingest_dedup_four_scenarios() {
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

    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();

    let base = sample_trade("buy", "10", "scenario-four");
    let first = ingest(&worker, &base).await.unwrap();
    assert!(first.created);
    assert!(!first.duplicate_fragment);

    let repeat = ingest(&worker, &base).await.unwrap();
    assert_eq!(repeat.event_id, first.event_id);
    assert!(!repeat.created);
    assert!(repeat.duplicate_fragment);

    let fragment_b = sample_trade("buy", "5", "scenario-four");
    let updated = ingest(&worker, &fragment_b).await.unwrap();
    assert_eq!(updated.event_id, first.event_id);
    assert!(!updated.created);
    assert!(!updated.duplicate_fragment);

    let mut tx = begin_as(&worker, None, "worker").await.unwrap();
    let size: Decimal = sqlx::query_scalar("SELECT size_shares FROM source_events WHERE id = $1")
        .bind(first.event_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(size > Decimal::new(10, 0));

    let repeat_fragment = ingest(&worker, &fragment_b).await.unwrap();
    assert_eq!(repeat_fragment.event_id, first.event_id);
    assert!(repeat_fragment.duplicate_fragment);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn concurrent_dedup_ingestion_is_safe() {
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

    let worker = connect(&env::var("WORKER_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let trade = sample_trade("buy", "8", "concurrent");

    let mut handles = Vec::new();
    for _ in 0..12 {
        let pool = worker.clone();
        let trade = trade.clone();
        handles.push(tokio::spawn(
            async move { ingest(&pool, &trade).await.unwrap() },
        ));
    }
    let mut outcomes = Vec::with_capacity(handles.len());
    for handle in handles {
        outcomes.push(handle.await.expect("ingest task"));
    }

    let created = outcomes.iter().filter(|o| o.created).count();
    assert_eq!(
        created, 1,
        "exactly one concurrent insert should create the event"
    );
    let event_id = outcomes[0].event_id;
    assert!(outcomes.iter().all(|o| o.event_id == event_id));

    let mut tx = begin_as(&worker, None, "worker").await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM source_events")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 1);

    let fragments: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM source_event_fragments WHERE source_event_id = $1",
    )
    .bind(event_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(fragments, 1);

    // Sequential replay remains idempotent.
    let again = ingest(&worker, &trade).await.unwrap();
    assert!(!again.created);
    assert!(again.duplicate_fragment);
}

fn sample_trade(side: &str, size: &str, tag: &str) -> RawTrade {
    let payload = json!({
        "proxyWallet": "0x2222222222222222222222222222222222222222",
        "transactionHash": format!("0x{tag}"),
        "asset": "55",
        "conditionId": "cond",
        "outcome": "Yes",
        "outcomeIndex": 0,
        "side": side,
        "price": 0.5,
        "size": size,
        "timestamp": Utc::now().timestamp(),
        "title": "Dedup test",
        "slug": "dedup-test"
    });
    pommarket::polymarket::parse_activity_payload(&payload, "integration-test")
        .pop()
        .unwrap()
}
