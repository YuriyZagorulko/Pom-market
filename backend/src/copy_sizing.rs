//! Single path for copy BUY notional sizing (pipeline + preview).
//!
//! ## Position Value Ratio valuation basis
//! Uses `source_trader_equity_snapshots.positions_value_usd` — aggregate **open-position market value**
//! from Polymarket Data API `/v2/value` (MTM). Does not use wallet USDC or `total_equity_usd`.
//! Optional `multiplier` on the strategy scales the PVR result (`NULL` = 1×).

use rust_decimal::{Decimal, RoundingStrategy};

use crate::display_format::{fmt_coef, fmt_usd};
use crate::engine::{
    small_buy_override_applies, small_buy_trade_multiplier, target_notional, RejectReason, SizingInput,
    SizingMode, EXCHANGE_MIN_NOTIONAL,
};
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
    pub copy_small_buys_1to1: bool,
    pub small_buy_threshold_usd: Option<Decimal>,
}

const SMALL_BUY_THRESHOLD_MAX: Decimal = Decimal::from_parts(10_000_000, 0, 0, false, 0);

/// Validates optional threshold when enabling copy-small-buys (API create/patch).
pub fn parse_small_buy_threshold(value: Option<Decimal>) -> Result<Option<Decimal>, &'static str> {
    let Some(t) = value else {
        return Ok(None);
    };
    if t <= Decimal::ZERO {
        return Err("small buy threshold must be greater than zero");
    }
    if t > SMALL_BUY_THRESHOLD_MAX {
        return Err("small buy threshold must be at most 10000000 USD");
    }
    Ok(Some(t.round_dp(8)))
}

pub fn small_buy_1to1_applies(params: &StrategyModeParams, source_notional: Decimal) -> bool {
    small_buy_override_applies(
        params.copy_small_buys_1to1,
        params.small_buy_threshold_usd,
        source_notional,
    )
}

fn small_buy_trade_multiplier_params(params: &StrategyModeParams) -> Decimal {
    let mode = parse_mode(&params.sizing_mode);
    let input = SizingInput {
        mode,
        fixed_amount_usd: params.fixed_amount_usd,
        proportion: params.proportion,
        multiplier: params.multiplier,
        source_notional: Decimal::ONE,
        max_order_usd: Decimal::ONE,
        min_order_usd: Decimal::ZERO,
        strategy_remaining_usd: Decimal::ONE,
        available_balance_usd: Decimal::ONE,
        strategy_allocated_capital_usd: None,
        source_trader_equity_usd: None,
        source_open_positions_value_usd: None,
        allow_subminimum_paper_trades: false,
        copy_small_buys_1to1: false,
        small_buy_threshold_usd: None,
    };
    small_buy_trade_multiplier(&input)
}

/// Audit fields when a BUY is rejected due to market min shares or USD floor.
pub fn buy_minimum_rejection_audit(
    limit_price: Decimal,
    computed_shares: Decimal,
    market_min_shares: Decimal,
    below_min_shares: bool,
    below_min_notional: bool,
) -> serde_json::Value {
    let minimum_required_notional = if market_min_shares > Decimal::ZERO && limit_price > Decimal::ZERO
    {
        Some((market_min_shares * limit_price).round_dp(2))
    } else {
        None
    };
    let detail = if below_min_shares {
        "below_market_min_shares"
    } else if below_min_notional {
        "below_exchange_min_notional"
    } else {
        "below_minimum"
    };
    serde_json::json!({
        "below_minimum_detail": detail,
        "copy_limit_price": limit_price,
        "computed_shares": computed_shares.to_string(),
        "market_min_order_size": market_min_shares.to_string(),
        "minimum_required_notional": minimum_required_notional.map(|n| n.to_string()),
        "exchange_min_notional_usd": EXCHANGE_MIN_NOTIONAL,
    })
}

/// Trade multiplier for Equal Copy (`proportional`) and legacy `multiplier` sizing modes.
pub fn equal_copy_coefficient(params: &StrategyModeParams) -> Option<Decimal> {
    match parse_mode(&params.sizing_mode) {
        SizingMode::Proportional => params
            .proportion
            .or(params.multiplier)
            .filter(|c| *c > Decimal::ZERO),
        SizingMode::Multiplier => params.multiplier.filter(|c| *c > Decimal::ZERO),
        _ => None,
    }
}

