use rust_decimal::prelude::*;
use rust_decimal::Decimal;
use serde::Serialize;

pub const EXCHANGE_MIN_NOTIONAL: Decimal = Decimal::from_parts(1, 0, 0, false, 0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizingMode {
    FixedAmount,
    Proportional,
    Multiplier,
    PortfolioRatio,
}

#[derive(Clone, Debug)]
pub struct SizingInput {
    pub mode: SizingMode,
    pub fixed_amount_usd: Option<Decimal>,
    pub proportion: Option<Decimal>,
    pub multiplier: Option<Decimal>,
    pub source_notional: Decimal,
    pub max_order_usd: Decimal,
    pub min_order_usd: Decimal,
    pub strategy_remaining_usd: Decimal,
    pub account_exposure_remaining_usd: Decimal,
    pub available_balance_usd: Decimal,
    /// Strategy capital allocated to this copy strategy (not whole account balance).
    pub strategy_allocated_capital_usd: Option<Decimal>,
    /// Observed source trader total equity for portfolio-relative sizing.
    pub source_trader_equity_usd: Option<Decimal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RejectReason {
    MissingSizing,
    BelowMinimum,
    InsufficientBalance,
    StrategyBudget,
    ExposureCap,
    Slippage,
    NoLiquidity,
    NoPosition,
    SellLargerThanOpen,
    CopySideDisabled,
    MonitoringMode,
    StrategyDisabled,
    StaleData,
    MarketNotAllowed,
    DailyLoss,
    RateLimited,
    KillSwitch,
    EmergencyStop,
    SoftCloseBuyBlocked,
    HardCloseInProgress,
    AccountNoBuy,
    LiveNotConfirmed,
    Cooldown,
    MaxPositions,
    Duplicate,
    CredentialsMissing,
    ExchangeUnknown,
    FeeUnknown,
    SourceEquityUnavailable,
    StrategyCapitalNotInitialized,
}

impl RejectReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingSizing => "missing_sizing",
            Self::BelowMinimum => "below_minimum",
            Self::InsufficientBalance => "insufficient_balance",
            Self::StrategyBudget => "strategy_budget",
            Self::ExposureCap => "exposure_capped",
            Self::Slippage => "slippage_exceeded",
            Self::NoLiquidity => "no_liquidity",
            Self::NoPosition => "no_position",
            Self::SellLargerThanOpen => "sell_exceeds_open_shares",
            Self::CopySideDisabled => "copy_side_disabled",
            Self::MonitoringMode => "monitoring_mode",
            Self::StrategyDisabled => "strategy_disabled",
            Self::StaleData => "stale_data",
            Self::MarketNotAllowed => "market_not_allowed",
            Self::DailyLoss => "daily_loss_stop",
            Self::RateLimited => "rate_limited",
            Self::KillSwitch => "kill_switch",
            Self::EmergencyStop => "emergency_stop",
            Self::SoftCloseBuyBlocked => "soft_close_no_buy",
            Self::HardCloseInProgress => "hard_close_in_progress",
            Self::AccountNoBuy => "account_no_buy",
            Self::LiveNotConfirmed => "live_not_confirmed",
            Self::Cooldown => "cooldown",
            Self::MaxPositions => "max_open_positions",
            Self::Duplicate => "duplicate",
            Self::CredentialsMissing => "credentials_missing",
            Self::ExchangeUnknown => "exchange_unknown",
            Self::FeeUnknown => "fee_unknown",
            Self::SourceEquityUnavailable => "source_equity_unavailable",
            Self::StrategyCapitalNotInitialized => "strategy_capital_not_initialized",
        }
    }
}

/// User-facing title and detail for API / activity views.
pub fn decision_reason_explain(code: &str) -> Option<(&'static str, &'static str)> {
    Some(match code {
        "strategy_capital_not_initialized" => (
            "Strategy capital not funded",
            "No cash is reserved in this strategy's ledger yet. Enable the strategy or allocate paper capital from the trading account.",
        ),
        "strategy_disabled" => ("Strategy disabled", "Turn on the strategy to copy new trades."),
        "source_equity_unavailable" => (
            "Source portfolio equity unknown",
            "Portfolio-relative sizing needs a fresh equity snapshot for the leader wallet.",
        ),
        "below_minimum" => (
            "Below minimum order size",
            "The computed copy size is below exchange or strategy minimums after caps.",
        ),
        "strategy_budget" => (
            "Strategy budget exhausted",
            "Remaining strategy allocation cannot cover this copy size.",
        ),
        "insufficient_balance" => (
            "Insufficient balance",
            "Available strategy cash or account balance is too low for this order.",
        ),
        "no_liquidity" => (
            "No liquidity",
            "The order book could not fill the target size within the limit price.",
        ),
        "slippage_exceeded" => (
            "Slippage limit",
            "Simulated fill slippage exceeds the strategy maximum.",
        ),
        "copy_side_disabled" => (
            "Copy side disabled",
            "This strategy is not configured to copy that side (buy or sell).",
        ),
        "no_position" => (
            "No position to sell",
            "The strategy has no open shares matching this exit.",
        ),
        _ => return None,
    })
}

