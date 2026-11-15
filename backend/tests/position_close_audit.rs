//! Audit tests: close idempotency, historical partials, lifecycle P&L, activity feed.

mod test_support;

use pommarket::db::{connect, migrate};
use pommarket::position_close::{
    idempotency_key, record_close, record_close_if_full_exit, CloseReason, CloseSnapshot,
    CloseSnapshotMeta, RecordCloseResult,
};
use pommarket::strategy_analytics::{list_activity, ActivityListParams};
use rust_decimal::Decimal;
use std::str::FromStr;
use test_support::assert_test_database_url;
use uuid::Uuid;

async fn seed_strategy(pool: &sqlx::PgPool) -> (Uuid, Uuid, Uuid, Uuid) {
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ($1,'Aud','x','user','active',now()) RETURNING id",
    )
    .bind(format!("close-audit-{}@example.com", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .unwrap();
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_accounts (user_id, name, signature_type, signer_address, funder_address, operating_mode, status)
         VALUES ($1,'A','eoa','0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc','0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc','paper_trading','connected')
         RETURNING id",
    )
    .bind(user)
    .fetch_one(pool)
    .await
    .unwrap();
    let trader: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias) VALUES ($1,$2,'L') RETURNING id",
    )
    .bind(user)
    .bind("0xdddddddddddddddddddddddddddddddddddddddd")
    .fetch_one(pool)
    .await
    .unwrap();
    let strategy: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_strategies (user_id, trading_account_id, source_trader_id, enabled, sizing_mode, proportion, strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions, max_slippage_bps, min_order_usd, copy_buy, copy_sell)
         VALUES ($1,$2,$3,false,'proportional',0.1,100,50,500,5,150,1,true,true) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(trader)
    .fetch_one(pool)
    .await
    .unwrap();
    (user, account, trader, strategy)
}