pub fn uses_equal_copy_sell_notional(sizing_mode: &str) -> bool {
    matches!(sizing_mode, "proportional" | "multiplier")
}

pub fn sizing_rule_label(params: &StrategyModeParams, source_notional: Decimal) -> &'static str {
    if small_buy_1to1_applies(params, source_notional) {
        "small_buy_override"
    } else {
        match parse_mode(&params.sizing_mode) {
            SizingMode::PositionValueRatio => "pvr",
            SizingMode::PortfolioRatio => "portfolio_ratio",
            SizingMode::Proportional | SizingMode::Multiplier => "equal_copy",
            SizingMode::FixedAmount => "fixed_amount",
        }
    }
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
    if small_buy_1to1_applies(params, source_notional) {
        return Ok(
            (source_notional * small_buy_trade_multiplier_params(params)).round_dp(8),
        );
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
        let base = source_notional * params.strategy_allocated_capital_usd / positions;
        return Ok((base * pvr_trade_multiplier(params.multiplier)).round_dp(8));
    }
    match mode {
        SizingMode::FixedAmount => params.fixed_amount_usd.ok_or(RejectReason::MissingSizing),
        SizingMode::Proportional => {
            let coef = equal_copy_coefficient(params).ok_or(RejectReason::MissingSizing)?;
            Ok((source_notional * coef).round_dp(8))
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
        copy_small_buys_1to1: params.copy_small_buys_1to1,
        small_buy_threshold_usd: params.small_buy_threshold_usd,
    };
    target_notional(&sizing)
}

