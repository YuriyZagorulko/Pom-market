//! Copy-strategy "can the pipeline trade?" checks for user-facing status.

use rust_decimal::Decimal;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;
use crate::db::begin_as;
use crate::source_equity;
use crate::strategy_accounting::is_capital_bootstrapped;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessCode {
    ReadyToTrade,
    StrategyDisabled,
    CapitalNotFunded,
    SourceEquityUnavailable,
    SourcePositionValueUnavailable,
    SourceMonitoringDisabled,
    TradingBlockedByControl,
    AccountNotPaperReady,
    MissingSizingConfig,
}

#[derive(Debug, Clone, Serialize)]
pub struct StrategyReadiness {
    pub code: ReadinessCode,
    pub label: &'static str,
    pub detail: String,
    pub ready: bool,
    pub enabled: bool,
    pub capital_funded_usd: Option<Decimal>,
    pub capital_bootstrap_quality: Option<String>,
    pub source_equity_status: Option<String>,
}

pub async fn readiness_for_user(
    pool: &PgPool,
    user_id: Uuid,
    strategy_id: Uuid,
    cfg: &Config,
) -> anyhow::Result<StrategyReadiness> {
    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    #[derive(sqlx::FromRow)]
    struct Row {
        enabled: bool,
        sizing_mode: String,
        proportion: Option<Decimal>,
        fixed_amount_usd: Option<Decimal>,
        multiplier: Option<Decimal>,
        source_trader_id: Uuid,
        monitoring_enabled: bool,
        operating_mode: String,
        account_status: String,
        emergency_stop: bool,
        trading_control_state: String,
    }
    let row = sqlx::query_as::<_, Row>(
        "SELECT s.enabled, s.sizing_mode, s.proportion, s.fixed_amount_usd, s.multiplier,
                s.source_trader_id, t.monitoring_enabled, a.operating_mode, a.status AS account_status,
                a.emergency_stop, a.trading_control_state
         FROM copy_strategies s
         JOIN source_traders t ON t.id = s.source_trader_id
         JOIN trading_accounts a ON a.id = s.trading_account_id
         WHERE s.id = $1 AND s.user_id = $2",
    )
    .bind(strategy_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        anyhow::bail!("strategy not found");
    };

    let funded: Option<(Decimal, String)> = sqlx::query_as(
        "SELECT funded_capital_usd, bootstrap_quality FROM strategy_capital_accounts WHERE strategy_id = $1",
    )
    .bind(strategy_id)
    .fetch_optional(&mut *tx)
    .await?;

    let kill: bool = sqlx::query_scalar(
        "SELECT COALESCE((value->>'enabled')::boolean, false) FROM system_settings WHERE key = 'global_kill_switch'",
    )
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    if !row.enabled {
        return Ok(blocked(
            ReadinessCode::StrategyDisabled,
            "Strategy disabled",
            "Enable the strategy from the strategy page header.",
            row.enabled,
            funded,
            None,
        ));
    }

    if row.account_status == "disconnected"
        || row.operating_mode == "monitoring"
        || row.emergency_stop
        || kill
        || row.trading_control_state != "normal"
    {
        let detail: String = if kill {
            "Global kill switch is on.".into()
        } else if row.emergency_stop {
            "Emergency stop is active on the trading account.".into()
        } else if row.trading_control_state != "normal" {
            format!(
                "Trading account control state: {}.",
                row.trading_control_state
            )
        } else if row.operating_mode != "paper_trading" {
            "Trading account must be in paper_trading mode.".into()
        } else {
            "Connect the trading account.".into()
        };
        return Ok(blocked(
            ReadinessCode::TradingBlockedByControl,
            "Trading blocked by control",
            detail,
            row.enabled,
            funded,
            None,
        ));
    }

    if row.operating_mode != "paper_trading" {
        return Ok(blocked(
            ReadinessCode::AccountNotPaperReady,
            "Account not in PAPER mode",
            "Set operating mode to Paper trading on the account page.",
            row.enabled,
            funded,
            None,
        ));
    }

    if !row.monitoring_enabled {
        return Ok(blocked(
            ReadinessCode::SourceMonitoringDisabled,
            "Source monitoring disabled",
            "On the source trader page, turn on monitoring.",
            row.enabled,
            funded,
            None,
        ));
    }

    let bootstrapped = {
        let mut tx = begin_as(pool, Some(user_id), "user").await?;
        let ok = is_capital_bootstrapped(&mut tx, strategy_id).await?;
        tx.commit().await?;
        ok
    };
    let funded_amt = funded.as_ref().map(|(f, _)| *f).unwrap_or(Decimal::ZERO);
    if !bootstrapped || funded_amt <= Decimal::ZERO {
        return Ok(blocked(
            ReadinessCode::CapitalNotFunded,
            "Capital not funded",
            "Open Settings, confirm Budget and allocation, click Save changes (funds move from account paper cash). Or disable and enable again.",
            row.enabled,
            funded,
            None,
        ));
    }

    match row.sizing_mode.as_str() {
        "proportional" if row.proportion.is_none() => {
            return Ok(blocked(
                ReadinessCode::MissingSizingConfig,
                "Missing sizing config",
                "Set copy % of source trade notional in Settings.",
                row.enabled,
                funded,
                None,
            ));
        }
        "fixed_amount" if row.fixed_amount_usd.is_none() => {
            return Ok(blocked(
                ReadinessCode::MissingSizingConfig,
                "Missing sizing config",
                "Set fixed amount (USD) per trade in Settings.",
                row.enabled,
                funded,
                None,
            ));
        }
        "multiplier" if row.multiplier.is_none() => {
            return Ok(blocked(
                ReadinessCode::MissingSizingConfig,
                "Missing sizing config",
                "Set multiplier in Settings.",
                row.enabled,
                funded,
                None,
            ));
        }
        "portfolio_ratio" => {
            let snap = source_equity::load_snapshot(pool, row.source_trader_id, cfg).await?;
            let _status = snap.as_ref().map(|s| s.status);
            let allows = snap
                .as_ref()
                .map(|s| s.status.allows_portfolio_buy())
                .unwrap_or(false);
            if !allows {
                let st = snap
                    .as_ref()
                    .map(|s| s.status.as_str())
                    .unwrap_or("unavailable");
                return Ok(blocked(
                    ReadinessCode::SourceEquityUnavailable,
                    "Source equity unavailable",
                    format!(
                        "Portfolio-relative sizing needs a usable equity snapshot for the leader wallet (current status: {st}). Check Data API / POLYGON_RPC_URL on the worker, or switch sizing to Fixed amount or Proportional."
                    ),
                    row.enabled,
                    funded,
                    Some(st.to_string()),
                ));
            }
        }
        "position_value_ratio" => {
            let snap = source_equity::load_snapshot(pool, row.source_trader_id, cfg).await?;
            let st = snap.as_ref().map(|s| s.status.as_str()).unwrap_or("missing");
            match snap.as_ref() {
                Some(s) => match source_equity::position_value_for_sizing(s) {
                    Ok(_pv) => {}
                    Err(crate::engine::RejectReason::SourcePositionValueStale) => {
                        return Ok(blocked(
                            ReadinessCode::SourcePositionValueUnavailable,
                            "Open-position value stale",
                            "Position Value Ratio needs a fresh open-position value snapshot (market value of leader positions from the Data API). Refresh monitoring or wait for the worker to update.".to_string(),
                            row.enabled,
                            funded,
                            Some(st.to_string()),
                        ));
                    }
                    Err(crate::engine::RejectReason::SourcePositionValueZero) => {
                        return Ok(blocked(
                            ReadinessCode::SourcePositionValueUnavailable,
                            "Open-position value is zero",
                            "The leader has no open-position market value on record. Position Value Ratio cannot scale buys until they hold positions.".to_string(),
                            row.enabled,
                            funded,
                            Some(st.to_string()),
                        ));
                    }
                    Err(_) => {
                        return Ok(blocked(
                            ReadinessCode::SourcePositionValueUnavailable,
                            "Open-position value unavailable",
                            format!(
                                "Position Value Ratio uses leader open-position market value (not wallet equity). Current snapshot status: {st}."
                            ),
                            row.enabled,
                            funded,
                            Some(st.to_string()),
                        ));
                    }
                },
                None => {
                    return Ok(blocked(
                        ReadinessCode::SourcePositionValueUnavailable,
                        "Open-position value unavailable",
                        "No equity/position snapshot for the leader yet. Enable source monitoring and wait for the worker.".to_string(),
                        row.enabled,
                        funded,
                        None,
                    ));
                }
            }
        }
        _ => {}
    }

    Ok(StrategyReadiness {
        code: ReadinessCode::ReadyToTrade,
        label: "Ready to trade",
        detail: "Pipeline checks passed; new source events can create copy decisions (individual trades may still be rejected for liquidity or risk).".into(),
        ready: true,
        enabled: row.enabled,
        capital_funded_usd: Some(funded_amt),
        capital_bootstrap_quality: funded.as_ref().map(|(_, q)| q.clone()),
        source_equity_status: if row.sizing_mode == "portfolio_ratio"
            || row.sizing_mode == "position_value_ratio"
        {
            let snap = source_equity::load_snapshot(pool, row.source_trader_id, cfg).await?;
            snap.map(|s| s.status.as_str().to_string())
        } else {
            None
        },
    })
}

fn blocked(
    code: ReadinessCode,
    label: &'static str,
    detail: impl Into<String>,
    enabled: bool,
    funded: Option<(Decimal, String)>,
    source_equity_status: Option<String>,
) -> StrategyReadiness {
    StrategyReadiness {
        code,
        label,
        detail: detail.into(),
        ready: false,
        enabled,
        capital_funded_usd: funded.as_ref().map(|(f, _)| *f),
        capital_bootstrap_quality: funded.as_ref().map(|(_, q)| q.clone()),
        source_equity_status,
    }
}
