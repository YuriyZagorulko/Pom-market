//! P0 copy-trading: small BUY override, min-size audit, share-fraction SELL.

use pommarket::copy_sizing::{
    buy_minimum_rejection_audit, capped_copy_notional, raw_copy_notional, sizing_rule_label,
    small_buy_1to1_applies, BuyNotionalLimits, StrategyModeParams,
};
use pommarket::engine::{plan_exit, target_notional, RejectReason, SizingInput, SizingMode};
use rust_decimal::Decimal;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn prop_small_buy(coef: &str) -> StrategyModeParams {
    StrategyModeParams {
        sizing_mode: "proportional".into(),
        fixed_amount_usd: None,
        proportion: Some(d(coef)),
        multiplier: None,
        strategy_allocated_capital_usd: d("8000"),
        source_trader_equity_usd: None,
        source_equity_status: None,
        source_open_positions_value_usd: None,
        copy_small_buys_1to1: true,
        small_buy_threshold_usd: Some(d("5")),
    }
}

#[test]
fn small_buy_override_proportional_leader_four_not_point_four() {
    let params = prop_small_buy("1");
    assert_eq!(raw_copy_notional(&params, d("4")).unwrap(), d("4"));
    assert_eq!(sizing_rule_label(&params, d("4")), "small_buy_override");
    let limits = BuyNotionalLimits {
        max_order_usd: d("50"),
        max_position_usd: d("200"),
        min_order_usd: d("1"),
        strategy_remaining_usd: d("8000"),
        available_balance_usd: d("8000"),
        allow_subminimum_paper_trades: false,
    };
    assert_eq!(capped_copy_notional(&params, d("4"), &limits).unwrap(), d("4"));
}

#[test]
fn small_buy_threshold_boundary_cases() {
    let params = prop_small_buy("1");
    for src in ["0.50", "1.00", "4.99", "5.00"] {
        let n = d(src);
        assert!(
            small_buy_1to1_applies(&params, n),
            "expected override at {src}"
        );
        assert_eq!(raw_copy_notional(&params, n).unwrap(), n);
    }
    assert!(!small_buy_1to1_applies(&params, d("5.01")));
    assert_eq!(raw_copy_notional(&params, d("5.01")).unwrap(), d("5.01"));
}

#[test]
fn small_buy_multiplier_applied_once() {
    let params = prop_small_buy("0.5");
    assert_eq!(raw_copy_notional(&params, d("4")).unwrap(), d("2"));
    assert!(!small_buy_1to1_applies(&params, d("6")));
    assert_eq!(raw_copy_notional(&params, d("6")).unwrap(), d("3"));
}

#[test]
fn pvr_small_buy_at_threshold_skips_portfolio_denominator() {
    let params = StrategyModeParams {
        sizing_mode: "position_value_ratio".into(),
        fixed_amount_usd: None,
        proportion: None,
        multiplier: None,
        strategy_allocated_capital_usd: d("2000"),
        source_trader_equity_usd: None,
        source_equity_status: None,
        source_open_positions_value_usd: None,
        copy_small_buys_1to1: true,
        small_buy_threshold_usd: Some(d("5")),
    };
    assert_eq!(raw_copy_notional(&params, d("5")).unwrap(), d("5"));
}

#[test]
fn fixed_small_buy_override_below_threshold() {
    let params = StrategyModeParams {
        sizing_mode: "fixed_amount".into(),
        fixed_amount_usd: Some(d("25")),
        proportion: None,
        multiplier: None,
        strategy_allocated_capital_usd: d("2000"),
        source_trader_equity_usd: None,
        source_equity_status: None,
        source_open_positions_value_usd: None,
        copy_small_buys_1to1: true,
        small_buy_threshold_usd: Some(d("5")),
    };
    assert_eq!(raw_copy_notional(&params, d("3")).unwrap(), d("3"));
    assert_eq!(raw_copy_notional(&params, d("10")).unwrap(), d("25"));
}

#[test]
fn sell_reconciliation_leader_half_follower_sixty() {
    let plan = plan_exit(d("60"), d("100"), d("50"), Decimal::ZERO, d("0.5"), d("1"), false);
    assert!(plan.reject.is_none());
    assert_eq!(plan.shares, d("30"));
}

#[test]
fn sell_no_position_when_follower_never_bought() {
    let plan = plan_exit(Decimal::ZERO, Decimal::ZERO, d("50"), Decimal::ZERO, d("0.5"), d("1"), false);
    assert_eq!(plan.reject, Some(RejectReason::NoPosition));
}

#[test]
fn below_minimum_shares_audit_has_required_notional() {
    let audit = buy_minimum_rejection_audit(d("0.63"), d("2.38"), d("5"), true, false);
    assert_eq!(audit["below_minimum_detail"], "below_market_min_shares");
    assert_eq!(audit["minimum_required_notional"], "3.15");
}

#[test]
fn target_notional_respects_budget_after_small_buy() {
    let input = SizingInput {
        mode: SizingMode::Proportional,
        fixed_amount_usd: None,
        proportion: Some(d("1")),
        multiplier: None,
        source_notional: d("4"),
        max_order_usd: d("100"),
        min_order_usd: d("1"),
        strategy_remaining_usd: d("2"),
        available_balance_usd: d("100"),
        strategy_allocated_capital_usd: None,
        source_trader_equity_usd: None,
        source_open_positions_value_usd: None,
        allow_subminimum_paper_trades: false,
        copy_small_buys_1to1: true,
        small_buy_threshold_usd: Some(d("5")),
    };
    assert_eq!(target_notional(&input).unwrap(), d("2"));
}