/// Cap Equal Copy SELL notional (no cash/budget caps — shares limited separately).
pub fn cap_equal_copy_sell_notional(
    raw: Decimal,
    max_order_usd: Decimal,
    min_order_usd: Decimal,
    allow_subminimum_paper_trades: bool,
) -> Result<Decimal, RejectReason> {
    if raw <= Decimal::ZERO {
        return Err(RejectReason::BelowMinimum);
    }
    let capped = raw.min(max_order_usd).round_dp_with_strategy(2, RoundingStrategy::ToZero);
    let floor = if allow_subminimum_paper_trades {
        min_order_usd.max(Decimal::ZERO)
    } else {
        min_order_usd.max(EXCHANGE_MIN_NOTIONAL)
    };
    if floor > Decimal::ZERO && capped < floor {
        return Err(RejectReason::BelowMinimum);
    }
    Ok(capped)
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
/// Human-readable sizing line for Activity (does not invent numbers; uses audit JSON).
pub fn explain_sizing_audit(audit: &serde_json::Value) -> Option<String> {
    let mode = audit.get("sizing_mode")?.as_str()?;
    let source = audit
        .get("source_notional_usd")
        .and_then(json_decimal)
        .filter(|d| *d > Decimal::ZERO)?;
    let raw = audit
        .get("raw_target_notional_usd")
        .and_then(json_decimal);
    let capped = audit
        .get("capped_target_notional_usd")
        .and_then(json_decimal);
    let final_target = capped.or(raw);
    let cap_reason = audit
        .get("cap_reason")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let reject = audit
        .get("rejection_reason")
        .and_then(|v| v.as_str());

    let mut lines = Vec::new();
    let sizing_rule = audit.get("sizing_rule").and_then(|v| v.as_str());
    let small_buy_rule = matches!(
        sizing_rule,
        Some("small_buy_override") | Some("pvr_small_buy_override") | Some("small_buy_1to1")
    );
    if small_buy_rule {
        let thresh = audit
            .get("small_buy_threshold_usd")
            .and_then(json_decimal)
            .filter(|t| *t > Decimal::ZERO)?;
        let mult = audit
            .get("pvr_trade_multiplier")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<Decimal>().ok())
            .filter(|m| *m > Decimal::ZERO)
            .unwrap_or(Decimal::ONE);
        let raw_disp = raw.unwrap_or(source * mult);
        let capped_disp = capped.unwrap_or(raw_disp);
        let mut line = format!(
            "Small BUY Override: leader {} × {} = {} (threshold {}).",
            fmt_usd(source),
            fmt_coef(mult),
            fmt_usd(raw_disp),
            fmt_usd(thresh),
        );
        if capped_disp + Decimal::new(1, 2) < raw_disp {
            line.push_str(&format!(
                " Order target after limits: {}.",
                fmt_usd(capped_disp)
            ));
        }
        if let Some(cr) = cap_reason {
            line.push_str(&format!(" Capped: {}.", cr));
        }
        lines.push(line);
    } else {
    match mode {
        "position_value_ratio" => {
            let alloc = audit
                .get("strategy_allocated_capital_usd")
                .and_then(json_decimal)?;
            let open = audit
                .get("open_positions_value_usd")
                .and_then(json_decimal)
                .filter(|v| *v > Decimal::ZERO)?;
            let mult = audit
                .get("pvr_trade_multiplier")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<Decimal>().ok())
                .filter(|m| *m > Decimal::ZERO)
                .unwrap_or(Decimal::ONE);
            let base = (source * alloc / open).round_dp(4);
            let raw_disp = raw.unwrap_or(base * mult);
            lines.push(format!(
                "PVR: leader {} × ({} ÷ {} open position value) × {} (PVR mult) ≈ {}",
                fmt_usd(source),
                fmt_usd(alloc),
                fmt_usd(open),
                fmt_coef(mult),
                fmt_usd(raw_disp),
            ));
        }
        "proportional" | "multiplier" => {
            let coef = audit
                .get("proportion_coefficient")
                .and_then(json_decimal)
                .or_else(|| audit.get("pvr_trade_multiplier").and_then(json_decimal))
                .filter(|c| *c > Decimal::ZERO)?;
            let raw_disp = raw.unwrap_or(source * coef);
            let mut line = format!(
                "Equal Copy: leader {} × {} = {}",
                fmt_usd(source),
                fmt_coef(coef),
                fmt_usd(raw_disp),
            );
            lines.push(line);
        }
        "fixed_amount" => {
            let fixed = audit
                .get("fixed_amount_usd")
                .and_then(json_decimal)
                .or(raw)?;
            lines.push(format!("Fixed USD: {} per trade", fmt_usd(fixed)));
        }
        _ => return None,
    }
        if let Some(capped) = final_target {
            if let Some(raw) = raw {
                if capped + Decimal::new(1, 2) < raw {
                    lines.push(format!(
                        "Capped from {} to {}",
                        fmt_usd(raw),
                        fmt_usd(capped),
                    ));
                    if let Some(r) = cap_reason {
                        lines.push(format!("Cap: {r}"));
                    }
                }
            }
            lines.push(format!("Order target: {}", fmt_usd(capped)));
        }
    }
    if let Some(r) = reject {
        lines.push(format!("Rejected: {r}"));
    }
    Some(lines.join(". "))
}

fn json_decimal(value: &serde_json::Value) -> Option<Decimal> {
    match value {
        serde_json::Value::String(s) => s.parse().ok(),
        serde_json::Value::Number(n) => n.to_string().parse().ok(),
        _ => None,
    }
}

