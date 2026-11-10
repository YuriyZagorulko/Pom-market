use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::begin_as;

#[derive(Debug, Clone, serde::Serialize)]
pub struct BudgetUtilization {
    pub allocated_budget_usd: Decimal,
    pub deployed_capital_usd: Decimal,
    pub reserved_capital_usd: Decimal,
    pub available_budget_usd: Decimal,
    pub open_positions_count: i64,
    pub updated_at: DateTime<Utc>,
}

const OPEN_ORDER_STATUSES: &[&str] = &[
    "pending",
    "submitting",
    "submitted",
    "partially_filled",
    "unknown",
];

/// Deployed = cost basis in open positions. Reserved = open order reserves (not double-counted).
pub async fn strategy_utilization(
    pool: &PgPool,
    strategy_id: Uuid,
) -> anyhow::Result<BudgetUtilization> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let row = utilization_in_tx(&mut tx, strategy_id).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn utilization_for_user(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
) -> anyhow::Result<BudgetUtilization> {
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let owns: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM copy_strategies WHERE id = $1 AND user_id = $2)",
    )
    .bind(strategy_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !owns {
        anyhow::bail!("strategy not found");
    }
    let row = utilization_in_tx(&mut tx, strategy_id).await?;
    tx.commit().await?;
    Ok(row)
}

async fn utilization_in_tx(
    tx: &mut crate::db::Tx<'_>,
    strategy_id: Uuid,
) -> anyhow::Result<BudgetUtilization> {
    let allocated: Decimal =
        sqlx::query_scalar("SELECT strategy_budget_usd FROM copy_strategies WHERE id = $1")
            .bind(strategy_id)
            .fetch_one(&mut **tx)
            .await?;

    let deployed: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(cost_basis_usd), 0) FROM positions
         WHERE strategy_id = $1 AND open_shares > 0",
    )
    .bind(strategy_id)
    .fetch_one(&mut **tx)
    .await?;

    let reserved: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(o.position_reserved_usd + o.fee_reserved_usd), 0)
         FROM orders o
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1
           AND o.status = ANY($2::text[])",
    )
    .bind(strategy_id)
    .bind(
        OPEN_ORDER_STATUSES
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<String>>(),
    )
    .fetch_one(&mut **tx)
    .await?;

    let open_positions_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM positions WHERE strategy_id = $1 AND open_shares > 0",
    )
    .bind(strategy_id)
    .fetch_one(&mut **tx)
    .await?;

    let updated_at: DateTime<Utc> = sqlx::query_scalar(
        "SELECT GREATEST(
            COALESCE((SELECT MAX(updated_at) FROM positions WHERE strategy_id = $1), '-infinity'::timestamptz),
            COALESCE((SELECT MAX(o.updated_at) FROM orders o JOIN copy_decisions d ON d.id = o.decision_id WHERE d.strategy_id = $1), '-infinity'::timestamptz),
            (SELECT updated_at FROM copy_strategies WHERE id = $1)
         )",
    )
    .bind(strategy_id)
    .fetch_one(&mut **tx)
    .await?;

    let available = (allocated - deployed - reserved).max(Decimal::ZERO);

    Ok(BudgetUtilization {
        allocated_budget_usd: allocated,
        deployed_capital_usd: deployed,
        reserved_capital_usd: reserved,
        available_budget_usd: available,
        open_positions_count,
        updated_at,
    })
}

/// Capital tied up for strategy budget checks (deployed + reserved).
pub async fn strategy_exposure_usd(
    tx: &mut crate::db::Tx<'_>,
    strategy_id: Uuid,
) -> anyhow::Result<Decimal> {
    let util = utilization_in_tx(tx, strategy_id).await?;
    Ok(util.deployed_capital_usd + util.reserved_capital_usd)
}
