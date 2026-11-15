//! Strategy activity feed pagination and filters (pommarket_test).

mod test_support;

use pommarket::db::{connect, migrate};
use pommarket::strategy_analytics::{list_activity, ActivityListParams};
use test_support::assert_test_database_url;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn activity_pagination_and_type_filter() {
    assert_test_database_url(&std::env::var("MIGRATION_DATABASE_URL").unwrap());
    let url = std::env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ($1,'Act','x','user','active',now()) RETURNING id",
    )
    .bind(format!("act-{}@example.com", Uuid::new_v4()))
    .fetch_one(&pool)
    .await
    .unwrap();
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signature_type, signer_address, funder_address, operating_mode, status)
         VALUES ($1,'A','eoa','0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc','0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc','paper_trading','connected')
         RETURNING id",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();
    let trader: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias) VALUES ($1,$2,'L') RETURNING id",
    )
    .bind(user)
    .bind("0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
    .fetch_one(&pool)
    .await
    .unwrap();
    let strategy: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (user_id, trading_account_id, source_trader_id, enabled, sizing_mode, proportion, strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions, max_slippage_bps, min_order_usd, copy_buy, copy_sell)
         VALUES ($1,$2,$3,false,'proportional',0.1,100,50,500,5,150,1,true,true) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(trader)
    .fetch_one(&pool)
    .await
    .unwrap();
    for i in 0..3 {
        let ev: Uuid = sqlx::query_scalar(
            "INSERT INTO source_events (dedup_key, source_wallet, token_id, side, price, size_shares, notional_usd, event_ts, market_title, outcome, status, origin, raw)
             VALUES ($1,'0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee','tok-a','buy',0.5,10,5,now(),'Alpha Market','Yes','processed','test','{}'::jsonb) RETURNING id",
        )
        .bind(format!("act-u-{i}"))
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO copy_decisions (user_id, trading_account_id, strategy_id, source_trader_id, source_event_id, side, status, reason, operating_mode, strategy_snapshot, checks, decided_at, detected_at)
             VALUES ($1,$2,$3,$4,$5,'buy','skipped','below_minimum','paper_trading','{}','{}', now() - ($6::text || ' hours')::interval, now())",
        )
        .bind(user)
        .bind(account)
        .bind(strategy)
        .bind(trader)
        .bind(ev)
        .bind(i.to_string())
        .execute(&pool)
        .await
        .unwrap();
    }
    let (page1, total) = list_activity(
        &pool,
        user,
        strategy,
        ActivityListParams {
            activity_type: None,
            side: None,
            status: None,
            search: Some("alpha"),
            from: None,
            to: None,
            limit: 2,
            offset: 0,
        },
    )
    .await
    .unwrap();
    assert_eq!(total, 3);
    assert_eq!(page1.len(), 2);
    let (page2, _) = list_activity(
        &pool,
        user,
        strategy,
        ActivityListParams {
            activity_type: None,
            side: None,
            status: None,
            search: Some("alpha"),
            from: None,
            to: None,
            limit: 2,
            offset: 2,
        },
    )
    .await
    .unwrap();
    assert_eq!(page2.len(), 1);
    let ids1: Vec<Uuid> = page1.iter().map(|r| r.decision_id).collect();
    let ids2: Vec<Uuid> = page2.iter().map(|r| r.decision_id).collect();
    assert!(ids1.iter().all(|id| !ids2.contains(id)));

    let (skipped_only, t2) = list_activity(
        &pool,
        user,
        strategy,
        ActivityListParams {
            activity_type: None,
            side: None,
            status: Some("skipped"),
            search: None,
            from: None,
            to: None,
            limit: 25,
            offset: 0,
        },
    )
    .await
    .unwrap();
    assert_eq!(t2, 3);
    assert!(skipped_only.iter().all(|r| r.execution_status == "skipped"));
}
