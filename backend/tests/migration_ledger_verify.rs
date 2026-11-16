mod test_support;

use pommarket::db::{connect, migrate, verify_migration_ledger};
use test_support::assert_test_database_url;

#[tokio::test]
#[ignore = "requires PostgreSQL pommarket_test"]
async fn verify_migration_ledger_passes_after_migrate() {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    assert_test_database_url(&url);
    migrate(&url).await.expect("migrate");
    let pool = connect(&url).await.expect("connect");
    verify_migration_ledger(&pool)
        .await
        .expect("ledger should match embedded migrations");
}