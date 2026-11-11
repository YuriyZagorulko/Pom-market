//! Single path for copy BUY notional sizing (pipeline + preview).
//!
//! ## Position Value Ratio valuation basis
//! Uses `source_trader_equity_snapshots.positions_value_usd` — aggregate **open-position market value**
//! from Polymarket Data API `/v2/value` (MTM). Does not use wallet USDC or `total_equity_usd`.
//! Optional `multiplier` on the strategy scales the PVR result (`NULL` = 1×).

use rust_decimal::Decimal;

use crate::engine::{target_notional, RejectReason, SizingInput, SizingMode};
use crate::source_equity::EquityStatus;

#[derive(Clone, Debug)]
pub struct BuyNotionalLimits {
    pub max_order_usd: Decimal,
    pub max_position_usd: Decimal,
    pub min_order_usd: Decimal,
    pub strategy_remaining_usd: Decimal,
    pub available_balance_usd: Decimal,
    /// PAPER strategy opt-in: skip Polymarket $1 / min-shares floors in sizing (not LIVE).
    pub allow_subminimum_paper_trades: bool,
}

#[derive(Clone, Debug)]
pub struct StrategyModeParams {
    pub sizing_mode: String,
    pub fixed_amount_usd: Option<Decimal>,
    pub proportion: Option<Decimal>,
    pub multiplier: Option<Decimal>,
    pub strategy_allocated_capital_usd: Decimal,
    pub source_trader_equity_usd: Option<Decimal>,
    pub source_equity_status: Option<EquityStatus>,
    /// Leader open-position MTM (`/v2/value`) for `position_value_ratio`.
    pub source_open_positions_value_usd: Option<Decimal>,
}

/// Coefficient after base PVR notional (`NULL` or absent → 1).
pub fn pvr_trade_multiplier(multiplier: Option<Decimal>) -> Decimal {
    multiplier.filter(|m| *m > Decimal::ZERO).unwrap_or(Decimal::ONE)
}

const PVR_MULTIPLIER_MAX: Decimal = Decimal::from_parts(10000, 0, 0, false, 0);

/// API validation for PVR trade multiplier (`None` → 1× at sizing time).
pub fn parse_pvr_multiplier(value: Option<Decimal>) -> Result<Option<Decimal>, &'static str> {
    let Some(m) = value else {
        return Ok(None);
    };
    if m <= Decimal::ZERO {
        return Err("PVR trade multiplier must be greater than zero");
    }
    if m > PVR_MULTIPLIER_MAX {
        return Err("PVR trade multiplier must be at most 10000");
    }
    Ok(Some(m.round_dp(8)))
}

/// Raw copy notional before risk caps (same formula pipeline uses).
pub fn raw_copy_notional(
    params: &StrategyModeParams,
    source_notional: Decimal,
) -> Result<Decimal, RejectReason> {
    let mode = parse_mode(&params.sizing_mode);
    if source_notional <= Decimal::ZERO {
        return Err(RejectReason::BelowMinimum);
    }
    if params.strategy_allocated_capital_usd <= Decimal::ZERO {
        return Err(RejectReason::MissingSizing);
    }
    if mode == SizingMode::PortfolioRatio {
        let status = params
            .source_equity_status
            .ok_or(RejectReason::SourceEquityUnavailable)?;
        if !status.allows_portfolio_buy() {
            return Err(RejectReason::SourceEquityUnavailable);
        }
        let equity = params
            .source_trader_equity_usd
            .filter(|v| *v > Decimal::ZERO)
            .ok_or(RejectReason::SourceEquityUnavailable)?;
        return Ok((source_notional * params.strategy_allocated_capital_usd / equity).round_dp(8));
    }
    if mode == SizingMode::PositionValueRatio {
        let positions = match params.source_open_positions_value_usd {
            None => return Err(RejectReason::SourcePositionValueUnavailable),
            Some(v) if v <= Decimal::ZERO => return Err(RejectReason::SourcePositionValueZero),
            Some(positions) => positions,
        };
        let base =
            source_notional * params.strategy_allocated_capital_usd / positions;
        return Ok((base * pvr_trade_multiplier(params.multiplier)).round_dp(8));
    }
    match mode {
        SizingMode::FixedAmount => params.fixed_amount_usd.ok_or(RejectReason::MissingSizing),
        SizingMode::Proportional => {
            Ok(source_notional * params.proportion.ok_or(RejectReason::MissingSizing)?)
        }
        SizingMode::Multiplier => {
            Ok(source_notional * params.multiplier.ok_or(RejectReason::MissingSizing)?)
        }
        SizingMode::PortfolioRatio | SizingMode::PositionValueRatio => {
            Err(RejectReason::MissingSizing)
        }
    }
}

