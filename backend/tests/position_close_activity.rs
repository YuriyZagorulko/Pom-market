//! Activity feed includes position close snapshots and closed filter.

mod test_support;

use pommarket::db::{connect, migrate};
use pommarket::strategy_analytics::{list_activity, ActivityListParams};
use rust_decimal::Decimal;
use test_support::assert_test_database_url;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn closed_filter_lists_close_events_once() {
    assert_test_database_url(&std::env::var("MIGRATION_DATABASE_URL").unwrap());
    let url = std::env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ($1,'CloseAct','x','user','active',now()) RETURNING id",
    )
    .bind(format!("close-act-{}@example.com", Uuid::new_v4()))
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
    .bind("0xcccccccccccccccccccccccccccccccccccccccc")
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
    let position: Uuid = sqlx::query_scalar(
        "INSERT INTO positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, market_title, outcome, operating_mode, open_shares, avg_price, cost_basis_usd)
         VALUES ($1,$2,$3,'0xcccccccccccccccccccccccccccccccccccccccc','tok-close','Close Market','Yes','paper_trading',0,0.5,20) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(strategy)
    .fetch_one(&pool)
    .await
    .unwrap();
    let idem = format!("test-close-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO position_close_events (
            user_id, trading_account_id, strategy_id, position_id, token_id,
            market_title, outcome, close_reason, closed_shares, avg_entry_price,
            avg_exit_price, total_cost_basis_usd, total_proceeds_usd, total_fees_usd,
            realized_pnl_usd, idempotency_key, closed_at
         ) VALUES ($1,$2,$3,$4,'tok-close','Close Market','Yes','leader_sell',40,0.5,0.55,20,22,0.1,2,$5,now())",
    )
    .bind(user)
    .bind(account)
    .bind(strategy)
    .bind(position)
    .bind(&idem)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO position_close_events (
            user_id, trading_account_id, strategy_id, position_id, token_id,
            market_title, outcome, close_reason, closed_shares, avg_entry_price,
            avg_exit_price, total_cost_basis_usd, total_proceeds_usd, total_fees_usd,
            realized_pnl_usd, idempotency_key, closed_at
         ) VALUES ($1,$2,$3,$4,'tok-close','Close Market','Yes','leader_sell',40,0.5,0.55,20,22,0.1,2,$5,now())",
    )
    .bind(user)
    .bind(account)
    .bind(strategy)
    .bind(position)
    .bind(&idem)
    .execute(&pool)
    .await
    .err()
    .expect("duplicate idempotency_key must fail");

    let (all, total_all) = list_activity(
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
            limit: 50,
            offset: 0,
        },
    )
    .await
    .unwrap();
    assert!(total_all >= 1);
    assert!(all.iter().any(|r| r.activity_kind == "closed"));

    let (closed_only, total_closed) = list_activity(
        &pool,
        user,
        strategy,
        ActivityListParams {
            activity_type: Some("closed"),
            side: None,
            status: None,
            search: None,
            from: None,
            to: None,
            limit: 50,
            offset: 0,
        },
    )
    .await
    .unwrap();
    assert_eq!(total_closed, 1);
    assert_eq!(closed_only.len(), 1);
    assert_eq!(closed_only[0].activity_kind, "closed");
    assert_eq!(
        closed_only[0].close_reason.as_deref(),
        Some("leader_sell")
    );
    assert_eq!(
        closed_only[0].close_realized_pnl_usd,
        Some(Decimal::from(2))
    );
}