pub fn target_notional(input: &SizingInput) -> Result<Decimal, RejectReason> {
    let raw = match input.mode {
        SizingMode::FixedAmount => input.fixed_amount_usd.ok_or(RejectReason::MissingSizing)?,
        SizingMode::Proportional => {
            input.source_notional * input.proportion.ok_or(RejectReason::MissingSizing)?
        }
        SizingMode::Multiplier => {
            input.source_notional * input.multiplier.ok_or(RejectReason::MissingSizing)?
        }
        SizingMode::PortfolioRatio => {
            let equity = input
                .source_trader_equity_usd
                .filter(|v| *v > Decimal::ZERO)
                .ok_or(RejectReason::SourceEquityUnavailable)?;
            let alloc = input
                .strategy_allocated_capital_usd
                .filter(|v| *v > Decimal::ZERO)
                .ok_or(RejectReason::MissingSizing)?;
            if input.source_notional <= Decimal::ZERO {
                return Err(RejectReason::BelowMinimum);
            }
            (input.source_notional * alloc / equity).round_dp(8)
        }
    };
    if raw <= Decimal::ZERO {
        return Err(RejectReason::BelowMinimum);
    }
    if input.available_balance_usd < input.min_order_usd.max(EXCHANGE_MIN_NOTIONAL) {
        return Err(RejectReason::InsufficientBalance);
    }
    if input.strategy_remaining_usd < input.min_order_usd.max(EXCHANGE_MIN_NOTIONAL) {
        return Err(RejectReason::StrategyBudget);
    }
    if input.account_exposure_remaining_usd < input.min_order_usd.max(EXCHANGE_MIN_NOTIONAL) {
        return Err(RejectReason::ExposureCap);
    }
    let capped = raw
        .min(input.max_order_usd)
        .min(input.strategy_remaining_usd)
        .min(input.account_exposure_remaining_usd)
        .min(input.available_balance_usd)
        .round_dp_with_strategy(2, RoundingStrategy::ToZero);
    let floor = input.min_order_usd.max(EXCHANGE_MIN_NOTIONAL);
    if capped < floor {
        if raw > input.available_balance_usd {
            return Err(RejectReason::InsufficientBalance);
        }
        if raw > input.strategy_remaining_usd {
            return Err(RejectReason::StrategyBudget);
        }
        if raw > input.account_exposure_remaining_usd {
            return Err(RejectReason::ExposureCap);
        }
        return Err(RejectReason::BelowMinimum);
    }
    Ok(capped)
}

pub fn limit_price(
    side_buy: bool,
    source_price: Decimal,
    slippage_bps: i32,
    tick: Decimal,
) -> Decimal {
    let bps = Decimal::from(slippage_bps.max(0)) / Decimal::from(10_000);
    let one = Decimal::ONE;
    let raw = if side_buy {
        (source_price * (one + bps)).min(Decimal::new(999, 3))
    } else {
        (source_price * (one - bps)).max(Decimal::new(1, 3))
    };
    if tick <= Decimal::ZERO {
        return raw;
    }
    if side_buy {
        (raw / tick).floor() * tick
    } else {
        (raw / tick).ceil() * tick
    }
}

/// SDK 0.9.0 lot size: share quantities are truncated toward zero at 2 decimal places.
pub const LOT_SIZE_SCALE: u32 = 2;

pub fn truncate_lot(size: Decimal) -> Decimal {
    if size <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    size.round_dp_with_strategy(LOT_SIZE_SCALE, RoundingStrategy::ToZero)
}

/// Collateral and share atoms use 6 decimal places, matching the SDK `to_fixed_u128` path.
pub fn signed_order_amounts(side_buy: bool, price: Decimal, shares: Decimal) -> (u128, u128) {
    let shares = truncate_lot(shares);
    let usdc = (shares * price).trunc_with_scale(8);
    let share_atoms = fixed_6(shares);
    let usdc_atoms = fixed_6(usdc);
    if side_buy {
        (usdc_atoms, share_atoms)
    } else {
        (share_atoms, usdc_atoms)
    }
}

