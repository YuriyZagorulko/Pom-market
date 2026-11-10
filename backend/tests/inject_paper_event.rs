//! Safe local paper-event inject for E2E (pommarket_e2e) or primary when opted in.
//!
//!   E2E_MODE=1 / E2E_DATABASE_OK=1 → pommarket_e2e only
//!   E2E_PRIMARY_OK=1 → legacy primary pommarket
//!   ENCRYPTION_MASTER_KEY=... ENCRYPTION_KEY_ID=k1 \
//!   INJECT_WALLET=0x... INJECT_SIDE=buy INJECT_TX_SUFFIX=playwright1 \
//!   cargo test -p pommarket --test inject_paper_event inject_trade -- --ignored --nocapture

mod common;
mod test_support;

use std::env;

use common::TestApi;

#[tokio::test]
#[ignore = "manual paper inject; set E2E_DATABASE_OK=1 or E2E_PRIMARY_OK=1"]
async fn inject_trade() {
    let e2e = env::var("E2E_DATABASE_OK").ok().as_deref() == Some("1")
        || env::var("E2E_MODE").ok().as_deref() == Some("1");
    let primary = env::var("E2E_PRIMARY_OK").ok().as_deref() == Some("1");
    if !e2e && !primary {
        return;
    }
    let worker_url = env::var("WORKER_DATABASE_URL").expect("WORKER_DATABASE_URL");
    if e2e {
        test_support::assert_e2e_database_url(&worker_url);
    } else {
        test_support::assert_primary_database_url(&worker_url);
    }

    let wallet = env::var("INJECT_WALLET").expect("INJECT_WALLET");
    let side = env::var("INJECT_SIDE").unwrap_or_else(|_| "buy".into());
    let suffix = env::var("INJECT_TX_SUFFIX").expect("INJECT_TX_SUFFIX");
    let tx = format!("0xe2e{suffix}");

    let pool = pommarket::db::connect(&worker_url).await.expect("connect");
    let vault = pommarket::security::Vault::new(
        &env::var("ENCRYPTION_MASTER_KEY").expect("ENCRYPTION_MASTER_KEY"),
        &env::var("ENCRYPTION_KEY_ID").expect("ENCRYPTION_KEY_ID"),
        None,
        None,
    )
    .expect("vault");
    let books = common::FixedBook;
    let pipeline = pommarket::pipeline::PipelineConfig {
        clob_host: env::var("POLYMARKET_CLOB_HOST")
            .unwrap_or_else(|_| "https://clob.polymarket.com".into()),
        confirm_live: false,
        default_fee_rate: rust_decimal::Decimal::ZERO,
        default_fee_exponent: 1,
    };

    let trade = TestApi::sample_trade(&wallet, &side, "10", &tx);
    let outcome = pommarket::pipeline::ingest(&pool, &trade)
        .await
        .expect("ingest");
    if outcome.created || !outcome.duplicate_fragment {
        pommarket::pipeline::process_event(&pool, &vault, &books, &pipeline, outcome.event_id)
            .await
            .expect("process");
    }
    eprintln!(
        "ingest created={} event_id={}",
        outcome.created, outcome.event_id
    );
}
