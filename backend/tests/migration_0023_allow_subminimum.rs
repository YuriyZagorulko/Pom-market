//! Validates migration 0023 default for allow_subminimum_paper_trades.

mod test_support;

use pommarket::db::{connect, migrate};
use test_support::assert_test_database_url;

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn migration_0023_defaults_allow_subminimum_false() {
    assert_test_database_url(&std::env::var("MIGRATION_DATABASE_URL").unwrap());
    let url = std::env::var("MIGRATION_DATABASE_URL").unwrap();
    migrate(&url).await.unwrap();
    let pool = connect(&url).await.unwrap();
    let is_bool_not_null: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM information_schema.columns
            WHERE table_name = 'copy_strategies'
              AND column_name = 'allow_subminimum_paper_trades'
              AND udt_name = 'bool'
              AND is_nullable = 'NO'
        )",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(is_bool_not_null);
    let default: Option<String> = sqlx::query_scalar(
        "SELECT column_default::text FROM information_schema.columns
         WHERE table_name = 'copy_strategies' AND column_name = 'allow_subminimum_paper_trades'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        default.as_deref().unwrap_or("").contains("false"),
        "expected default false, got {:?}",
        default
    );
}