pub fn capped_copy_notional(
    params: &StrategyModeParams,
    source_notional: Decimal,
    limits: &BuyNotionalLimits,
) -> Result<Decimal, RejectReason> {
    let _raw = raw_copy_notional(params, source_notional)?;
    let mode = parse_mode(&params.sizing_mode);
    let alloc_modes = mode == SizingMode::PortfolioRatio || mode == SizingMode::PositionValueRatio;
    let sizing = SizingInput {
        mode,
        fixed_amount_usd: params.fixed_amount_usd,
        proportion: params.proportion,
        multiplier: params.multiplier,
        source_notional,
        max_order_usd: limits.max_order_usd.min(limits.max_position_usd),
        min_order_usd: limits.min_order_usd,
        strategy_remaining_usd: limits.strategy_remaining_usd,
        available_balance_usd: limits.available_balance_usd,
        strategy_allocated_capital_usd: if alloc_modes {
            Some(params.strategy_allocated_capital_usd)
        } else {
            None
        },
        source_trader_equity_usd: params.source_trader_equity_usd,
        source_open_positions_value_usd: params.source_open_positions_value_usd,
        allow_subminimum_paper_trades: limits.allow_subminimum_paper_trades,
    };
    target_notional(&sizing)
}

pub fn parse_mode(value: &str) -> SizingMode {
    match value {
        "proportional" => SizingMode::Proportional,
        "multiplier" => SizingMode::Multiplier,
        "portfolio_ratio" => SizingMode::PortfolioRatio,
        "position_value_ratio" => SizingMode::PositionValueRatio,
        _ => SizingMode::FixedAmount,
    }
}

pub fn scaling_ratio(allocated: Decimal, source_equity: Decimal) -> Option<Decimal> {
    if source_equity > Decimal::ZERO && allocated > Decimal::ZERO {
        Some((allocated / source_equity * Decimal::from(100)).round_dp(4))
    } else {
        None
    }
}

pub fn source_trade_percent(source_notional: Decimal, reference_usd: Decimal) -> Option<Decimal> {
    if reference_usd > Decimal::ZERO && source_notional > Decimal::ZERO {
        Some((source_notional / reference_usd * Decimal::from(100)).round_dp(4))
    } else {
        None
    }
}

