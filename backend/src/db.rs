use sqlx::{migrate::Migrator, PgPool, Postgres, Transaction};
use uuid::Uuid;

pub type Tx<'a> = Transaction<'a, Postgres>;

/// Single canonical SQLx migration chain embedded at compile time (backend/migrations).
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

pub async fn connect(url: &str) -> anyhow::Result<PgPool> {
    Ok(PgPool::connect(url).await?)
}

pub async fn migrate(url: &str) -> anyhow::Result<()> {
    let pool = connect(url).await?;
    MIGRATOR.run(&pool).await?;
    Ok(())
}

/// Every migration recorded in `_sqlx_migrations` must exist in this binary with the same checksum.
/// Call before event processing when `POM_SKIP_STARTUP_MIGRATION` skips `migrate()`.
pub async fn verify_migration_ledger(pool: &PgPool) -> anyhow::Result<()> {
    let applied: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version, checksum FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(pool)
    .await?;
    for (version, checksum) in applied {
        let embedded = MIGRATOR
            .iter()
            .find(|m| m.version == version)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "migration {version} was previously applied but is missing from the resolved migrations"
                )
            })?;
        if embedded.checksum != checksum {
            anyhow::bail!(
                "migration {version} checksum mismatch between database and application binary; rebuild api/worker from current source"
            );
        }
    }
    Ok(())
}

/// Highest embedded migration version (for startup logs / diagnostics).
pub fn embedded_migration_head_version() -> i64 {
    MIGRATOR
        .iter()
        .map(|m| m.version)
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod migration_source_tests {
    use super::*;

    #[test]
    fn embedded_migrations_are_sequential_and_include_head() {
        let mut versions: Vec<i64> = MIGRATOR.iter().map(|m| m.version).collect();
        versions.sort_unstable();
        assert!(
            versions.last().copied().unwrap_or(0) >= 38,
            "expected migration head >= 38, got {:?}",
            versions.last()
        );
        for w in versions.windows(2) {
            assert_eq!(w[1] - w[0], 1, "non-sequential migration versions: {w:?}");
        }
    }
}

pub async fn begin_as<'a>(
    pool: &'a PgPool,
    user_id: Option<Uuid>,
    role: &str,
) -> anyhow::Result<Tx<'a>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true), set_config('app.role', $2, true)")
        .bind(user_id.map(|id| id.to_string()).unwrap_or_default())
        .bind(role)
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}
