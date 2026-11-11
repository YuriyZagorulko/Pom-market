use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::copy_sizing::{
    capped_copy_notional, raw_copy_notional, source_trade_percent, BuyNotionalLimits,
    StrategyModeParams,
};
use crate::engine::{plan_exit, truncate_lot, RejectReason, SizingMode, EXCHANGE_MIN_NOTIONAL};
use crate::source_equity::EquityStatus;

#[derive(Clone, Debug, Deserialize)]
pub struct CopyPreviewParams {
    pub sizing_mode: String,
    pub fixed_amount_usd: Option<Decimal>,
    /// Coefficient 0–1 (same as DB `proportion`).
    pub proportion: Option<Decimal>,
    pub multiplier: Option<Decimal>,
    /// Effective strategy capital cap (after fixed/percent allocation).
    pub effective_allocation_cap_usd: Decimal,
    pub max_position_usd: Decimal,
    pub max_exposure_usd: Decimal,
    pub account_max_exposure: Decimal,
    pub max_order_usd: Decimal,
    pub min_order_usd: Decimal,
    pub copy_buy: bool,
    pub copy_sell: bool,
    pub source_equity_usd: Option<Decimal>,
    pub source_equity_status: Option<EquityStatus>,
    /// Open-position MTM for position_value_ratio preview rows.
    pub source_open_positions_value_usd: Option<Decimal>,
    pub scaling_ratio_percent: Option<Decimal>,
    #[serde(default)]
    pub paper_trading: bool,
    #[serde(default)]
    pub allow_subminimum_paper_trades: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PreviewRow {
    pub source_event_id: Uuid,
    pub event_ts: DateTime<Utc>,
    pub side: String,
    pub market_title: Option<String>,
    pub market_icon: Option<String>,
    pub outcome: Option<String>,
    pub source_notional_usd: Decimal,
    pub source_equity_usd: Option<Decimal>,
    pub source_trade_percent: Option<Decimal>,
    pub target_allocation_usd: Decimal,
    pub scaling_ratio_percent: Option<Decimal>,
    pub raw_copy_notional_usd: Option<Decimal>,
    pub final_notional_usd: Option<Decimal>,
    pub available_budget_before_usd: Decimal,
    pub available_budget_after_usd: Decimal,
    pub result: String,
    pub reason: Option<String>,
}

#[derive(Clone, Debug)]
struct SimPosition {
    open_shares: Decimal,
    source_observed_shares: Decimal,
    cost_basis_usd: Decimal,
    pending_exit_shares: Decimal,
}

pub fn parse_sizing_mode(mode: &str) -> SizingMode {
    match mode {
        "proportional" => SizingMode::Proportional,
        "multiplier" => SizingMode::Multiplier,
        "portfolio_ratio" => SizingMode::PortfolioRatio,
        "position_value_ratio" => SizingMode::PositionValueRatio,
        _ => SizingMode::FixedAmount,
    }
}

/// UI percent → DB coefficient (`proportion`). Standard copy % is 1–100; legacy multipliers may exceed 100 (e.g. 150 → 1.5).
pub fn percent_to_proportion(percent: Decimal) -> Option<Decimal> {
    let max = Decimal::from(10_000);
    if percent <= Decimal::ZERO || percent > max {
        return None;
    }
    Some((percent / Decimal::from(100)).round_dp(8))
}

pub fn proportion_to_percent(proportion: Decimal) -> Decimal {
    (proportion * Decimal::from(100)).round_dp(4)
}

/// Thin wrapper kept for formula tests; delegates to shared sizing.
pub fn portfolio_ratio_notional(
    source_notional: Decimal,
    strategy_allocated: Decimal,
    source_equity: Decimal,
) -> Result<Decimal, RejectReason> {
    let params = StrategyModeParams {
        sizing_mode: "portfolio_ratio".into(),
        fixed_amount_usd: None,
        proportion: None,
        multiplier: None,
        strategy_allocated_capital_usd: strategy_allocated,
        source_trader_equity_usd: Some(source_equity),
        source_equity_status: Some(EquityStatus::Partial),
        source_open_positions_value_usd: None,
    };
    raw_copy_notional(&params, source_notional)
}

/// Thin wrapper for formula tests.
pub fn position_value_ratio_notional(
    source_notional: Decimal,
    strategy_allocated: Decimal,
    open_positions_value: Decimal,
    trade_multiplier: Option<Decimal>,
) -> Result<Decimal, RejectReason> {
    crate::copy_sizing::position_value_ratio_notional(
        source_notional,
        strategy_allocated,
        open_positions_value,
        trade_multiplier,
    )
}

fn mode_params(params: &CopyPreviewParams) -> StrategyModeParams {
    StrategyModeParams {
        sizing_mode: params.sizing_mode.clone(),
        fixed_amount_usd: params.fixed_amount_usd,
        proportion: params.proportion,
        multiplier: params.multiplier,
        strategy_allocated_capital_usd: params.effective_allocation_cap_usd,
        source_trader_equity_usd: params.source_equity_usd,
        source_equity_status: params.source_equity_status,
        source_open_positions_value_usd: params.source_open_positions_value_usd,
    }
}

#[derive(Clone, Debug)]
pub struct PreviewEvent {
    pub id: Uuid,
    pub token_id: String,
    pub side: String,
    pub price: Decimal,
    pub size_shares: Decimal,
    pub notional_usd: Decimal,
    pub event_ts: DateTime<Utc>,
    pub market_title: Option<String>,
    pub market_icon: Option<String>,
    pub outcome: Option<String>,
    pub condition_id: Option<String>,
    pub market_slug: Option<String>,
    pub allowed_check: bool,
}

/// Read-only estimate. Assumes immediate full fills (reserved capital stays zero);
/// live/paper may briefly reserve open orders before settlement.
pub fn simulate_copy_preview(
    events: &[PreviewEvent],
    params: &CopyPreviewParams,
) -> Vec<PreviewRow> {
    let mut sorted: Vec<&PreviewEvent> = events.iter().collect();
    sorted.sort_by_key(|e| e.event_ts);

    let mut deployed = Decimal::ZERO;
    let reserved = Decimal::ZERO;
    let mut lots: HashMap<String, SimPosition> = HashMap::new();
    let mut rows = Vec::with_capacity(sorted.len());
    let mode = mode_params(params);

    for event in sorted {
        let available_before =
            (params.effective_allocation_cap_usd - deployed - reserved).max(Decimal::ZERO);
        let side_buy = event.side == "buy";
        let mut result = "can_copy".to_string();
        let mut reason: Option<String> = None;
        let mut raw_copy: Option<Decimal> = None;
        let mut final_notional: Option<Decimal> = None;
        let reference_usd = if params.sizing_mode == "position_value_ratio" {
            params.source_open_positions_value_usd
        } else {
            params.source_equity_usd
        };
        let src_pct = reference_usd.and_then(|v| source_trade_percent(event.notional_usd, v));

        if !event.allowed_check {
            result = "rejected".into();
            reason = Some(RejectReason::MarketNotAllowed.as_str().into());
        } else if side_buy && !params.copy_buy || !side_buy && !params.copy_sell {
            result = "rejected".into();
            reason = Some(RejectReason::CopySideDisabled.as_str().into());
        } else if side_buy {
            match raw_copy_notional(&mode, event.notional_usd) {
                Ok(raw) => {
                    raw_copy = Some(raw);
                    let limits = BuyNotionalLimits {
                        max_order_usd: params.max_order_usd,
                        max_position_usd: params.max_position_usd,
                        min_order_usd: params.min_order_usd,
                        strategy_remaining_usd: available_before,
                        available_balance_usd: Decimal::MAX,
                        allow_subminimum_paper_trades: params.paper_trading
                            && params.allow_subminimum_paper_trades,
                    };
                    match capped_copy_notional(&mode, event.notional_usd, &limits) {
                        Ok(notional) => {
                            final_notional = Some(notional);
                            let submin = params.paper_trading && params.allow_subminimum_paper_trades;
                            let exchange_floor = if submin {
                                if params.min_order_usd > Decimal::ZERO {
                                    params.min_order_usd
                                } else {
                                    Decimal::ZERO
                                }
                            } else {
                                params.min_order_usd.max(EXCHANGE_MIN_NOTIONAL)
                            };
                            if notional < raw {
                                result = if notional < exchange_floor {
                                    "rejected".into()
                                } else {
                                    "limited".into()
                                };
                                if result == "rejected" {
                                    reason = Some(RejectReason::StrategyBudget.as_str().into());
                                } else {
                                    reason = Some("capped_by_budget_or_risk".into());
                                }
                            }
                            if notional < exchange_floor || (submin && notional <= Decimal::ZERO)
                            {
                                result = "rejected".into();
                                reason = Some(RejectReason::BelowMinimum.as_str().into());
                                final_notional = None;
                            } else {
                                let lot =
                                    lots.entry(event.token_id.clone()).or_insert(SimPosition {
                                        open_shares: Decimal::ZERO,
                                        source_observed_shares: Decimal::ZERO,
                                        cost_basis_usd: Decimal::ZERO,
                                        pending_exit_shares: Decimal::ZERO,
                                    });
                                let shares =
                                    truncate_lot(notional / event.price.max(Decimal::new(1, 3)));
                                lot.open_shares += shares;
                                lot.source_observed_shares += event.size_shares;
                                lot.cost_basis_usd += notional;
                                deployed += notional;
                            }
                        }
                        Err(err) => {
                            result = "rejected".into();
                            reason = Some(err.as_str().into());
                        }
                    }
                }
                Err(err) => {
                    result = "rejected".into();
                    reason = Some(err.as_str().into());
                }
            }
        } else {
            let lot = lots.get(&event.token_id);
            let plan = plan_exit(
                lot.map(|l| l.open_shares).unwrap_or(Decimal::ZERO),
                lot.map(|l| l.source_observed_shares)
                    .unwrap_or(Decimal::ZERO),
                event.size_shares,
                lot.map(|l| l.pending_exit_shares).unwrap_or(Decimal::ZERO),
                event.price,
                params.min_order_usd,
                params.paper_trading && params.allow_subminimum_paper_trades,
            );
            if let Some(reject) = plan.reject {
                if let Some(lot) = lots.get_mut(&event.token_id) {
                    lot.pending_exit_shares = plan.pending_exit_shares;
                }
                result = "rejected".into();
                reason = Some(reject.as_str().into());
            } else {
                let shares = plan.shares;
                final_notional = Some((shares * event.price).round_dp(2));
                raw_copy = final_notional;
                if let Some(lot) = lots.get_mut(&event.token_id) {
                    let sold = shares.min(lot.open_shares);
                    let released = if lot.open_shares > Decimal::ZERO {
                        (lot.cost_basis_usd * sold / lot.open_shares).round_dp(8)
                    } else {
                        Decimal::ZERO
                    };
                    lot.open_shares -= sold;
                    lot.cost_basis_usd = (lot.cost_basis_usd - released).max(Decimal::ZERO);
                    lot.source_observed_shares =
                        (lot.source_observed_shares - event.size_shares).max(Decimal::ZERO);
                    lot.pending_exit_shares = plan.pending_exit_shares;
                    deployed = (deployed - released).max(Decimal::ZERO);
                }
            }
        }

        let available_after =
            (params.effective_allocation_cap_usd - deployed - reserved).max(Decimal::ZERO);
        rows.push(PreviewRow {
            source_event_id: event.id,
            event_ts: event.event_ts,
            side: event.side.clone(),
            market_title: event.market_title.clone(),
            market_icon: event.market_icon.clone(),
            outcome: event.outcome.clone(),
            source_notional_usd: event.notional_usd,
            source_equity_usd: params.source_equity_usd,
            source_trade_percent: src_pct,
            target_allocation_usd: params.effective_allocation_cap_usd,
            scaling_ratio_percent: params.scaling_ratio_percent,
            raw_copy_notional_usd: raw_copy,
            final_notional_usd: final_notional,
            available_budget_before_usd: available_before,
            available_budget_after_usd: available_after,
            result,
            reason,
        });
    }

    rows.reverse();
    rows
}

pub fn market_allowed_list(allowed: &Value, event: &PreviewEvent) -> bool {
    let Some(items) = allowed.as_array() else {
        return true;
    };
    if items.is_empty() {
        return true;
    }
    items.iter().any(|item| {
        let text = item.as_str().unwrap_or_default();
        text == event.token_id
            || event.condition_id.as_deref() == Some(text)
            || event.market_slug.as_deref() == Some(text)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::copy_sizing::scaling_ratio;
    use chrono::Utc;
    use rust_decimal::Decimal;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn base_params() -> CopyPreviewParams {
        CopyPreviewParams {
            sizing_mode: "proportional".into(),
            fixed_amount_usd: None,
            proportion: Some(d("0.1")),
            multiplier: None,
            effective_allocation_cap_usd: d("150"),
            max_position_usd: d("500"),
            max_exposure_usd: d("500"),
            account_max_exposure: d("500"),
            max_order_usd: d("500"),
            min_order_usd: d("1"),
            copy_buy: true,
            copy_sell: true,
            source_equity_usd: None,
            source_equity_status: None,
            source_open_positions_value_usd: None,
            scaling_ratio_percent: None,
            paper_trading: false,
            allow_subminimum_paper_trades: false,
        }
    }

    #[test]
    fn percent_to_proportion_allows_above_100_for_legacy_multiplier() {
        assert_eq!(percent_to_proportion(d("150")).unwrap(), d("1.5"));
        assert_eq!(percent_to_proportion(d("20")).unwrap(), d("0.2"));
    }

    #[test]
    fn percent_to_proportion_converts() {
        assert_eq!(percent_to_proportion(d("10")).unwrap(), d("0.1"));
        assert_eq!(percent_to_proportion(d("100")).unwrap(), d("1"));
        assert!(percent_to_proportion(d("0")).is_none());
        assert_eq!(percent_to_proportion(d("101")).unwrap(), d("1.01"));
        assert!(percent_to_proportion(d("10001")).is_none());
    }

    #[test]
    fn preview_budget_caps_buys() {
        let params = base_params();
        let events = vec![PreviewEvent {
            id: Uuid::new_v4(),
            token_id: "tok".into(),
            side: "buy".into(),
            price: d("0.5"),
            size_shares: d("2000"),
            notional_usd: d("1000"),
            event_ts: Utc::now(),
            market_title: None,
            market_icon: None,
            outcome: None,
            condition_id: None,
            market_slug: None,
            allowed_check: true,
        }];
        let rows = simulate_copy_preview(&events, &params);
        assert_eq!(rows[0].final_notional_usd, Some(d("100.00")));
        assert_eq!(rows[0].result, "can_copy");

        let events2 = vec![
            PreviewEvent {
                id: Uuid::new_v4(),
                token_id: "tok".into(),
                side: "buy".into(),
                price: d("0.5"),
                size_shares: d("2000"),
                notional_usd: d("1000"),
                event_ts: Utc::now(),
                market_title: None,
                market_icon: None,
                outcome: None,
                condition_id: None,
                market_slug: None,
                allowed_check: true,
            },
            PreviewEvent {
                id: Uuid::new_v4(),
                token_id: "tok2".into(),
                side: "buy".into(),
                price: d("0.5"),
                size_shares: d("2000"),
                notional_usd: d("1000"),
                event_ts: Utc::now(),
                market_title: None,
                market_icon: None,
                outcome: None,
                condition_id: None,
                market_slug: None,
                allowed_check: true,
            },
        ];
        let rows2 = simulate_copy_preview(&events2, &params);
        assert!(rows2
            .iter()
            .any(|r| r.result == "limited" || r.result == "rejected"));
    }

    #[test]
    fn portfolio_preview_matches_shared_sizing() {
        let params = CopyPreviewParams {
            sizing_mode: "portfolio_ratio".into(),
            fixed_amount_usd: None,
            proportion: None,
            multiplier: None,
            effective_allocation_cap_usd: d("2000"),
            max_position_usd: d("5000"),
            max_exposure_usd: d("5000"),
            account_max_exposure: d("5000"),
            max_order_usd: d("5000"),
            min_order_usd: d("1"),
            copy_buy: true,
            copy_sell: true,
            source_equity_usd: Some(d("20000")),
            source_equity_status: Some(EquityStatus::Partial),
            source_open_positions_value_usd: None,
            scaling_ratio_percent: scaling_ratio(d("2000"), d("20000")),
            paper_trading: false,
            allow_subminimum_paper_trades: false,
        };
        let events = vec![PreviewEvent {
            id: Uuid::new_v4(),
            token_id: "tok".into(),
            side: "buy".into(),
            price: d("0.5"),
            size_shares: d("8000"),
            notional_usd: d("4000"),
            event_ts: Utc::now(),
            market_title: None,
            market_icon: None,
            outcome: None,
            condition_id: None,
            market_slug: None,
            allowed_check: true,
        }];
        let rows = simulate_copy_preview(&events, &params);
        assert_eq!(rows[0].raw_copy_notional_usd, Some(d("400")));
        assert_eq!(rows[0].final_notional_usd, Some(d("400")));
    }

    #[test]
    fn paper_preview_allows_sub_dollar_pvr() {
        let mut params = CopyPreviewParams {
            sizing_mode: "position_value_ratio".into(),
            fixed_amount_usd: None,
            proportion: None,
            multiplier: None,
            effective_allocation_cap_usd: d("2000"),
            max_position_usd: d("5000"),
            max_exposure_usd: d("5000"),
            account_max_exposure: d("5000"),
            max_order_usd: d("50"),
            min_order_usd: Decimal::ZERO,
            copy_buy: true,
            copy_sell: true,
            source_equity_usd: None,
            source_equity_status: Some(EquityStatus::Unavailable),
            source_open_positions_value_usd: Some(d("500000")),
            scaling_ratio_percent: None,
            paper_trading: true,
            allow_subminimum_paper_trades: true,
        };
        let events = vec![PreviewEvent {
            id: Uuid::new_v4(),
            token_id: "tok".into(),
            side: "buy".into(),
            price: d("0.50"),
            size_shares: d("100"),
            notional_usd: d("50"),
            event_ts: Utc::now(),
            market_title: None,
            market_icon: None,
            outcome: None,
            condition_id: None,
            market_slug: None,
            allowed_check: true,
        }];
        let rows = simulate_copy_preview(&events, &params);
        let n = rows[0].final_notional_usd.unwrap();
        assert!(n < Decimal::ONE);
        assert_eq!(rows[0].result, "can_copy");
    }

    #[test]
    fn position_value_preview_matches_shared_sizing() {
        let params = CopyPreviewParams {
            sizing_mode: "position_value_ratio".into(),
            fixed_amount_usd: None,
            proportion: None,
            multiplier: None,
            effective_allocation_cap_usd: d("2000"),
            max_position_usd: d("5000"),
            max_exposure_usd: d("5000"),
            account_max_exposure: d("5000"),
            max_order_usd: d("5000"),
            min_order_usd: d("1"),
            copy_buy: true,
            copy_sell: true,
            source_equity_usd: None,
            source_equity_status: Some(EquityStatus::Unavailable),
            source_open_positions_value_usd: Some(d("4000")),
            scaling_ratio_percent: None,
            paper_trading: false,
            allow_subminimum_paper_trades: false,
        };
        let events = vec![PreviewEvent {
            id: Uuid::new_v4(),
            token_id: "tok".into(),
            side: "buy".into(),
            price: d("0.5"),
            size_shares: d("400"),
            notional_usd: d("200"),
            event_ts: Utc::now(),
            market_title: None,
            market_icon: None,
            outcome: None,
            condition_id: None,
            market_slug: None,
            allowed_check: true,
        }];
        let rows = simulate_copy_preview(&events, &params);
        assert_eq!(rows[0].raw_copy_notional_usd, Some(d("100")));
        assert_eq!(rows[0].source_trade_percent, Some(d("5")));
    }
}