async fn seed_position(
    pool: &sqlx::PgPool,
    user: Uuid,
    account: Uuid,
    strategy: Uuid,
    close_cycle: i64,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, market_title, outcome, operating_mode, open_shares, avg_price, cost_basis_usd, close_cycle)
         VALUES ($1,$2,$3,'0xdddddddddddddddddddddddddddddddddddddddd','tok-audit','Audit M','Yes','paper_trading',0,0.5,0,$4) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(strategy)
    .bind(close_cycle)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn dual_close_triggers_one_closed_row_per_cycle() {
    assert_test_database_url(&std::env::var("MIGRATION_DATABASE_URL").unwrap());
    let url = std::env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let (user, account, _, strategy) = seed_strategy(&pool).await;
    let position_id = seed_position(&pool, user, account, strategy, 0).await;
    sqlx::query("UPDATE positions SET open_shares = 0 WHERE id = $1")
        .bind(position_id)
        .execute(&pool)
        .await
        .unwrap();

    let mut tx = pommarket::db::begin_as(&pool, None, "worker").await.unwrap();
    let meta = CloseSnapshotMeta {
        user_id: user,
        trading_account_id: account,
        strategy_id: strategy,
        position_id,
        token_id: "tok-audit".into(),
        operating_mode: "paper_trading".into(),
        market_title: Some("Audit M".into()),
        outcome: Some("Yes".into()),
        reason: CloseReason::LeaderSell,
        close_cycle: 0,
        realized_pnl_usd: Decimal::from(3),
        fallback_closed_shares: Decimal::from(10),
        fallback_cost_basis: Decimal::from(5),
        fallback_avg_entry: Decimal::new(5, 1),
    };
  let r1 = record_close_if_full_exit(&mut tx, meta.clone(), None).await.unwrap();
    assert_eq!(r1, RecordCloseResult::Inserted);
    let mut meta2 = meta;
    meta2.reason = CloseReason::HardClose;
    let r2 = record_close_if_full_exit(&mut tx, meta2, None).await.unwrap();
    assert_eq!(r2, RecordCloseResult::AlreadyRecorded);
    tx.commit().await.unwrap();

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM position_close_events WHERE position_id = $1",
    )
    .bind(position_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        idempotency_key(position_id, 0),
        format!("close|pos:{position_id}|cycle:0")
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn historical_partial_sell_remaining_not_current_position() {
    assert_test_database_url(&std::env::var("MIGRATION_DATABASE_URL").unwrap());
    let url = std::env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let (user, account, trader, strategy) = seed_strategy(&pool).await;

    for (i, (side, shares_after)) in [("sell", "40"), ("sell", "0")].iter().enumerate() {
        let ev: Uuid = sqlx::query_scalar(
            "INSERT INTO source_events (dedup_key, source_wallet, token_id, side, price, size_shares, notional_usd, event_ts, market_title, outcome, status, origin, raw)
             VALUES ($1,'0xdddddddddddddddddddddddddddddddddddddddd','tok-audit','sell',0.5,60,30,now() - ($2::text || ' minutes')::interval,'Audit M','Yes','processed','test','{}'::jsonb) RETURNING id",
        )
        .bind(format!("audit-partial-{i}-{}", Uuid::new_v4()))
        .bind((10 - i).to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
        let decision: Uuid = sqlx::query_scalar(
            "INSERT INTO copy_decisions (user_id, trading_account_id, strategy_id, source_trader_id, source_event_id, side, status, reason, operating_mode, strategy_snapshot, checks, decided_at, detected_at)
             VALUES ($1,$2,$3,$4,$5,'sell','approved','ok','paper_trading','{}','{}', now() - ($6::text || ' minutes')::interval, now()) RETURNING id",
        )
        .bind(user)
        .bind(account)
        .bind(strategy)
        .bind(trader)
        .bind(ev)
        .bind((10 - i).to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
        let order: Uuid = sqlx::query_scalar(
            "INSERT INTO orders (user_id, trading_account_id, decision_id, source_event_id, client_order_id, operating_mode, order_type, side, token_id, limit_price, size_shares, status, position_open_shares_after)
             VALUES ($1,$2,$3,$4,gen_random_uuid(),'paper_trading','FAK','sell','tok-audit',0.5,60,'filled',$5) RETURNING id",
        )
        .bind(user)
        .bind(account)
        .bind(decision)
        .bind(ev)
        .bind(Decimal::from_str(shares_after).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO fills (user_id, trading_account_id, order_id, price, shares, notional_usd, fee_status, filled_at)
             VALUES ($1,$2,$3,0.5,60,30,'known_zero', now() - ($4::text || ' minutes')::interval)",
        )
        .bind(user)
        .bind(account)
        .bind(order)
        .bind((10 - i).to_string())
        .execute(&pool)
        .await
        .unwrap();
    }

    let (rows, _) = list_activity(
        &pool,
        user,
        strategy,
        ActivityListParams {
            activity_type: Some("sell"),
            side: None,
            status: None,
            search: None,
            from: None,
            to: None,
            limit: 10,
            offset: 0,
        },
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    let mut remainders: Vec<Option<Decimal>> = rows
        .iter()
        .map(|r| r.position_open_shares)
        .collect();
    remainders.sort_by_key(|v| v.map(|d| d.to_string()));
    assert_eq!(remainders[0], Some(Decimal::ZERO));
    assert_eq!(remainders[1], Some(Decimal::from(40)));
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn reopen_position_allows_new_closed_on_next_cycle() {
    assert_test_database_url(&std::env::var("MIGRATION_DATABASE_URL").unwrap());
    let url = std::env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let (user, account, _, strategy) = seed_strategy(&pool).await;
    let position_id = seed_position(&pool, user, account, strategy, 1).await;
    sqlx::query("UPDATE positions SET open_shares = 0 WHERE id = $1")
        .bind(position_id)
        .execute(&pool)
        .await
        .unwrap();

    let mut tx = pommarket::db::begin_as(&pool, None, "worker").await.unwrap();
    let base = CloseSnapshotMeta {
        user_id: user,
        trading_account_id: account,
        strategy_id: strategy,
        position_id,
        token_id: "tok-audit".into(),
        operating_mode: "paper_trading".into(),
        market_title: None,
        outcome: None,
        reason: CloseReason::LeaderSell,
        close_cycle: 1,
        realized_pnl_usd: Decimal::ONE,
        fallback_closed_shares: Decimal::from(5),
        fallback_cost_basis: Decimal::from(2),
        fallback_avg_entry: Decimal::new(4, 1),
    };
    record_close_if_full_exit(&mut tx, base, None).await.unwrap();
    tx.commit().await.unwrap();

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM position_close_events WHERE position_id = $1",
    )
    .bind(position_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn activity_capital_filter_and_pagination_with_closed() {
    assert_test_database_url(&std::env::var("MIGRATION_DATABASE_URL").unwrap());
    let url = std::env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let (user, account, _, strategy) = seed_strategy(&pool).await;
    let position_id = seed_position(&pool, user, account, strategy, 0).await;

    sqlx::query(
        "INSERT INTO capital_ledger_entries (user_id, trading_account_id, strategy_id, entry_type, amount_usd, idempotency_key)
         VALUES ($1,$2,$3,'strategy_allocation',500,$4)",
    )
    .bind(user)
    .bind(account)
    .bind(strategy)
    .bind(format!("audit-alloc-{}", Uuid::new_v4()))
    .execute(&pool)
    .await
    .unwrap();

    let mut tx = pommarket::db::begin_as(&pool, None, "worker").await.unwrap();
    let snap = CloseSnapshot {
        user_id: user,
        trading_account_id: account,
        strategy_id: strategy,
        position_id,
        token_id: "tok-audit".into(),
        market_title: Some("Audit M".into()),
        outcome: Some("Yes".into()),
        reason: CloseReason::ResolutionWin,
        close_cycle: 0,
        closed_shares: Decimal::from(20),
        avg_entry_price: Decimal::new(5, 1),
        avg_exit_price: None,
        total_cost_basis_usd: Decimal::from(10),
        total_proceeds_usd: Decimal::from(20),
        total_fees_usd: Decimal::ZERO,
        realized_pnl_usd: Decimal::from(10),
    };
    record_close(&mut tx, &snap).await.unwrap();
    tx.commit().await.unwrap();

    let (capital, t_cap) = list_activity(
        &pool,
        user,
        strategy,
        ActivityListParams {
            activity_type: Some("capital"),
            side: None,
            status: None,
            search: None,
            from: None,
            to: None,
            limit: 20,
            offset: 0,
        },
    )
    .await
    .unwrap();
    assert_eq!(t_cap, 1);
    assert_eq!(capital[0].activity_kind, "capital");

    let (all, total) = list_activity(
        &pool,
        user,
        strategy,
        ActivityListParams {
            activity_type: None,
            side: None,
            status: None,
            search: None,
            from: None,
            to: None,
            limit: 1,
            offset: 0,
        },
    )
    .await
    .unwrap();
    assert!(total >= 2);
    assert_eq!(all.len(), 1);
    let (page2, _) = list_activity(
        &pool,
        user,
        strategy,
        ActivityListParams {
            activity_type: None,
            side: None,
            status: None,
            search: None,
            from: None,
            to: None,
            limit: 1,
            offset: 1,
        },
    )
    .await
    .unwrap();
    assert_eq!(page2.len(), 1);
    assert_ne!(all[0].decision_id, page2[0].decision_id);
}

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn repeat_idempotent_record_close_same_cycle() {
    assert_test_database_url(&std::env::var("MIGRATION_DATABASE_URL").unwrap());
    let url = std::env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let (user, account, _, strategy) = seed_strategy(&pool).await;
    let position_id = seed_position(&pool, user, account, strategy, 0).await;
    let snap = CloseSnapshot {
        user_id: user,
        trading_account_id: account,
        strategy_id: strategy,
        position_id,
        token_id: "tok-audit".into(),
        market_title: None,
        outcome: None,
        reason: CloseReason::LeaderSell,
        close_cycle: 0,
        closed_shares: Decimal::ONE,
        avg_entry_price: Decimal::ONE,
        avg_exit_price: None,
        total_cost_basis_usd: Decimal::ONE,
        total_proceeds_usd: Decimal::ONE,
        total_fees_usd: Decimal::ZERO,
        realized_pnl_usd: Decimal::ZERO,
    };
    let mut tx = pommarket::db::begin_as(&pool, None, "worker").await.unwrap();
    assert_eq!(record_close(&mut tx, &snap).await.unwrap(), RecordCloseResult::Inserted);
    assert_eq!(
        record_close(&mut tx, &snap).await.unwrap(),
        RecordCloseResult::AlreadyRecorded
    );
    tx.commit().await.unwrap();
}