fn fixed_6(value: Decimal) -> u128 {
    value.trunc_with_scale(6).mantissa().unsigned_abs()
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SizeTrace {
    pub requested_notional_usd: Decimal,
    pub source_price: Decimal,
    pub copy_limit_price: Decimal,
    pub market_tick_size: Decimal,
    pub market_min_order_size: Decimal,
    pub raw_requested_shares: Decimal,
    pub shares_from_source_price: Decimal,
    pub budget_capped_shares: Decimal,
    pub liquidity_available_shares: Decimal,
    pub cent_grid_shares: Decimal,
    pub strategy_size_rounding: Decimal,
    pub sdk_normalized_shares: Decimal,
    pub maker_amount: u128,
    pub taker_amount: u128,
    pub fee_reserved_usd: Decimal,
    pub position_reserved_usd: Decimal,
    pub fee_price: Decimal,
    pub spend_cap_usd: Decimal,
    pub fee_unknown: bool,
    pub below_min_order_size: bool,
}

pub fn conservative_fee_price(low: Decimal, high: Decimal) -> Decimal {
    if low <= Decimal::ZERO && high <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let (start, end) = if low <= high {
        (low.max(Decimal::ZERO), high)
    } else {
        (high.max(Decimal::ZERO), low)
    };
    let half = Decimal::new(5, 1);
    if start <= half && half <= end {
        half
    } else if (start - half).abs() <= (end - half).abs() {
        start
    } else {
        end
    }
}

fn collateral_usd(side_buy: bool, price: Decimal, shares: Decimal) -> Decimal {
    let (maker, taker) = signed_order_amounts(side_buy, price, shares);
    let atoms = if side_buy { maker } else { taker };
    Decimal::from_i128_with_scale(i128::try_from(atoms).unwrap_or(i128::MAX), 6)
}

/// Cash that is still free after open-order reserves. Never negative.
pub fn available_after_reserve(cash: Decimal, reserved: Decimal) -> Decimal {
    (cash - reserved).max(Decimal::ZERO)
}

/// Buy reserve is the worse of the SDK collateral and the cent-rounded notional, plus the taker fee.
pub fn order_reserves(
    side_buy: bool,
    limit: Decimal,
    shares: Decimal,
    fee_price: Decimal,
    schedule: Option<&FeeSchedule>,
) -> (Decimal, Decimal) {
    let position = if side_buy {
        let atoms = collateral_usd(true, limit, shares);
        let rounded = (truncate_lot(shares) * limit).round_dp(2);
        atoms.max(rounded)
    } else {
        Decimal::ZERO
    };
    let fee = schedule
        .and_then(|schedule| taker_fee(shares, fee_price, schedule).amount)
        .unwrap_or(Decimal::ZERO);
    (position, fee)
}

/// After a fill, the booked notional and fee live on the position. Only the unfilled shares stay reserved.
/// A missing booked fee keeps the fee reserve so an unknown fee is not treated as zero.
pub fn reserves_after_execution(
    order_shares: Decimal,
    filled_shares: Decimal,
    position_reserved: Decimal,
    fee_reserved: Decimal,
    booked_fee: Option<Decimal>,
) -> (Decimal, Decimal) {
    let remaining = (order_shares - filled_shares).max(Decimal::ZERO);
    if order_shares <= Decimal::ZERO || remaining <= Decimal::ZERO {
        let fee_left = if booked_fee.is_some() {
            Decimal::ZERO
        } else {
            fee_reserved
        };
        return (Decimal::ZERO, fee_left);
    }
    let ratio = remaining / order_shares;
    let position_left = (position_reserved * ratio).round_dp(8);
    let fee_left = if booked_fee.is_some() {
        (fee_reserved * ratio).round_dp(8)
    } else {
        fee_reserved
    };
    (position_left, fee_left)
}

/// Replace an estimated fee with the exchange fee by the difference, so the estimate is not charged again.
pub fn fee_reconciliation_delta(booked_estimate: Decimal, actual: Decimal) -> Decimal {
    actual - booked_estimate
}

/// `spend_cap` is the cash that notional plus the fee reserve must fit in.
/// Shares stay on the 2-decimal SDK lot and step down by 0.01 when the fee does not fit.
#[allow(clippy::too_many_arguments)] // sizing inputs are easier to audit when separate
pub fn trace_buy_size(
    requested: Decimal,
    source_price: Decimal,
    limit: Decimal,
    tick: Decimal,
    min_order_size: Decimal,
    available_shares: Decimal,
    spend_cap: Decimal,
    schedule: Option<&FeeSchedule>,
    fee_price: Decimal,
) -> SizeTrace {
    let raw = if limit > Decimal::ZERO {
        requested / limit
    } else {
        Decimal::ZERO
    };
    let from_source = if source_price > Decimal::ZERO {
        requested / source_price
    } else {
        Decimal::ZERO
    };
    let liquidity = raw.min(available_shares.max(Decimal::ZERO));
    let fee_unknown = schedule.is_none();
    let mut shares = if fee_unknown {
        Decimal::ZERO
    } else {
        truncate_lot(liquidity)
    };
    let mut position = Decimal::ZERO;
    let mut fee_reserved = Decimal::ZERO;
    if let Some(schedule) = schedule {
        while shares > Decimal::ZERO {
            (position, fee_reserved) =
                order_reserves(true, limit, shares, fee_price, Some(schedule));
            if position + fee_reserved <= spend_cap {
                break;
            }
            shares -= Decimal::new(1, 2);
            position = Decimal::ZERO;
            fee_reserved = Decimal::ZERO;
        }
    }
    if shares < Decimal::ZERO {
        shares = Decimal::ZERO;
    }
    let (maker, taker) = signed_order_amounts(true, limit, shares);
    SizeTrace {
        requested_notional_usd: requested,
        source_price,
        copy_limit_price: limit,
        market_tick_size: tick,
        market_min_order_size: min_order_size,
        raw_requested_shares: raw,
        shares_from_source_price: from_source,
        budget_capped_shares: liquidity,
        liquidity_available_shares: liquidity,
        cent_grid_shares: shares_on_cent_grid(from_source, limit),
        strategy_size_rounding: shares,
        sdk_normalized_shares: shares,
        maker_amount: maker,
        taker_amount: taker,
        fee_reserved_usd: fee_reserved,
        position_reserved_usd: position,
        fee_price,
        spend_cap_usd: spend_cap,
        fee_unknown,
        below_min_order_size: min_order_size > Decimal::ZERO && shares < min_order_size,
    }
}

/// Previous internal grid. For price 0.812 it steps by 2.5 shares and is not the CLOB lot size.
pub fn shares_on_cent_grid(max_shares: Decimal, price: Decimal) -> Decimal {
    if max_shares <= Decimal::ZERO || price <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let price = price.round_dp(8);
    let scale = price.scale();
    let mantissa = match u64::try_from(price.mantissa().unsigned_abs()) {
        Ok(v) if v > 0 => v,
        _ => return Decimal::ZERO,
    };
    let divisor = 10u64.pow(3 + scale);
    let share_scale = 100_000u64;
    let g = gcd(mantissa, divisor);
    let step_k = divisor / g;
    let max_k = (max_shares * Decimal::from(share_scale))
        .round_dp_with_strategy(0, RoundingStrategy::ToZero)
        .to_u64()
        .unwrap_or(0);
    if max_k < step_k {
        return Decimal::ZERO;
    }
    let k = (max_k / step_k) * step_k;
    Decimal::from(k) / Decimal::from(share_scale)
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BookLevel {
    pub price: Decimal,
    pub size: Decimal,
}

#[derive(Clone, Debug)]
pub struct OrderBook {
    pub bids: Vec<BookLevel>,
    pub asks: Vec<BookLevel>,
    pub tick: Decimal,
    pub min_order_size: Decimal,
}

impl OrderBook {
    pub fn normalized(mut self) -> Self {
        self.asks.sort_by_key(|level| level.price);
        self.bids
            .sort_by_key(|level| std::cmp::Reverse(level.price));
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SimulatedFill {
    pub shares: Decimal,
    pub notional: Decimal,
    pub vwap: Decimal,
    pub fee_usd: Option<Decimal>,
    pub fee_status: &'static str,
    pub complete: bool,
}

pub fn walk_book(
    book: &OrderBook,
    side_buy: bool,
    limit: Decimal,
    want_shares: Decimal,
    schedule: Option<&FeeSchedule>,
) -> Result<SimulatedFill, RejectReason> {
    if want_shares <= Decimal::ZERO {
        return Err(RejectReason::BelowMinimum);
    }
    let levels: Vec<&BookLevel> = if side_buy {
        book.asks
            .iter()
            .filter(|l| l.price <= limit && l.price > Decimal::ZERO)
            .collect()
    } else {
        book.bids
            .iter()
            .filter(|l| l.price >= limit && l.price > Decimal::ZERO)
            .collect()
    };
    if levels.is_empty() {
        return Err(RejectReason::NoLiquidity);
    }
    let mut left = want_shares;
    let mut notional = Decimal::ZERO;
    let mut shares = Decimal::ZERO;
    for level in levels {
        if left <= Decimal::ZERO {
            break;
        }
        let take = left.min(level.size);
        let snapped = truncate_lot(take);
        if snapped <= Decimal::ZERO {
            continue;
        }
        shares += snapped;
        notional += (snapped * level.price).round_dp(2);
        left -= snapped;
    }
    if shares <= Decimal::ZERO || notional < EXCHANGE_MIN_NOTIONAL {
        return Err(RejectReason::NoLiquidity);
    }
    let vwap = (notional / shares).round_dp(6);
    let assessment = match schedule {
        Some(schedule) => taker_fee(shares, vwap, schedule),
        None => FeeAssessment::unknown(),
    };
    Ok(SimulatedFill {
        shares,
        notional,
        vwap,
        fee_usd: assessment.amount,
        fee_status: assessment.status,
        complete: left <= Decimal::new(1, 5),
    })
}

pub const FEE_MODEL: &str = "polymarket-fee-v1";

#[derive(Clone, Debug, PartialEq)]
pub struct FeeSchedule {
    pub fees_enabled: bool,
    pub rate: Decimal,
    pub exponent: u32,
    pub taker_only: bool,
    pub rebate_rate: Option<Decimal>,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeeAssessment {
    pub amount: Option<Decimal>,
    pub status: &'static str,
    pub model: &'static str,
}

impl FeeAssessment {
    fn unknown() -> Self {
        Self {
            amount: None,
            status: "unknown",
            model: FEE_MODEL,
        }
    }
}

/// `fee = shares × rate × (price × (1 - price))^exponent`, rounded to 5 decimal places.
/// A missing schedule stays unknown. A disabled schedule is a known zero, not an assumption.
pub fn taker_fee(shares: Decimal, price: Decimal, schedule: &FeeSchedule) -> FeeAssessment {
    if !schedule.fees_enabled {
        return FeeAssessment {
            amount: Some(Decimal::ZERO),
            status: "not_charged",
            model: FEE_MODEL,
        };
    }
    let curve = price * (Decimal::ONE - price);
    if shares <= Decimal::ZERO || price <= Decimal::ZERO || curve < Decimal::ZERO {
        return FeeAssessment::unknown();
    }
    let mut component = Decimal::ONE;
    for _ in 0..schedule.exponent {
        component *= curve;
    }
    let mut fee = (shares * schedule.rate * component).round_dp(5);
    if fee < Decimal::new(1, 5) {
        fee = Decimal::ZERO;
    }
    FeeAssessment {
        amount: Some(fee),
        status: "estimated",
        model: FEE_MODEL,
    }
}

pub fn maker_fee(schedule: &FeeSchedule) -> FeeAssessment {
    if !schedule.fees_enabled || schedule.taker_only {
        FeeAssessment {
            amount: Some(Decimal::ZERO),
            status: "not_charged",
            model: FEE_MODEL,
        }
    } else {
        FeeAssessment::unknown()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExitPlan {
    pub shares: Decimal,
    pub pending_exit_shares: Decimal,
    pub reject: Option<RejectReason>,
}

pub fn plan_exit(
    open_shares: Decimal,
    source_observed_shares: Decimal,
    source_sold_shares: Decimal,
    pending_exit_shares: Decimal,
    reference_price: Decimal,
    min_notional: Decimal,
) -> ExitPlan {
    if open_shares <= Decimal::ZERO || source_observed_shares <= Decimal::ZERO {
        return ExitPlan {
            shares: Decimal::ZERO,
            pending_exit_shares,
            reject: Some(RejectReason::NoPosition),
        };
    }
    let fraction = (source_sold_shares / source_observed_shares)
        .min(Decimal::ONE)
        .max(Decimal::ZERO);
    let desired = (open_shares * fraction + pending_exit_shares).min(open_shares);
    let notional = desired * reference_price;
    if desired <= Decimal::ZERO {
        return ExitPlan {
            shares: Decimal::ZERO,
            pending_exit_shares,
            reject: Some(RejectReason::BelowMinimum),
        };
    }
    if notional < min_notional.max(EXCHANGE_MIN_NOTIONAL) {
        return ExitPlan {
            shares: Decimal::ZERO,
            pending_exit_shares: pending_exit_shares + desired.min(open_shares),
            reject: Some(RejectReason::BelowMinimum),
        };
    }
    if desired > open_shares {
        return ExitPlan {
            shares: Decimal::ZERO,
            pending_exit_shares,
            reject: Some(RejectReason::SellLargerThanOpen),
        };
    }
    ExitPlan {
        shares: desired,
        pending_exit_shares: Decimal::ZERO,
        reject: None,
    }
}

pub fn slippage_bps(reference: Decimal, actual: Decimal) -> i32 {
    if reference <= Decimal::ZERO {
        return 0;
    }
    let diff = ((actual - reference).abs() / reference) * Decimal::from(10_000);
    diff.round_dp_with_strategy(0, RoundingStrategy::MidpointNearestEven)
        .to_i32()
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteObservation {
    Unreachable,
    NotFound,
    Open,
    Partial,
    Filled,
    Cancelled,
    Unrecognized,
}

/// Never turns an unconfirmed live order into `failed`. Absence of a response stays `unknown`.
pub fn reconcile_live_status(local: &str, aged: bool, remote: RemoteObservation) -> String {
    let confirmed = match remote {
        RemoteObservation::Filled => Some("filled"),
        RemoteObservation::Partial => Some("partially_filled"),
        RemoteObservation::Cancelled => Some("cancelled"),
        RemoteObservation::Open => Some("submitted"),
        RemoteObservation::Unreachable
        | RemoteObservation::Unrecognized
        | RemoteObservation::NotFound => None,
    };
    let next = if let Some(status) = confirmed {
        status
    } else {
        match local {
            "submitting" if aged => "unknown",
            "submitting" => "submitting",
            "unknown" => "unknown",
            "submitted" => "submitted",
            _ => "unknown",
        }
    };
    next.to_string()
}

pub fn dedup_key(tx_hash: &str, wallet: &str, token_id: &str, side: &str) -> String {
    format!(
        "{}|{}|{}|{}",
        tx_hash.trim().to_ascii_lowercase(),
        normalize_address(wallet),
        token_id.trim(),
        side.trim().to_ascii_lowercase()
    )
}

pub fn normalize_address(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub fn is_eth_address(value: &str) -> bool {
    let v = value.trim();
    let rest = v.strip_prefix("0x").unwrap_or(v);
    rest.len() == 40 && rest.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn is_private_key(value: &str) -> bool {
    let rest = value.trim().strip_prefix("0x").unwrap_or(value.trim());
    rest.len() == 64 && rest.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod decision_reason_tests {
    use super::decision_reason_explain;

    #[test]
    fn explains_capital_not_initialized() {
        let (t, d) = decision_reason_explain("strategy_capital_not_initialized").unwrap();
        assert!(t.contains("capital"));
        assert!(!d.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(v: &str) -> Decimal {
        Decimal::from_str(v).unwrap()
    }

    #[test]
    fn fixed_amount_respects_budget_and_balance_independently() {
        let mut input = SizingInput {
            mode: SizingMode::FixedAmount,
            fixed_amount_usd: Some(d("100")),
            proportion: None,
            multiplier: None,
            source_notional: d("5000"),
            max_order_usd: d("250"),
            min_order_usd: d("1"),
            strategy_remaining_usd: d("40"),
            account_exposure_remaining_usd: d("1000"),
            available_balance_usd: d("1000"),
            strategy_allocated_capital_usd: None,
            source_trader_equity_usd: None,
        };
        assert_eq!(target_notional(&input).unwrap(), d("40"));
        input.strategy_remaining_usd = d("100");
        input.available_balance_usd = d("25");
        assert_eq!(target_notional(&input).unwrap(), d("25"));
        input.available_balance_usd = d("0.5");
        assert_eq!(
            target_notional(&input),
            Err(RejectReason::InsufficientBalance)
        );
    }

    #[test]
    fn proportional_and_multiplier() {
        let proportional = SizingInput {
            mode: SizingMode::Proportional,
            fixed_amount_usd: None,
            proportion: Some(d("0.25")),
            multiplier: None,
            source_notional: d("80"),
            max_order_usd: d("100"),
            min_order_usd: d("1"),
            strategy_remaining_usd: d("100"),
            account_exposure_remaining_usd: d("100"),
            available_balance_usd: d("100"),
            strategy_allocated_capital_usd: None,
            source_trader_equity_usd: None,
        };
        assert_eq!(target_notional(&proportional).unwrap(), d("20"));
        let mult = SizingInput {
            mode: SizingMode::Multiplier,
            fixed_amount_usd: None,
            proportion: None,
            multiplier: Some(d("2")),
            ..proportional
        };
        assert_eq!(target_notional(&mult).unwrap(), d("100"));
    }

    #[test]
    fn portfolio_ratio_scales_by_allocation_over_equity() {
        let input = SizingInput {
            mode: SizingMode::PortfolioRatio,
            fixed_amount_usd: None,
            proportion: None,
            multiplier: None,
            source_notional: d("4000"),
            max_order_usd: d("10000"),
            min_order_usd: d("1"),
            strategy_remaining_usd: d("2000"),
            account_exposure_remaining_usd: d("10000"),
            available_balance_usd: d("10000"),
            strategy_allocated_capital_usd: Some(d("2000")),
            source_trader_equity_usd: Some(d("20000")),
        };
        assert_eq!(target_notional(&input).unwrap(), d("400"));
    }

    #[test]
    fn cent_grid_notional_has_two_decimals() {
        let shares = shares_on_cent_grid(d("13.33333"), d("0.37"));
        let notional = shares * d("0.37");
        let cents = notional * d("100");
        assert_eq!(cents, cents.trunc());
        assert!(shares <= d("13.33333"));
        assert!(shares.scale() <= 5);
    }

    #[test]
    fn book_walk_is_partial_and_respects_limit() {
        let book = OrderBook {
            bids: vec![BookLevel {
                price: d("0.40"),
                size: d("100"),
            }],
            asks: vec![
                BookLevel {
                    price: d("0.55"),
                    size: d("10"),
                },
                BookLevel {
                    price: d("0.45"),
                    size: d("4"),
                },
            ],
            tick: d("0.01"),
            min_order_size: Decimal::ZERO,
        }
        .normalized();
        let fill = walk_book(&book, true, d("0.50"), d("20"), None).unwrap();
        assert_eq!(fill.shares, d("4"));
        assert!(!fill.complete);
        assert!(walk_book(&book, true, d("0.40"), d("5"), None).is_err());
        assert_eq!(fill.fee_status, "unknown");
    }

    #[test]
    fn exit_never_sells_more_than_open_and_parks_dust() {
        let dust = plan_exit(d("10"), d("10"), d("0.1"), d("0"), d("0.40"), d("1"));
        assert_eq!(dust.reject, Some(RejectReason::BelowMinimum));
        assert!(dust.pending_exit_shares > Decimal::ZERO);
        let full = plan_exit(d("10"), d("8"), d("4"), d("0"), d("0.50"), d("1")).unwrap_shares();
        assert!(full <= d("10"));
    }

    #[test]
    fn unknown_is_not_failed_without_exchange_proof() {
        assert_eq!(
            reconcile_live_status("submitting", true, RemoteObservation::Unreachable),
            "unknown"
        );
        assert_eq!(
            reconcile_live_status("unknown", true, RemoteObservation::Unreachable),
            "unknown"
        );
        assert_eq!(
            reconcile_live_status("unknown", true, RemoteObservation::Unrecognized),
            "unknown"
        );
        assert_eq!(
            reconcile_live_status("unknown", true, RemoteObservation::Filled),
            "filled"
        );
        assert_eq!(
            reconcile_live_status("unknown", true, RemoteObservation::Partial),
            "partially_filled"
        );
        assert_eq!(
            reconcile_live_status("unknown", true, RemoteObservation::Cancelled),
            "cancelled"
        );
        assert_eq!(
            reconcile_live_status("submitted", false, RemoteObservation::Open),
            "submitted"
        );
        assert_ne!(
            reconcile_live_status("unknown", true, RemoteObservation::Unreachable),
            "failed"
        );
        assert_eq!(
            reconcile_live_status("unknown", true, RemoteObservation::NotFound),
            "unknown"
        );
    }

    #[test]
    fn sell_before_buy_does_not_open_a_position() {
        let plan = plan_exit(
            Decimal::ZERO,
            Decimal::ZERO,
            d("5"),
            Decimal::ZERO,
            d("0.4"),
            d("1"),
        );
        assert_eq!(plan.reject, Some(RejectReason::NoPosition));
        assert_eq!(plan.shares, Decimal::ZERO);
    }

    #[test]
    fn crypto_fee_matches_the_published_table() {
        let crypto = FeeSchedule {
            fees_enabled: true,
            rate: d("0.07"),
            exponent: 1,
            taker_only: true,
            rebate_rate: Some(d("0.20")),
            source: "gamma".into(),
        };
        let fee = taker_fee(d("100"), d("0.80"), &crypto);
        assert_eq!(fee.status, "estimated");
        assert_eq!(fee.amount.unwrap(), d("1.12"));
        assert_eq!(
            taker_fee(d("100"), d("0.50"), &crypto).amount.unwrap(),
            d("1.75")
        );
        assert_eq!(maker_fee(&crypto).amount.unwrap(), Decimal::ZERO);
        assert_eq!(maker_fee(&crypto).status, "not_charged");
        let mut free = crypto.clone();
        free.fees_enabled = false;
        assert_eq!(taker_fee(d("100"), d("0.80"), &free).status, "not_charged");
    }

    #[test]
    fn fifteen_budget_reserves_the_crypto_taker_fee() {
        let crypto = FeeSchedule {
            fees_enabled: true,
            rate: d("0.07"),
            exponent: 1,
            taker_only: true,
            rebate_rate: Some(d("0.20")),
            source: "gamma".into(),
        };
        let gross = trace_buy_size(
            d("15"),
            d("0.80"),
            d("0.812"),
            d("0.001"),
            d("5"),
            d("1000"),
            d("100"),
            Some(&crypto),
            d("0.812"),
        );
        assert_eq!(truncate_lot(gross.raw_requested_shares), d("18.47"));
        assert_eq!(gross.sdk_normalized_shares, d("18.47"));
        let tight = trace_buy_size(
            d("15"),
            d("0.80"),
            d("0.812"),
            d("0.001"),
            d("5"),
            d("1000"),
            d("15"),
            Some(&crypto),
            d("0.812"),
        );
        assert_eq!(tight.cent_grid_shares, d("17.5"));
        assert_eq!(tight.sdk_normalized_shares, d("18.23"));
        assert_eq!(tight.position_reserved_usd, d("14.802760"));
        assert_eq!(tight.fee_reserved_usd, d("0.19480"));
        assert_eq!(
            tight.sdk_normalized_shares,
            truncate_lot(tight.sdk_normalized_shares)
        );
        assert!(tight.position_reserved_usd + tight.fee_reserved_usd <= d("15"));
        assert!(tight.fee_reserved_usd > Decimal::ZERO);
        let sdk =
            polymarket_client_sdk_v2::clob::types::Amount::shares(tight.sdk_normalized_shares)
                .unwrap();
        assert_eq!(sdk.as_inner(), tight.sdk_normalized_shares);
        let bumped = tight.sdk_normalized_shares + Decimal::new(1, 2);
        let bumped_cost = collateral_usd(true, d("0.812"), bumped)
            + taker_fee(bumped, d("0.812"), &crypto).amount.unwrap();
        assert!(bumped_cost > d("15"));
    }

    #[test]
    fn lot_size_cases_do_not_use_min_order_as_a_step() {
        let crypto = FeeSchedule {
            fees_enabled: false,
            rate: Decimal::ZERO,
            exponent: 1,
            taker_only: true,
            rebate_rate: None,
            source: "test".into(),
        };
        let at_80 = trace_buy_size(
            d("10"),
            d("0.80"),
            d("0.812"),
            d("0.01"),
            d("1"),
            d("1000"),
            d("100"),
            Some(&crypto),
            d("0.812"),
        );
        assert_eq!(
            truncate_lot(d("10") / d("0.812")),
            at_80.sdk_normalized_shares
        );
        let exact = trace_buy_size(
            d("15"),
            d("0.80"),
            d("0.80"),
            d("0.01"),
            d("5"),
            d("1000"),
            d("100"),
            Some(&crypto),
            d("0.80"),
        );
        assert_eq!(exact.sdk_normalized_shares, d("18.75"));
        let thin = trace_buy_size(
            d("15"),
            d("0.80"),
            d("0.812"),
            d("0.001"),
            d("5"),
            d("10"),
            d("100"),
            Some(&crypto),
            d("0.812"),
        );
        assert_eq!(thin.sdk_normalized_shares, d("10.00"));
        let tiny = trace_buy_size(
            d("0.40"),
            d("0.50"),
            d("0.50"),
            d("0.01"),
            d("5"),
            d("100"),
            d("100"),
            Some(&crypto),
            d("0.50"),
        );
        assert!(tiny.below_min_order_size);
        let unknown = trace_buy_size(
            d("15"),
            d("0.80"),
            d("0.812"),
            d("0.001"),
            d("1"),
            d("1000"),
            d("15"),
            None,
            d("0.812"),
        );
        assert!(unknown.fee_unknown);
        assert_eq!(unknown.sdk_normalized_shares, Decimal::ZERO);
    }

    #[test]
    fn fee_reserve_respects_budget_balance_and_other_schedules() {
        let crypto = FeeSchedule {
            fees_enabled: true,
            rate: d("0.07"),
            exponent: 1,
            taker_only: true,
            rebate_rate: Some(d("0.20")),
            source: "gamma".into(),
        };
        let politics = FeeSchedule {
            fees_enabled: true,
            rate: d("0.04"),
            exponent: 1,
            taker_only: true,
            rebate_rate: Some(d("0.25")),
            source: "gamma".into(),
        };
        let crypto_fit = trace_buy_size(
            d("15"),
            d("0.80"),
            d("0.812"),
            d("0.001"),
            d("5"),
            d("1000"),
            d("15"),
            Some(&crypto),
            d("0.80"),
        );
        let politics_fit = trace_buy_size(
            d("15"),
            d("0.80"),
            d("0.812"),
            d("0.001"),
            d("5"),
            d("1000"),
            d("15"),
            Some(&politics),
            d("0.80"),
        );
        assert_eq!(crypto_fit.sdk_normalized_shares, d("18.22"));
        assert_eq!(crypto_fit.fee_reserved_usd, d("0.20406"));
        assert_eq!(politics_fit.sdk_normalized_shares, d("18.32"));
        assert_eq!(politics_fit.fee_reserved_usd, d("0.11725"));
        assert!(
            crypto_fit.fee_reserved_usd > politics_fit.fee_reserved_usd
                || crypto_fit.sdk_normalized_shares < politics_fit.sdk_normalized_shares
        );
        assert!(crypto_fit.position_reserved_usd + crypto_fit.fee_reserved_usd <= d("15"));
        assert!(politics_fit.position_reserved_usd + politics_fit.fee_reserved_usd <= d("15"));
        assert!(politics_fit.sdk_normalized_shares >= crypto_fit.sdk_normalized_shares);

        let broke = trace_buy_size(
            d("15"),
            d("0.80"),
            d("0.812"),
            d("0.001"),
            d("5"),
            d("1000"),
            d("0.50"),
            Some(&crypto),
            d("0.80"),
        );
        assert!(broke.below_min_order_size || broke.sdk_normalized_shares * d("0.812") < d("1"));
        assert!(broke.position_reserved_usd + broke.fee_reserved_usd <= d("0.50"));

        let first = trace_buy_size(
            d("15"),
            d("0.80"),
            d("0.812"),
            d("0.001"),
            d("1"),
            d("1000"),
            d("15"),
            Some(&crypto),
            d("0.80"),
        );
        let left = available_after_reserve(
            d("15"),
            first.position_reserved_usd + first.fee_reserved_usd,
        );
        let second = trace_buy_size(
            d("15"),
            d("0.80"),
            d("0.812"),
            d("0.001"),
            d("1"),
            d("1000"),
            left,
            Some(&crypto),
            d("0.80"),
        );
        assert!(
            first.position_reserved_usd
                + first.fee_reserved_usd
                + second.position_reserved_usd
                + second.fee_reserved_usd
                <= d("15")
        );
        assert_eq!(available_after_reserve(d("10"), d("12")), Decimal::ZERO);

        let (position_left, fee_left) =
            reserves_after_execution(d("10"), d("4"), d("8"), d("0.20"), Some(d("0.08")));
        assert_eq!(position_left, d("4.80000000"));
        assert_eq!(fee_left, d("0.12000000"));
        assert_eq!(
            reserves_after_execution(d("10"), d("10"), d("8"), d("0.20"), Some(d("0.19"))),
            (Decimal::ZERO, Decimal::ZERO)
        );
        assert_eq!(
            reserves_after_execution(d("10"), d("10"), d("8"), d("0.20"), None).1,
            d("0.20")
        );
        assert_eq!(fee_reconciliation_delta(d("0.20"), d("0.17")), d("-0.03"));
        assert_eq!(conservative_fee_price(d("0.80"), d("0.812")), d("0.80"));
        assert_eq!(conservative_fee_price(d("0.40"), d("0.70")), d("0.5"));
    }

    #[test]
    fn dedup_key_is_stable() {
        let a = dedup_key("0xABC", "0xAa", "1", "BUY");
        let b = dedup_key("0xabc", "0xaa", "1", "buy");
        assert_eq!(a, b);
    }

    impl ExitPlan {
        fn unwrap_shares(self) -> Decimal {
            assert!(self.reject.is_none());
            self.shares
        }
    }
}
