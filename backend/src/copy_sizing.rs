//! Single path for copy BUY notional sizing (pipeline + preview).

use rust_decimal::Decimal;

use crate::engine::{target_notional, RejectReason, SizingInput, SizingMode};
use crate::source_equity::EquityStatus;

#[derive(Clone, Debug)]
pub struct BuyNotionalLimits {
    pub max_order_usd: Decimal,
    pub max_position_usd: Decimal,
    pub min_order_usd: Decimal,
    pub strategy_remaining_usd: Decimal,
    pub account_exposure_remaining_usd: Decimal,
    pub available_balance_usd: Decimal,
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
}

/// Raw copy notional before risk caps (same formula pipeline uses).
pub fn raw_copy_notional(
    params: &StrategyModeParams,
    source_notional: Decimal,
) -> Result<Decimal, RejectReason> {
    let mode = parse_mode(&params.sizing_mode);
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
        if params.strategy_allocated_capital_usd <= Decimal::ZERO {
            return Err(RejectReason::MissingSizing);
        }
        if source_notional <= Decimal::ZERO {
            return Err(RejectReason::BelowMinimum);
        }
        return Ok((source_notional * params.strategy_allocated_capital_usd / equity).round_dp(8));
    }
    match mode {
        SizingMode::FixedAmount => params.fixed_amount_usd.ok_or(RejectReason::MissingSizing),
        SizingMode::Proportional => {
            Ok(source_notional * params.proportion.ok_or(RejectReason::MissingSizing)?)
        }
        SizingMode::Multiplier => {
            Ok(source_notional * params.multiplier.ok_or(RejectReason::MissingSizing)?)
        }
        SizingMode::PortfolioRatio => Err(RejectReason::SourceEquityUnavailable),
    }
}

pub fn capped_copy_notional(
    params: &StrategyModeParams,
    source_notional: Decimal,
    limits: &BuyNotionalLimits,
) -> Result<Decimal, RejectReason> {
    let _raw = raw_copy_notional(params, source_notional)?;
    let sizing = SizingInput {
        mode: parse_mode(&params.sizing_mode),
        fixed_amount_usd: params.fixed_amount_usd,
        proportion: params.proportion,
        multiplier: params.multiplier,
        source_notional,
        max_order_usd: limits.max_order_usd.min(limits.max_position_usd),
        min_order_usd: limits.min_order_usd,
        strategy_remaining_usd: limits.strategy_remaining_usd,
        account_exposure_remaining_usd: limits.account_exposure_remaining_usd,
        available_balance_usd: limits.available_balance_usd,
        strategy_allocated_capital_usd: if params.sizing_mode == "portfolio_ratio" {
            Some(params.strategy_allocated_capital_usd)
        } else {
            None
        },
        source_trader_equity_usd: params.source_trader_equity_usd,
    };
    target_notional(&sizing)
}

pub fn parse_mode(value: &str) -> SizingMode {
    match value {
        "proportional" => SizingMode::Proportional,
        "multiplier" => SizingMode::Multiplier,
        "portfolio_ratio" => SizingMode::PortfolioRatio,
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

pub fn source_trade_percent(source_notional: Decimal, source_equity: Decimal) -> Option<Decimal> {
    if source_equity > Decimal::ZERO && source_notional > Decimal::ZERO {
        Some((source_notional / source_equity * Decimal::from(100)).round_dp(4))
    } else {
        None
    }
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
        };
        assert_eq!(raw_copy_notional(&params, d("4000")).unwrap(), d("400"));
    }

    #[test]
    fn unavailable_equity_blocks_portfolio() {
        let params = StrategyModeParams {
            sizing_mode: "portfolio_ratio".into(),
            fixed_amount_usd: None,
            proportion: None,
            multiplier: None,
            strategy_allocated_capital_usd: d("2000"),
            source_trader_equity_usd: Some(d("20000")),
            source_equity_status: Some(EquityStatus::Unavailable),
        };
        assert_eq!(
            raw_copy_notional(&params, d("100")),
            Err(RejectReason::SourceEquityUnavailable)
        );
    }
}
