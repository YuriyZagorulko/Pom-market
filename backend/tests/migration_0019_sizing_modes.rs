//! Validates migration 0019 multiplier → proportional on pommarket_test only.

mod test_support;

use pommarket::copy_sizing::{raw_copy_notional, StrategyModeParams};
use pommarket::db::{connect, migrate};
use rust_decimal::Decimal;
use test_support::assert_test_database_url;
use uuid::Uuid;

const MIGRATION_0019_UPDATE: &str = r"
UPDATE copy_strategies
SET sizing_mode = 'proportional',
    proportion = COALESCE(proportion, multiplier),
    multiplier = NULL
WHERE sizing_mode = 'multiplier'
  AND COALESCE(proportion, multiplier) IS NOT NULL;
";

async fn seed_strategy(pool: &sqlx::PgPool, sizing_mode: &str) -> Uuid {
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ($1,'M','x','user','active',now()) RETURNING id",
    )
    .bind(format!("m19-{}@example.com", Uuid::new_v4()))
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
    sqlx::query(
        "INSERT INTO account_risk_limits (trading_account_id, user_id, max_order_usd, max_daily_loss_usd, max_exposure_usd, max_slippage_bps, max_orders_per_interval, interval_seconds, max_stale_data_seconds, paper_cash_usd)
         VALUES ($1,$2,500,1000,5000,150,20,60,120,1000)",
    )
    .bind(account)
    .bind(user)
    .execute(pool)
    .await
    .unwrap();
    let trader: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias, monitoring_enabled) VALUES ($1,$2,'L',true) RETURNING id",
    )
    .bind(user)
    .bind("0xdddddddddddddddddddddddddddddddddddddddd")
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO copy_strategies (user_id, trading_account_id, source_trader_id, enabled, sizing_mode, proportion, multiplier, strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions, max_slippage_bps, min_order_usd, copy_buy, copy_sell)
         VALUES ($1,$2,$3,false,$4,$5,$6,500,200,500,10,150,1,true,true) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(trader)
    .bind(sizing_mode)
    .bind(None::<Decimal>)
    .bind(None::<Decimal>)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_with_sizing(
    pool: &sqlx::PgPool,
    sizing_mode: &str,
    proportion: Option<Decimal>,
    multiplier: Option<Decimal>,
) -> Uuid {
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, role, status, email_verified_at)
         VALUES ($1,'M','x','user','active',now()) RETURNING id",
    )
    .bind(format!("m19b-{}@example.com", Uuid::new_v4()))
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
    sqlx::query(
        "INSERT INTO account_risk_limits (trading_account_id, user_id, max_order_usd, max_daily_loss_usd, max_exposure_usd, max_slippage_bps, max_orders_per_interval, interval_seconds, max_stale_data_seconds, paper_cash_usd)
         VALUES ($1,$2,500,1000,5000,150,20,60,120,1000)",
    )
    .bind(account)
    .bind(user)
    .execute(pool)
    .await
    .unwrap();
    let trader: Uuid = sqlx::query_scalar(
        "INSERT INTO source_traders (user_id, wallet_address, alias, monitoring_enabled) VALUES ($1,$2,'L',true) RETURNING id",
    )
    .bind(user)
    .bind("0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO copy_strategies (user_id, trading_account_id, source_trader_id, enabled, sizing_mode, proportion, multiplier, strategy_budget_usd, max_position_usd, max_exposure_usd, max_open_positions, max_slippage_bps, min_order_usd, copy_buy, copy_sell)
         VALUES ($1,$2,$3,false,$4,$5,$6,500,200,500,10,150,1,true,true) RETURNING id",
    )
    .bind(user)
    .bind(account)
    .bind(trader)
    .bind(sizing_mode)
    .bind(proportion)
    .bind(multiplier)
    .fetch_one(pool)
    .await
    .unwrap()
}

type Row = (String, Option<Decimal>, Option<Decimal>);

async fn fetch_row(pool: &sqlx::PgPool, id: Uuid) -> Row {
    sqlx::query_as(
        "SELECT sizing_mode, proportion, multiplier FROM copy_strategies WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL pointed at pommarket_test"]
async fn migration_0019_multiplier_and_preserves_other_modes() {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&url);
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();

    let mult_only = insert_with_sizing(&pool, "multiplier", None, Some(Decimal::new(15, 1))).await;
    let mult_with_prop =
        insert_with_sizing(&pool, "multiplier", Some(Decimal::new(1, 1)), Some(Decimal::new(15, 1)))
            .await;
    let prop = insert_with_sizing(
        &pool,
        "proportional",
        Some(Decimal::new(2, 1)),
        None,
    )
    .await;
    let fixed = seed_strategy(&pool, "fixed_amount").await;
    let pvr = seed_strategy(&pool, "position_value_ratio").await;
    let portfolio = seed_strategy(&pool, "portfolio_ratio").await;

    sqlx::query(MIGRATION_0019_UPDATE)
        .execute(&pool)
        .await
        .unwrap();

    let (mode, prop_v, mult_v) = fetch_row(&pool, mult_only).await;
    assert_eq!(mode, "proportional");
    assert_eq!(prop_v, Some(Decimal::new(15, 1)));
    assert!(mult_v.is_none());

    let (mode, prop_v, mult_v) = fetch_row(&pool, mult_with_prop).await;
    assert_eq!(mode, "proportional");
    assert_eq!(prop_v, Some(Decimal::new(1, 1)));
    assert!(mult_v.is_none());

    let (mode, prop_v, _) = fetch_row(&pool, prop).await;
    assert_eq!(mode, "proportional");
    assert_eq!(prop_v, Some(Decimal::new(2, 1)));

    let (mode, _, _) = fetch_row(&pool, fixed).await;
    assert_eq!(mode, "fixed_amount");
    let (mode, _, _) = fetch_row(&pool, pvr).await;
    assert_eq!(mode, "position_value_ratio");
    let (mode, _, _) = fetch_row(&pool, portfolio).await;
    assert_eq!(mode, "portfolio_ratio");

    let params = StrategyModeParams {
        sizing_mode: "proportional".into(),
        fixed_amount_usd: None,
        proportion: Some(Decimal::new(15, 1)),
        multiplier: None,
        strategy_allocated_capital_usd: Decimal::from(500),
        source_trader_equity_usd: None,
        source_equity_status: None,
        source_open_positions_value_usd: None,
        copy_small_buys_1to1: false,
        small_buy_threshold_usd: None,
    };
    assert_eq!(
        raw_copy_notional(&params, Decimal::from(100)).unwrap(),
        Decimal::from(150)
    );

    let defs: Vec<String> = sqlx::query_scalar(
        "SELECT pg_get_constraintdef(con.oid)
         FROM pg_constraint con
         JOIN pg_class rel ON rel.oid = con.conrelid
         WHERE rel.relname = 'copy_strategies'
           AND con.contype = 'c'
           AND pg_get_constraintdef(con.oid) LIKE '%sizing_mode%'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        defs.iter()
            .any(|d| d.contains("position_value_ratio") && d.contains("portfolio_ratio")),
        "sizing_mode check should still allow legacy portfolio_ratio: {defs:?}"
    );
}
