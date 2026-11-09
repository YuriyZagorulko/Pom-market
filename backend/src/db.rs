use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

pub type Tx<'a> = Transaction<'a, Postgres>;

pub async fn connect(url: &str) -> anyhow::Result<PgPool> {
    Ok(PgPool::connect(url).await?)
}

pub async fn migrate(url: &str) -> anyhow::Result<()> {
    let pool = connect(url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(())
}

pub async fn begin_as<'a>(pool: &'a PgPool, user_id: Option<Uuid>, role: &str) -> anyhow::Result<Tx<'a>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true), set_config('app.role', $2, true)")
        .bind(user_id.map(|id| id.to_string()).unwrap_or_default())
        .bind(role)
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}