/// Shared formula for tests: `allocation × source_notional / open_position_value × multiplier`.
pub fn position_value_ratio_notional(
    source_notional: Decimal,
    strategy_allocated: Decimal,
    open_positions_value: Decimal,
    trade_multiplier: Option<Decimal>,
) -> Result<Decimal, RejectReason> {
    let params = StrategyModeParams {
        sizing_mode: "position_value_ratio".into(),
        fixed_amount_usd: None,
        proportion: None,
        multiplier: trade_multiplier,
        strategy_allocated_capital_usd: strategy_allocated,
        source_trader_equity_usd: None,
        source_equity_status: None,
        source_open_positions_value_usd: Some(open_positions_value),
    };
    raw_copy_notional(&params, source_notional)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_equity::EquityStatus;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn portfolio_raw_matches_spec() {
        let params = StrategyModeParams {
            sizing_mode: "portfolio_ratio".into(),
            fixed_amount_usd: None,
            proportion: None,
            multiplier: None,
            strategy_allocated_capital_usd: d("2000"),
            source_trader_equity_usd: Some(d("20000")),
            source_equity_status: Some(EquityStatus::Partial),
            source_open_positions_value_usd: None,
        };
        assert_eq!(raw_copy_notional(&params, d("4000")).unwrap(), d("400"));
    }

    #[test]
    fn unavailable_equity_blocks_portfolio_only() {
        let params = StrategyModeParams {
            sizing_mode: "portfolio_ratio".into(),
            fixed_amount_usd: None,
            proportion: None,
            multiplier: None,
            strategy_allocated_capital_usd: d("2000"),
            source_trader_equity_usd: Some(d("20000")),
            source_equity_status: Some(EquityStatus::Unavailable),
            source_open_positions_value_usd: None,
        };
        assert_eq!(
            raw_copy_notional(&params, d("100")),
            Err(RejectReason::SourceEquityUnavailable)
        );
    }

    #[test]
    fn pvr_chihuahua_notional_matches_live_snapshot_scale() {
        let params = StrategyModeParams {
            sizing_mode: "position_value_ratio".into(),
            fixed_amount_usd: None,
            proportion: None,
            multiplier: None,
            strategy_allocated_capital_usd: d("2000"),
            source_trader_equity_usd: None,
            source_equity_status: Some(EquityStatus::Unavailable),
            source_open_positions_value_usd: Some(d("2201.1005502751377")),
        };
        let limits = BuyNotionalLimits {
            max_order_usd: d("50"),
            max_position_usd: d("200"),
            min_order_usd: d("1"),
            strategy_remaining_usd: d("2000"),
            available_balance_usd: d("2000"),
            allow_subminimum_paper_trades: false,
        };
        let capped = capped_copy_notional(&params, d("22"), &limits).unwrap();
        assert_eq!(capped, d("19.99"));
        assert_eq!(raw_copy_notional(&params, d("22")).unwrap().round_dp(2), d("19.99"));
    }

    #[test]
    fn pvr_raw_capped_by_max_order() {
        let params = StrategyModeParams {
            sizing_mode: "position_value_ratio".into(),
            fixed_amount_usd: None,
            proportion: None,
            multiplier: None,
            strategy_allocated_capital_usd: d("2000"),
            source_trader_equity_usd: None,
            source_equity_status: Some(EquityStatus::Unavailable),
            source_open_positions_value_usd: Some(d("1044.70")),
        };
        let limits = BuyNotionalLimits {
            max_order_usd: d("50"),
            max_position_usd: d("200"),
            min_order_usd: d("1"),
            strategy_remaining_usd: d("2000"),
            available_balance_usd: d("2000"),
            allow_subminimum_paper_trades: false,
        };
        assert!(raw_copy_notional(&params, d("384")).unwrap() > d("50"));
        assert_eq!(capped_copy_notional(&params, d("384"), &limits).unwrap(), d("50"));
    }

    #[test]
    fn position_value_ratio_examples() {
        assert_eq!(
            position_value_ratio_notional(d("200"), d("2000"), d("4000"), None).unwrap(),
            d("100")
        );
        assert_eq!(
            position_value_ratio_notional(d("100"), d("2000"), d("4000"), None).unwrap(),
            d("50")
        );
        assert_eq!(
            position_value_ratio_notional(d("200"), d("2000"), d("4000"), Some(d("10"))).unwrap(),
            d("1000")
        );
        assert_eq!(
            position_value_ratio_notional(d("200"), d("2000"), d("4000"), Some(d("0.5"))).unwrap(),
            d("50")
        );
    }

    #[test]
    fn position_value_zero_and_missing() {
        assert_eq!(
            position_value_ratio_notional(d("200"), d("2000"), d("0"), None),
            Err(RejectReason::SourcePositionValueZero)
        );
        let params = StrategyModeParams {
            sizing_mode: "position_value_ratio".into(),
            fixed_amount_usd: None,
            proportion: None,
            multiplier: None,
            strategy_allocated_capital_usd: d("2000"),
            source_trader_equity_usd: None,
            source_equity_status: None,
            source_open_positions_value_usd: None,
        };
        assert_eq!(
            raw_copy_notional(&params, d("200")),
            Err(RejectReason::SourcePositionValueUnavailable)
        );
    }

    #[test]
    fn position_value_without_wallet_equity() {
        let params = StrategyModeParams {
            sizing_mode: "position_value_ratio".into(),
            fixed_amount_usd: None,
            proportion: None,
            multiplier: None,
            strategy_allocated_capital_usd: d("2000"),
            source_trader_equity_usd: None,
            source_equity_status: Some(EquityStatus::Unavailable),
            source_open_positions_value_usd: Some(d("4000")),
        };
        assert_eq!(raw_copy_notional(&params, d("200")).unwrap(), d("100"));
    }

    #[test]
    fn bad_source_notional() {
        assert_eq!(
            position_value_ratio_notional(d("0"), d("2000"), d("4000"), None),
            Err(RejectReason::BelowMinimum)
        );
        assert_eq!(
            position_value_ratio_notional(d("-1"), d("2000"), d("4000"), None),
            Err(RejectReason::BelowMinimum)
        );
    }

    #[test]
    fn multiplier_matches_proportional_coefficient() {
        let mult = StrategyModeParams {
            sizing_mode: "multiplier".into(),
            fixed_amount_usd: None,
            proportion: None,
            multiplier: Some(d("0.2")),
            strategy_allocated_capital_usd: d("2000"),
            source_trader_equity_usd: None,
            source_equity_status: None,
            source_open_positions_value_usd: None,
        };
        let prop = StrategyModeParams {
            sizing_mode: "proportional".into(),
            fixed_amount_usd: None,
            proportion: Some(d("0.2")),
            multiplier: None,
            strategy_allocated_capital_usd: d("2000"),
            source_trader_equity_usd: None,
            source_equity_status: None,
            source_open_positions_value_usd: None,
        };
        let src = d("100");
        assert_eq!(raw_copy_notional(&mult, src).unwrap(), d("20"));
        assert_eq!(raw_copy_notional(&prop, src).unwrap(), d("20"));
    }

    #[test]
    fn capped_respects_max_order() {
        let params = StrategyModeParams {
            sizing_mode: "position_value_ratio".into(),
            fixed_amount_usd: None,
            proportion: None,
            multiplier: None,
            strategy_allocated_capital_usd: d("2000"),
            source_trader_equity_usd: None,
            source_equity_status: None,
            source_open_positions_value_usd: Some(d("4000")),
        };
        let limits = BuyNotionalLimits {
            max_order_usd: d("80"),
            max_position_usd: d("200"),
            min_order_usd: d("1"),
            strategy_remaining_usd: d("2000"),
            available_balance_usd: d("2000"),
            allow_subminimum_paper_trades: false,
        };
        assert_eq!(capped_copy_notional(&params, d("200"), &limits).unwrap(), d("80"));
    }

    #[test]
    fn paper_pvr_allows_sub_dollar_notional() {
        let params = StrategyModeParams {
            sizing_mode: "position_value_ratio".into(),
            fixed_amount_usd: None,
            proportion: None,
            multiplier: None,
            strategy_allocated_capital_usd: d("2000"),
            source_trader_equity_usd: None,
            source_equity_status: None,
            source_open_positions_value_usd: Some(d("500000")),
        };
        let limits = BuyNotionalLimits {
            max_order_usd: d("50"),
            max_position_usd: d("200"),
            min_order_usd: Decimal::ZERO,
            strategy_remaining_usd: d("2000"),
            available_balance_usd: d("2000"),
            allow_subminimum_paper_trades: true,
        };
        let capped = capped_copy_notional(&params, d("75"), &limits).unwrap();
        assert!(capped < Decimal::ONE);
        assert!(capped > Decimal::ZERO);
    }

    #[test]
    fn paper_pvr_rejects_sub_dollar_when_subminimum_disabled() {
        let params = StrategyModeParams {
            sizing_mode: "position_value_ratio".into(),
            fixed_amount_usd: None,
            proportion: None,
            multiplier: None,
            strategy_allocated_capital_usd: d("2000"),
            source_trader_equity_usd: None,
            source_equity_status: None,
            source_open_positions_value_usd: Some(d("500000")),
        };
        let limits = BuyNotionalLimits {
            max_order_usd: d("50"),
            max_position_usd: d("200"),
            min_order_usd: Decimal::ZERO,
            strategy_remaining_usd: d("2000"),
            available_balance_usd: d("2000"),
            allow_subminimum_paper_trades: false,
        };
        assert_eq!(
            capped_copy_notional(&params, d("75"), &limits),
            Err(RejectReason::BelowMinimum)
        );
    }
}
