use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::engine::{
    plan_exit, target_notional, truncate_lot, RejectReason, SizingInput, SizingMode,
    EXCHANGE_MIN_NOTIONAL,
};

#[derive(Clone, Debug, Deserialize)]
pub struct CopyPreviewParams {
    pub sizing_mode: String,
    pub fixed_amount_usd: Option<Decimal>,
    /// Coefficient 0–1 (same as DB `proportion`).
    pub proportion: Option<Decimal>,
    pub multiplier: Option<Decimal>,
    pub strategy_budget_usd: Decimal,
    pub max_position_usd: Decimal,
    pub max_exposure_usd: Decimal,
    pub account_max_exposure: Decimal,
    pub max_order_usd: Decimal,
    pub min_order_usd: Decimal,
    pub copy_buy: bool,
    pub copy_sell: bool,
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
        _ => SizingMode::FixedAmount,
    }
}

/// Percent UX → DB coefficient.
pub fn percent_to_proportion(percent: Decimal) -> Option<Decimal> {
    if percent <= Decimal::ZERO || percent > Decimal::from(100) {
        return None;
    }
    Some((percent / Decimal::from(100)).round_dp(8))
}

pub fn proportion_to_percent(proportion: Decimal) -> Decimal {
    (proportion * Decimal::from(100)).round_dp(4)
}

fn raw_notional(
    params: &CopyPreviewParams,
    source_notional: Decimal,
) -> Result<Decimal, RejectReason> {
    match parse_sizing_mode(&params.sizing_mode) {
        SizingMode::FixedAmount => params.fixed_amount_usd.ok_or(RejectReason::MissingSizing),
        SizingMode::Proportional => {
            Ok(source_notional * params.proportion.ok_or(RejectReason::MissingSizing)?)
        }
        SizingMode::Multiplier => {
            Ok(source_notional * params.multiplier.ok_or(RejectReason::MissingSizing)?)
        }
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

    for event in sorted {
        let available_before =
            (params.strategy_budget_usd - deployed - reserved).max(Decimal::ZERO);
        let side_buy = event.side == "buy";
        let mut result = "can_copy".to_string();
        let mut reason: Option<String> = None;
        let mut raw_copy: Option<Decimal> = None;
        let mut final_notional: Option<Decimal> = None;

        if !event.allowed_check {
            result = "rejected".into();
            reason = Some(RejectReason::MarketNotAllowed.as_str().into());
        } else if side_buy && !params.copy_buy || !side_buy && !params.copy_sell {
            result = "rejected".into();
            reason = Some(RejectReason::CopySideDisabled.as_str().into());
        } else if side_buy {
            match raw_notional(params, event.notional_usd) {
                Ok(raw) => {
                    raw_copy = Some(raw);
                    let sizing = SizingInput {
                        mode: parse_sizing_mode(&params.sizing_mode),
                        fixed_amount_usd: params.fixed_amount_usd,
                        proportion: params.proportion,
                        multiplier: params.multiplier,
                        source_notional: event.notional_usd,
                        max_order_usd: params.max_order_usd.min(params.max_position_usd),
                        min_order_usd: params.min_order_usd,
                        strategy_remaining_usd: available_before,
                        account_exposure_remaining_usd: params
                            .account_max_exposure
                            .min(params.max_exposure_usd),
                        available_balance_usd: Decimal::MAX,
                    };
                    match target_notional(&sizing) {
                        Ok(notional) => {
                            final_notional = Some(notional);
                            if notional < raw {
                                result =
                                    if notional < params.min_order_usd.max(EXCHANGE_MIN_NOTIONAL) {
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
                            if notional < params.min_order_usd.max(EXCHANGE_MIN_NOTIONAL) {
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

        let available_after = (params.strategy_budget_usd - deployed - reserved).max(Decimal::ZERO);
        rows.push(PreviewRow {
            source_event_id: event.id,
            event_ts: event.event_ts,
            side: event.side.clone(),
            market_title: event.market_title.clone(),
            market_icon: event.market_icon.clone(),
            outcome: event.outcome.clone(),
            source_notional_usd: event.notional_usd,
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
    use chrono::Utc;
    use rust_decimal::Decimal;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn percent_to_proportion_converts() {
        assert_eq!(percent_to_proportion(d("10")).unwrap(), d("0.1"));
        assert_eq!(percent_to_proportion(d("100")).unwrap(), d("1"));
        assert!(percent_to_proportion(d("0")).is_none());
        assert!(percent_to_proportion(d("101")).is_none());
    }

    #[test]
    fn preview_budget_caps_buys() {
        let params = CopyPreviewParams {
            sizing_mode: "proportional".into(),
            fixed_amount_usd: None,
            proportion: Some(d("0.1")),
            multiplier: None,
            strategy_budget_usd: d("150"),
            max_position_usd: d("500"),
            max_exposure_usd: d("500"),
            account_max_exposure: d("500"),
            max_order_usd: d("500"),
            min_order_usd: d("1"),
            copy_buy: true,
            copy_sell: true,
        };
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
}
