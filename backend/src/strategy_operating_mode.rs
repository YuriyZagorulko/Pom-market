//! Per-strategy paper vs live operating mode changes.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::json;
use uuid::Uuid;

use crate::config::Config;
use crate::db::Tx;

/// Product gate: live copy execution is not production-ready.
pub const LIVE_COPY_PRODUCT_READY: bool = false;

pub const LIVE_TRADING_NOT_READY_CODE: &str = "live_trading_not_ready";
/// Returned when live trading is requested but the trading account has no stored signing key.
pub const ACCOUNT_CREDENTIALS_REQUIRED_CODE: &str = "account_credentials_required";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeChangeReject {
    NotFound,
    Enabled,
    ActiveOrders,
    OpenPositions,
    UnresolvedExecution,
    LiveNotReady,
    CredentialsMissing,
    InvalidMode,
    BadRequest(&'static str),
}

impl ModeChangeReject {
    pub fn api_message(&self) -> String {
        match self {
            Self::NotFound => "strategy not found".into(),
            Self::Enabled => "disable the strategy before changing trading mode".into(),
            Self::ActiveOrders => "resolve or cancel active orders before changing trading mode".into(),
            Self::OpenPositions => "close open positions before changing trading mode".into(),
            Self::UnresolvedExecution => {
                "wait for hard close or other execution operations to finish".into()
            }
            Self::LiveNotReady => LIVE_TRADING_NOT_READY_CODE.into(),
            Self::CredentialsMissing => ACCOUNT_CREDENTIALS_REQUIRED_CODE.into(),
            Self::InvalidMode => {
                "operating_mode must be paper_trading or live_trading".into()
            }
            Self::BadRequest(msg) => (*msg).into(),
        }
    }
}

fn valid_mode(mode: &str) -> bool {
    matches!(mode, "paper_trading" | "live_trading")
}

pub async fn change_operating_mode(
    tx: &mut Tx<'_>,
    user_id: Uuid,
    strategy_id: Uuid,
    next_mode: &str,
    confirm_live: bool,
    confirmation: &str,
    cfg: &Config,
) -> Result<String, ModeChangeReject> {
    if !valid_mode(next_mode) {
        return Err(ModeChangeReject::InvalidMode);
    }

    #[derive(sqlx::FromRow)]
    struct Row {
        enabled: bool,
        operating_mode: String,
        trading_account_id: Uuid,
    }

    let row: Option<Row> = sqlx::query_as(
        "SELECT enabled, operating_mode, trading_account_id
         FROM copy_strategies
         WHERE id = $1 AND user_id = $2
         FOR UPDATE",
    )
    .bind(strategy_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| ModeChangeReject::NotFound)?;

    let Some(row) = row else {
        return Err(ModeChangeReject::NotFound);
    };

    if row.enabled {
        return Err(ModeChangeReject::Enabled);
    }

    if unresolved_hard_close(tx, strategy_id).await? {
        return Err(ModeChangeReject::UnresolvedExecution);
    }

    let active_orders: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders o
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1
           AND o.status IN ('pending', 'submitting', 'submitted', 'partially_filled', 'unknown')",
    )
    .bind(strategy_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| ModeChangeReject::NotFound)?;

    if active_orders > 0 {
        return Err(ModeChangeReject::ActiveOrders);
    }

    let open_positions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM positions
         WHERE strategy_id = $1 AND operating_mode = $2 AND open_shares > 0",
    )
    .bind(strategy_id)
    .bind(&row.operating_mode)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| ModeChangeReject::NotFound)?;

    if open_positions > 0 {
        return Err(ModeChangeReject::OpenPositions);
    }

    if row.operating_mode == next_mode {
        return Ok(row.operating_mode);
    }

    if next_mode == "live_trading" {
        let has_credentials: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM trading_credentials WHERE trading_account_id = $1)",
        )
        .bind(row.trading_account_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| ModeChangeReject::NotFound)?;
        if !has_credentials {
            return Err(ModeChangeReject::CredentialsMissing);
        }
        if !LIVE_COPY_PRODUCT_READY {
            return Err(ModeChangeReject::LiveNotReady);
        }
        if !confirm_live || confirmation != "ENABLE LIVE TRADING" {
            return Err(ModeChangeReject::BadRequest(
                "type ENABLE LIVE TRADING and set confirm_live=true",
            ));
        }
        if !cfg.confirm_live_trading {
            return Err(ModeChangeReject::BadRequest(
                "server CONFIRM_LIVE_TRADING is not enabled",
            ));
        }
        let account_ok: Option<(String, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT status, live_confirmed_at FROM trading_accounts WHERE id = $1 AND user_id = $2",
        )
        .bind(row.trading_account_id)
        .bind(user_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| ModeChangeReject::NotFound)?;
        let Some((status, live_confirmed)) = account_ok else {
            return Err(ModeChangeReject::NotFound);
        };
        if status != "connected" || live_confirmed.is_none() {
            return Err(ModeChangeReject::BadRequest(
                "complete live account confirmation on the trading account before a live strategy",
            ));
        }
    }

    if row.operating_mode == "paper_trading" && next_mode == "live_trading" {
        let funded: Decimal = sqlx::query_scalar(
            "SELECT COALESCE(funded_capital_usd, 0) FROM strategy_capital_accounts WHERE strategy_id = $1",
        )
        .bind(strategy_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| ModeChangeReject::NotFound)?;
        if funded > Decimal::ZERO {
            return Err(ModeChangeReject::BadRequest(
                "withdraw or zero paper strategy capital before switching to live",
            ));
        }
    }

    sqlx::query(
        "UPDATE copy_strategies SET operating_mode = $3, updated_at = now() WHERE id = $1 AND user_id = $2",
    )
    .bind(strategy_id)
    .bind(user_id)
    .bind(next_mode)
    .execute(&mut **tx)
    .await
    .map_err(|_| ModeChangeReject::NotFound)?;

    sqlx::query(
        "INSERT INTO audit_log (actor_user_id, action, target, metadata) VALUES ($1,$2,$3,$4)",
    )
    .bind(user_id)
    .bind("strategy_operating_mode_changed")
    .bind(strategy_id.to_string())
    .bind(json!({
        "from": row.operating_mode,
        "to": next_mode,
    }))
    .execute(&mut **tx)
    .await
    .map_err(|_| ModeChangeReject::NotFound)?;

    Ok(next_mode.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_missing_uses_stable_code() {
        assert_eq!(
            ModeChangeReject::CredentialsMissing.api_message(),
            ACCOUNT_CREDENTIALS_REQUIRED_CODE
        );
    }
}

async fn unresolved_hard_close(
    tx: &mut Tx<'_>,
    strategy_id: Uuid,
) -> Result<bool, ModeChangeReject> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hard_close_operations
         WHERE strategy_id = $1 AND status IN ('queued', 'in_progress')",
    )
    .bind(strategy_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| ModeChangeReject::NotFound)?;
    Ok(n > 0)
}