pub fn compute_cap_reason(
    raw: Decimal,
    capped: Decimal,
    limits: &BuyNotionalLimits,
) -> Option<&'static str> {
    if capped >= raw - Decimal::new(1, 2) {
        return None;
    }
    let candidates = [
        (limits.max_order_usd, "max_order_usd"),
        (limits.strategy_remaining_usd, "remaining_strategy_budget"),
        (limits.available_balance_usd, "available_cash"),
    ];
    for (lim, name) in candidates {
        if capped == raw.min(lim).round_dp_with_strategy(2, RoundingStrategy::ToZero) {
            return Some(name);
        }
    }
    Some("risk_limits")
}

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
        copy_small_buys_1to1: false,
        small_buy_threshold_usd: None,
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
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
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
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
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
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
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
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
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
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
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
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
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
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
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
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
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
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
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
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
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
    fn proportional_multiplier_examples() {
        let make = |coef: &str| StrategyModeParams {
            sizing_mode: "proportional".into(),
            fixed_amount_usd: None,
            proportion: Some(d(coef)),
            multiplier: None,
            strategy_allocated_capital_usd: d("2000"),
            source_trader_equity_usd: None,
            source_equity_status: None,
            source_open_positions_value_usd: None,
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
        };
        let src = d("11.70");
        assert_eq!(raw_copy_notional(&make("0.5"), src).unwrap(), d("5.85"));
        assert_eq!(raw_copy_notional(&make("1"), src).unwrap(), d("11.70"));
        assert_eq!(raw_copy_notional(&make("2"), src).unwrap(), d("23.40"));
        assert_eq!(raw_copy_notional(&make("10"), src).unwrap(), d("117"));
    }

    #[test]
    fn explain_pvr_does_not_imply_proportional_times_ten() {
        let audit = serde_json::json!({
            "sizing_mode": "position_value_ratio",
            "source_notional_usd": "11.70",
            "strategy_allocated_capital_usd": "2000",
            "open_positions_value_usd": "480018.96",
            "pvr_trade_multiplier": "10",
            "raw_target_notional_usd": "0.24",
            "rejection_reason": "below_minimum"
        });
        let s = explain_sizing_audit(&audit).unwrap();
        assert!(s.contains("PVR:"));
        assert!(s.contains("open position value"));
        assert!(!s.starts_with("Leader"));
        assert!(!s.contains("10.00000000"));
        assert!(s.contains("× 10"));
    }

    #[test]
    fn compute_cap_reason_identifies_budget_and_cash() {
        let limits = BuyNotionalLimits {
            max_order_usd: d("5000"),
            max_position_usd: d("500"),
            min_order_usd: d("1"),
            strategy_remaining_usd: d("25"),
            available_balance_usd: d("100"),
            allow_subminimum_paper_trades: false,
        };
        let raw = d("117");
        assert_eq!(
            compute_cap_reason(raw, d("25"), &limits),
            Some("remaining_strategy_budget")
        );
        let limits_cash = BuyNotionalLimits {
            strategy_remaining_usd: d("500"),
            available_balance_usd: d("40"),
            ..limits
        };
        assert_eq!(
            compute_cap_reason(raw, d("40"), &limits_cash),
            Some("available_cash")
        );
    }

    #[test]
    fn explain_proportional_small_buy_override() {
        let audit = serde_json::json!({
            "sizing_mode": "proportional",
            "sizing_rule": "small_buy_override",
            "source_notional_usd": "4",
            "proportion_coefficient": "0.1",
            "small_buy_threshold_usd": "5",
            "pvr_trade_multiplier": "1",
            "raw_target_notional_usd": "4",
        });
        let s = explain_sizing_audit(&audit).unwrap();
        assert!(s.contains("Small BUY Override"));
        assert!(s.contains("4"));
    }

    #[test]
    fn explain_proportional_times_ten() {
        let audit = serde_json::json!({
            "sizing_mode": "proportional",
            "source_notional_usd": "11.70",
            "proportion_coefficient": "10",
            "raw_target_notional_usd": "117",
        });
        let s = explain_sizing_audit(&audit).unwrap();
        assert!(s.contains("Equal Copy"));
        assert!(s.contains("× 10"));
        assert!(s.contains("117"));
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
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
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

    fn pvr_params(small_buy: bool, thresh: Option<&str>) -> StrategyModeParams {
        StrategyModeParams {
            sizing_mode: "position_value_ratio".into(),
            fixed_amount_usd: None,
            proportion: None,
            multiplier: None,
            strategy_allocated_capital_usd: d("2000"),
            source_trader_equity_usd: None,
            source_equity_status: None,
            source_open_positions_value_usd: Some(d("4000")),
            copy_small_buys_1to1: small_buy,
            small_buy_threshold_usd: thresh.map(d),
        }
    }

    fn std_limits() -> BuyNotionalLimits {
        BuyNotionalLimits {
            max_order_usd: d("5000"),
            max_position_usd: d("500"),
            min_order_usd: d("1"),
            strategy_remaining_usd: d("2000"),
            available_balance_usd: d("2000"),
            allow_subminimum_paper_trades: false,
        }
    }

    #[test]
    fn small_buy_1to1_examples() {
        let on = pvr_params(true, Some("5"));
        assert_eq!(raw_copy_notional(&on, d("3")).unwrap(), d("3"));
        assert_eq!(raw_copy_notional(&on, d("4.99")).unwrap(), d("4.99"));
        assert_eq!(raw_copy_notional(&on, d("5")).unwrap(), d("5"));
        assert_eq!(raw_copy_notional(&on, d("10")).unwrap(), d("5"));
        let mut on_half = pvr_params(true, Some("5"));
        on_half.multiplier = Some(d("0.5"));
        assert_eq!(raw_copy_notional(&on_half, d("4")).unwrap(), d("2"));
        let mut on_double = pvr_params(true, Some("5"));
        on_double.multiplier = Some(d("2"));
        assert_eq!(raw_copy_notional(&on_double, d("3")).unwrap(), d("6"));
        let off = pvr_params(false, Some("5"));
        assert_eq!(raw_copy_notional(&off, d("3")).unwrap(), d("1.5"));
    }

    #[test]
    fn small_buy_applies_for_proportional_not_fixed_above_threshold() {
        let mut prop = StrategyModeParams {
            sizing_mode: "proportional".into(),
            fixed_amount_usd: None,
            proportion: Some(d("0.1")),
            multiplier: None,
            strategy_allocated_capital_usd: d("2000"),
            source_trader_equity_usd: None,
            source_equity_status: None,
            source_open_positions_value_usd: None,
            copy_small_buys_1to1: true,
            small_buy_threshold_usd: Some(d("5")),
        };
        assert_eq!(raw_copy_notional(&prop, d("4")).unwrap(), d("0.4"));
        assert_eq!(
            sizing_rule_label(&prop, d("4")),
            "small_buy_override"
        );
        assert_eq!(raw_copy_notional(&prop, d("6")).unwrap(), d("0.6"));
        prop.copy_small_buys_1to1 = false;
        assert_eq!(raw_copy_notional(&prop, d("4")).unwrap(), d("0.4"));
        assert!(!small_buy_1to1_applies(&prop, d("4")));
        let fixed = StrategyModeParams {
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
        assert_eq!(raw_copy_notional(&fixed, d("3")).unwrap(), d("3"));
        assert_eq!(raw_copy_notional(&fixed, d("6")).unwrap(), d("25"));
    }

    #[test]
    fn small_buy_1to1_respects_caps() {
        let params = pvr_params(true, Some("5"));
        let mut limits = std_limits();
        limits.available_balance_usd = d("2");
        assert_eq!(capped_copy_notional(&params, d("3"), &limits).unwrap(), d("2"));
        limits.available_balance_usd = d("2000");
        limits.strategy_remaining_usd = d("2");
        assert_eq!(capped_copy_notional(&params, d("3"), &limits).unwrap(), d("2"));
        limits.strategy_remaining_usd = d("2000");
        limits.max_order_usd = d("2");
        assert_eq!(capped_copy_notional(&params, d("3"), &limits).unwrap(), d("2"));
    }

    #[test]
    fn small_buy_at_exchange_minimum_still_rejects() {
        let params = pvr_params(true, Some("5"));
        let limits = std_limits();
        assert_eq!(
            capped_copy_notional(&params, d("0.50"), &limits),
            Err(RejectReason::BelowMinimum)
        );
    }

    #[test]
    fn explain_small_buy_1to1_audit() {
        let audit = serde_json::json!({
            "sizing_mode": "position_value_ratio",
            "sizing_rule": "small_buy_override",
            "source_notional_usd": "4.50",
            "small_buy_threshold_usd": "5",
            "pvr_trade_multiplier": "0.5",
            "raw_target_notional_usd": "2.25",
            "capped_target_notional_usd": "2.25",
        });
        let s = explain_sizing_audit(&audit).unwrap();
        assert!(s.contains("Small BUY Override"));
        assert!(s.contains("× 0.5"));
        assert!(s.contains("threshold"));
    }

    #[test]
    fn equal_copy_buy_examples() {
        let make = |coef: &str| StrategyModeParams {
            sizing_mode: "proportional".into(),
            fixed_amount_usd: None,
            proportion: Some(d(coef)),
            multiplier: None,
            strategy_allocated_capital_usd: d("2000"),
            source_trader_equity_usd: None,
            source_equity_status: None,
            source_open_positions_value_usd: None,
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
        };
        let src = d("10");
        assert_eq!(raw_copy_notional(&make("1"), src).unwrap(), d("10"));
        assert_eq!(raw_copy_notional(&make("0.5"), src).unwrap(), d("5"));
        assert_eq!(raw_copy_notional(&make("0.25"), src).unwrap(), d("2.5"));
        assert_eq!(raw_copy_notional(&make("2"), src).unwrap(), d("20"));
        assert_eq!(sizing_rule_label(&make("1"), src), "equal_copy");
    }

    #[test]
    fn equal_copy_sell_cap_max_order() {
        let raw = d("20");
        assert_eq!(
            cap_equal_copy_sell_notional(raw, d("10"), d("1"), false).unwrap(),
            d("10")
        );
    }

    #[test]
    fn parse_small_buy_threshold_rejects_non_positive() {
        assert!(parse_small_buy_threshold(Some(d("0"))).is_err());
        assert!(parse_small_buy_threshold(Some(d("-1"))).is_err());
        assert!(parse_small_buy_threshold(Some(d("5"))).is_ok());
    }

    #[test]
    fn small_buy_threshold_boundary_inclusive() {
        let on = pvr_params(true, Some("5"));
        assert!(small_buy_1to1_applies(&on, d("5")));
        assert!(small_buy_1to1_applies(&on, d("4.99")));
        assert!(!small_buy_1to1_applies(&on, d("5.01")));
        assert_eq!(raw_copy_notional(&on, d("5.01")).unwrap(), d("2.505"));
    }

    #[test]
    fn small_buy_threshold_matrix_all_modes() {
        let prop = StrategyModeParams {
            sizing_mode: "proportional".into(),
            fixed_amount_usd: None,
            proportion: Some(d("0.5")),
            multiplier: None,
            strategy_allocated_capital_usd: d("2000"),
            source_trader_equity_usd: None,
            source_equity_status: None,
            source_open_positions_value_usd: None,
            copy_small_buys_1to1: true,
            small_buy_threshold_usd: Some(d("5")),
        };
        assert_eq!(raw_copy_notional(&prop, d("4.99")).unwrap(), d("2.495"));
        assert!(!small_buy_1to1_applies(&prop, d("5.01")));
        let pvr = pvr_params(true, Some("5"));
        assert!(small_buy_1to1_applies(&pvr, d("5")));
        assert!(!small_buy_1to1_applies(&pvr, d("5.01")));
    }

    #[test]
    fn buy_minimum_rejection_audit_shape() {
        let v = buy_minimum_rejection_audit(d("0.63"), d("2.38"), d("5"), true, false);
        assert_eq!(v["below_minimum_detail"], "below_market_min_shares");
        assert_eq!(v["minimum_required_notional"], "3.15");
    }

    #[test]
    fn small_buy_requires_positive_threshold_in_db_shape() {
        let mut on = pvr_params(true, Some("5"));
        on.small_buy_threshold_usd = None;
        assert!(!small_buy_1to1_applies(&on, d("3")));
        on.small_buy_threshold_usd = Some(d("0"));
        assert!(!small_buy_1to1_applies(&on, d("3")));
    }

    #[test]
    fn small_buy_large_leader_notional_uses_pvr_when_above_threshold() {
        let on = pvr_params(true, Some("5"));
        assert_eq!(raw_copy_notional(&on, d("100000")).unwrap(), d("50000"));
    }

    #[test]
    fn accumulated_small_buys_each_capped_by_remaining() {
        let params = pvr_params(true, Some("10"));
        let mut limits = std_limits();
        limits.strategy_remaining_usd = d("7");
        assert_eq!(capped_copy_notional(&params, d("3"), &limits).unwrap(), d("3"));
        limits.strategy_remaining_usd = d("2");
        assert_eq!(capped_copy_notional(&params, d("4"), &limits).unwrap(), d("2"));
        limits.strategy_remaining_usd = d("0.5");
        assert_eq!(
            capped_copy_notional(&params, d("0.4"), &limits),
            Err(RejectReason::StrategyBudget)
        );
    }
}
