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
    /// Leader open-position market value (Data API `/v2/value`), not wallet equity.
    PositionValueRatio,
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
    pub available_balance_usd: Decimal,
    /// Strategy capital allocated to this copy strategy (not whole account balance).
    pub strategy_allocated_capital_usd: Option<Decimal>,
    /// Observed source trader total equity for portfolio-relative sizing.
    pub source_trader_equity_usd: Option<Decimal>,
    /// Leader aggregate open-position MTM for position_value_ratio (not wallet equity).
    pub source_open_positions_value_usd: Option<Decimal>,
    /// PAPER strategy opt-in: skip Polymarket exchange $1 floor in sizing caps (LIVE always strict).
    pub allow_subminimum_paper_trades: bool,
    /// PVR-only: small BUY override when source notional is strictly below threshold.
    pub copy_small_buys_1to1: bool,
    pub small_buy_threshold_usd: Option<Decimal>,
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
    SourcePositionValueUnavailable,
    SourcePositionValueStale,
    SourcePositionValueZero,
    StrategyCapitalNotInitialized,
}

impl RejectReason {
    pub fn as_str(&self) -> &'static str {
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
            Self::SourcePositionValueUnavailable => "source_position_value_unavailable",
            Self::SourcePositionValueStale => "source_position_value_stale",
            Self::SourcePositionValueZero => "source_position_value_zero",
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
        "source_position_value_unavailable" => (
            "Leader open-position value unknown",
            "Position Value Ratio needs a fresh open-position market value snapshot (Polymarket Data API /v2/value).",
        ),
        "source_position_value_stale" => (
            "Leader open-position value stale",
            "The open-position value snapshot is too old; wait for the worker to refresh it.",
        ),
        "source_position_value_zero" => (
            "Leader has no open positions",
            "Position Value Ratio needs a positive open-position market value; the leader portfolio reads as empty.",
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
            "No liquidity within allowed price",
            "The Polymarket CLOB has no depth at or inside your copy limit. Paper mode does not fill at the leader price unless PAPER_ALLOW_SYNTHETIC_FILLS is explicitly enabled.",
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
        "fee_unknown" => (
            "Unable to determine market trading fee",
            "Polymarket fee parameters could not be loaded for this market. Copy was skipped until fee status is known (we do not assume zero fee).",
        ),
        _ => return None,
    })
}

/// Small BUY Override: leader notional at or below threshold uses `source × trade_multiplier` only.
pub fn small_buy_override_applies(
    copy_small_buys_1to1: bool,
    small_buy_threshold_usd: Option<Decimal>,
    source_notional: Decimal,
) -> bool {
    copy_small_buys_1to1
        && small_buy_threshold_usd
            .filter(|t| *t > Decimal::ZERO)
            .is_some_and(|t| source_notional <= t)
}

pub fn small_buy_trade_multiplier(input: &SizingInput) -> Decimal {
    match input.mode {
        SizingMode::PositionValueRatio | SizingMode::FixedAmount => input
            .multiplier
            .filter(|m| *m > Decimal::ZERO)
            .unwrap_or(Decimal::ONE),
        SizingMode::Proportional => input
            .proportion
            .or(input.multiplier)
            .filter(|c| *c > Decimal::ZERO)
            .unwrap_or(Decimal::ONE),
        SizingMode::Multiplier => input
            .multiplier
            .filter(|m| *m > Decimal::ZERO)
            .unwrap_or(Decimal::ONE),
        SizingMode::PortfolioRatio => Decimal::ONE,
    }
}

fn small_buy_1to1_applies(input: &SizingInput) -> bool {
    small_buy_override_applies(
        input.copy_small_buys_1to1,
        input.small_buy_threshold_usd,
        input.source_notional,
    )
}

fn position_value_ratio_raw(input: &SizingInput) -> Result<Decimal, RejectReason> {
    let positions = input
        .source_open_positions_value_usd
        .filter(|v| *v > Decimal::ZERO)
        .ok_or(RejectReason::SourcePositionValueUnavailable)?;
    let alloc = input
        .strategy_allocated_capital_usd
        .filter(|v| *v > Decimal::ZERO)
        .ok_or(RejectReason::MissingSizing)?;
    let base = input.source_notional * alloc / positions;
    let coeff = input
        .multiplier
        .filter(|m| *m > Decimal::ZERO)
        .unwrap_or(Decimal::ONE);
    Ok((base * coeff).round_dp(8))
}

pub fn target_notional(input: &SizingInput) -> Result<Decimal, RejectReason> {
    if input.source_notional <= Decimal::ZERO {
        return Err(RejectReason::BelowMinimum);
    }
    let raw = if small_buy_1to1_applies(input) {
        (input.source_notional * small_buy_trade_multiplier(input)).round_dp(8)
    } else {
        match input.mode {
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
                (input.source_notional * alloc / equity).round_dp(8)
            }
            SizingMode::PositionValueRatio => position_value_ratio_raw(input)?,
        }
    };
    if raw <= Decimal::ZERO {
        return Err(RejectReason::BelowMinimum);
    }
    let balance_floor = if input.allow_subminimum_paper_trades {
        input.min_order_usd.max(Decimal::ZERO)
    } else {
        input.min_order_usd.max(EXCHANGE_MIN_NOTIONAL)
    };
  if balance_floor > Decimal::ZERO {
        if input.available_balance_usd < balance_floor {
            return Err(RejectReason::InsufficientBalance);
        }
        if input.strategy_remaining_usd < balance_floor {
            return Err(RejectReason::StrategyBudget);
        }
    }
    let capped = raw
        .min(input.max_order_usd)
        .min(input.strategy_remaining_usd)
        .min(input.available_balance_usd)
        .round_dp_with_strategy(2, RoundingStrategy::ToZero);
    let floor = if input.allow_subminimum_paper_trades {
        input.min_order_usd.max(Decimal::ZERO)
    } else {
        input.min_order_usd.max(EXCHANGE_MIN_NOTIONAL)
    };
    if floor > Decimal::ZERO && capped < floor {
        if raw > input.available_balance_usd {
            return Err(RejectReason::InsufficientBalance);
        }
        if raw > input.strategy_remaining_usd {
            return Err(RejectReason::StrategyBudget);
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

/// How a PAPER copy order was simulated against liquidity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperExecutionMode {
    /// Walked the live CLOB snapshot within the copy limit price.
    Clob,
    /// Synthetic depth at the leader trade price (no CLOB depth within limit).
    SyntheticSourcePrice,
}

impl PaperExecutionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clob => "clob",
            Self::SyntheticSourcePrice => "synthetic_source_price",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PaperFillContext {
    pub source_price: Decimal,
    pub copy_limit_price: Decimal,
    pub book_available: bool,
  /// `clob_only` or `synthetic_experimental` when a fill is produced.
    pub simulation_policy: &'static str,
    /// True for PAPER micro/synthetic fills that are not CLOB-executable on Polymarket.
    #[serde(default)]
    pub paper_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_reason: Option<String>,
    /// Best opposite-side level on the CLOB (may be outside the copy limit).
    pub best_clob_price: Option<Decimal>,
    /// Best level on the CLOB within the copy limit (buy: ask ≤ limit).
    pub best_clob_price_within_limit: Option<Decimal>,
    pub clob_liquidity_shares_within_limit: Option<Decimal>,
    pub execution_price: Decimal,
    /// Leader/source vs best CLOB (absolute bps) when the book was readable.
    pub leader_vs_best_clob_bps: Option<i32>,
    /// `(best_clob_price − execution) / execution` in bps for the actual fill VWAP.
    pub deviation_from_best_clob_bps: Option<i32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CopyFillSim {
    pub fill: SimulatedFill,
    pub paper_execution_mode: PaperExecutionMode,
    pub context: PaperFillContext,
}

pub fn clob_book_insight(
    book: &OrderBook,
    side_buy: bool,
    limit: Decimal,
) -> (Option<Decimal>, Option<Decimal>, Decimal) {
    let levels: Vec<&BookLevel> = if side_buy {
        book.asks.iter().collect()
    } else {
        book.bids.iter().collect()
    };
    let best_any = levels.first().map(|l| l.price);
    let within: Vec<&BookLevel> = if side_buy {
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
    let best_within = within.first().map(|l| l.price);
    let liq = within.iter().fold(Decimal::ZERO, |sum, l| sum + l.size);
    (best_any, best_within, liq)
}

fn deviation_from_clob_bps(best_clob: Decimal, execution: Decimal) -> Option<i32> {
    if best_clob <= Decimal::ZERO || execution <= Decimal::ZERO {
        return None;
    }
    Some(slippage_bps(execution, best_clob))
}

pub fn paper_clob_diagnostic(
    book: &Result<OrderBook, String>,
    side_buy: bool,
    source_price: Decimal,
    limit: Decimal,
) -> PaperFillContext {
    let book_available = book.is_ok();
    let (best_any, best_within, liq) = book
        .as_ref()
        .map(|b| clob_book_insight(b, side_buy, limit))
        .unwrap_or((None, None, Decimal::ZERO));
    let leader_gap = best_any.map(|b| slippage_bps(source_price, b));
    PaperFillContext {
        source_price,
        copy_limit_price: limit,
        book_available,
        simulation_policy: "clob_only",
        best_clob_price: best_any,
        best_clob_price_within_limit: best_within,
        clob_liquidity_shares_within_limit: if book_available {
            Some(liq)
        } else {
            None
        },
        execution_price: Decimal::ZERO,
        leader_vs_best_clob_bps: leader_gap,
        deviation_from_best_clob_bps: None,
        paper_only: false,
        execution_reason: None,
    }
}

fn context_from_fill(
    book: &Result<OrderBook, String>,
    side_buy: bool,
    source_price: Decimal,
    limit: Decimal,
    fill: &SimulatedFill,
    policy: &'static str,
) -> PaperFillContext {
    let mut ctx = paper_clob_diagnostic(book, side_buy, source_price, limit);
    ctx.simulation_policy = policy;
    ctx.execution_price = fill.vwap;
    ctx.deviation_from_best_clob_bps = ctx
        .best_clob_price
        .and_then(|b| deviation_from_clob_bps(b, fill.vwap));
    ctx
}

/// Returns true when leader price is too far from the best visible CLOB quote to allow PAPER copy.
pub fn clob_market_gap_exceeds(source_price: Decimal, best_clob: Decimal, max_gap_bps: i32) -> bool {
    if source_price <= Decimal::ZERO || best_clob <= Decimal::ZERO {
        return false;
    }
    slippage_bps(source_price, best_clob) > max_gap_bps
}

/// PAPER simulation only: fills at the leader's trade price when the live book has no depth
/// within the copy limit (typical after the leader's fill moves the market).
pub fn paper_synthetic_fill_book(
    side_buy: bool,
    source_price: Decimal,
    depth_shares: Decimal,
    tick: Decimal,
) -> OrderBook {
    let price = if source_price > Decimal::ZERO {
        source_price
    } else {
        Decimal::new(50, 2)
    };
    let size = depth_shares.max(Decimal::from(1_000_000));
    let level = BookLevel { price, size };
    OrderBook {
        bids: if side_buy { vec![] } else { vec![level.clone()] },
        asks: if side_buy { vec![level] } else { vec![] },
        tick: if tick > Decimal::ZERO {
            tick
        } else {
            Decimal::new(1, 2)
        },
        min_order_size: Decimal::ZERO,
    }
    .normalized()
}

/// Shares for a PAPER copy from sized notional (2-decimal SDK lot).
pub fn paper_shares_from_notional(notional_usd: Decimal, source_price: Decimal) -> Decimal {
    if notional_usd <= Decimal::ZERO || source_price <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    truncate_lot(notional_usd / source_price)
}

pub fn paper_qualifies_for_clob_walk(
    shares: Decimal,
    limit: Decimal,
    market_min_shares: Decimal,
) -> bool {
    if shares <= Decimal::ZERO || limit <= Decimal::ZERO {
        return false;
    }
    shares >= market_min_shares.max(Decimal::ZERO) && shares * limit >= EXCHANGE_MIN_NOTIONAL
}

pub fn paper_fill_at_leader_price(
    shares: Decimal,
    source_price: Decimal,
    fee: Option<&FeeSchedule>,
) -> Result<SimulatedFill, RejectReason> {
    let shares = truncate_lot(shares);
    if shares <= Decimal::ZERO || source_price <= Decimal::ZERO {
        return Err(RejectReason::BelowMinimum);
    }
    let notional = (shares * source_price).round_dp(8);
    let assessment = match fee {
        Some(schedule) => taker_fee(shares, source_price, schedule),
        None => FeeAssessment::unknown(),
    };
    Ok(SimulatedFill {
        shares,
        notional,
        vwap: source_price,
        fee_usd: assessment.amount,
        fee_status: assessment.status,
        complete: true,
    })
}

/// PAPER: CLOB when size is exchange-qualified; otherwise synthetic at leader price (micro copy).
pub fn simulate_paper_copy_fill(
    book: &Result<OrderBook, String>,
    side_buy: bool,
    source_price: Decimal,
    limit: Decimal,
    target_shares: Decimal,
    target_notional_usd: Decimal,
    market_min_shares: Decimal,
    fee: Option<&FeeSchedule>,
) -> Result<CopyFillSim, RejectReason> {
    let shares = if target_shares > Decimal::ZERO {
        target_shares
    } else {
        paper_shares_from_notional(target_notional_usd, source_price)
    };
    if shares <= Decimal::ZERO {
        return Err(RejectReason::BelowMinimum);
    }

    let mode_clob = |fill: SimulatedFill| CopyFillSim {
        paper_execution_mode: PaperExecutionMode::Clob,
        context: context_from_fill(book, side_buy, source_price, limit, &fill, "clob"),
        fill,
    };

    if paper_qualifies_for_clob_walk(shares, limit, market_min_shares) {
        if let Ok(b) = book {
            if let Ok(fill) = walk_book(b, side_buy, limit, shares, fee, true) {
                return Ok(mode_clob(fill));
            }
        }
    }

    let fill = paper_fill_at_leader_price(shares, source_price, fee)?;
    let reason = if paper_qualifies_for_clob_walk(shares, limit, market_min_shares) {
        "clob_unavailable".to_string()
    } else {
        "below_exchange_minimum".to_string()
    };
    let mut ctx = paper_clob_diagnostic(book, side_buy, source_price, limit);
    ctx.simulation_policy = "paper_micro_synthetic";
    ctx.paper_only = true;
    ctx.execution_reason = Some(reason);
    ctx.execution_price = fill.vwap;
    ctx.deviation_from_best_clob_bps = ctx
        .best_clob_price
        .and_then(|b| deviation_from_clob_bps(b, fill.vwap));
    Ok(CopyFillSim {
        paper_execution_mode: PaperExecutionMode::SyntheticSourcePrice,
        context: ctx,
        fill,
    })
}

/// LIVE: strict CLOB walk. PAPER: micro sizing + synthetic leader-price fill when needed.
pub fn simulate_copy_fill(
    book: &Result<OrderBook, String>,
    paper_trading: bool,
    allow_subminimum_paper_trades: bool,
    _allow_synthetic_experimental: bool,
    _max_clob_gap_bps: i32,
    side_buy: bool,
    source_price: Decimal,
    limit: Decimal,
    target_shares: Decimal,
    target_notional_usd: Decimal,
    market_min_shares: Decimal,
    fee: Option<&FeeSchedule>,
) -> Result<CopyFillSim, RejectReason> {
    if paper_trading {
        if allow_subminimum_paper_trades {
            return simulate_paper_copy_fill(
                book,
                side_buy,
                source_price,
                limit,
                target_shares,
                target_notional_usd,
                market_min_shares,
                fee,
            );
        }
        let shares = if target_shares > Decimal::ZERO {
            target_shares
        } else {
            paper_shares_from_notional(target_notional_usd, source_price)
        };
        if shares <= Decimal::ZERO {
            return Err(RejectReason::BelowMinimum);
        }
        if !paper_qualifies_for_clob_walk(shares, limit, market_min_shares) {
            return Err(RejectReason::BelowMinimum);
        }
        let mode_clob = |fill: SimulatedFill| CopyFillSim {
            paper_execution_mode: PaperExecutionMode::Clob,
            context: context_from_fill(book, side_buy, source_price, limit, &fill, "paper_clob_strict"),
            fill,
        };
        return match book {
            Ok(b) => walk_book(b, side_buy, limit, shares, fee, true).map(mode_clob),
            Err(_) => Err(RejectReason::NoLiquidity),
        };
    }
    let mode_clob = |fill: SimulatedFill| CopyFillSim {
        paper_execution_mode: PaperExecutionMode::Clob,
        context: context_from_fill(book, side_buy, source_price, limit, &fill, "live_clob"),
        fill,
    };
    match book {
        Ok(b) => walk_book(b, side_buy, limit, target_shares, fee, true).map(mode_clob),
        Err(_) => Err(RejectReason::NoLiquidity),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    enforce_exchange_minimums: bool,
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
    if shares <= Decimal::ZERO {
        return Err(RejectReason::NoLiquidity);
    }
    if enforce_exchange_minimums && notional < EXCHANGE_MIN_NOTIONAL {
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
    allow_subminimum_paper_trades: bool,
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
    let floor = if allow_subminimum_paper_trades {
        min_notional.max(Decimal::ZERO)
    } else {
        min_notional.max(EXCHANGE_MIN_NOTIONAL)
    };
    if floor > Decimal::ZERO && notional < floor {
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

/// Equal Copy SELL: `leader_sell_notional × trade_multiplier`, capped by max order and open shares.
pub fn plan_equal_copy_exit(
    open_shares: Decimal,
    pending_exit_shares: Decimal,
    target_notional: Decimal,
    reference_price: Decimal,
    min_notional: Decimal,
    allow_subminimum_paper_trades: bool,
) -> ExitPlan {
    if open_shares <= Decimal::ZERO {
        return ExitPlan {
            shares: Decimal::ZERO,
            pending_exit_shares,
            reject: Some(RejectReason::NoPosition),
        };
    }
    if target_notional <= Decimal::ZERO || reference_price <= Decimal::ZERO {
        return ExitPlan {
            shares: Decimal::ZERO,
            pending_exit_shares,
            reject: Some(RejectReason::BelowMinimum),
        };
    }
    let floor = if allow_subminimum_paper_trades {
        min_notional.max(Decimal::ZERO)
    } else {
        min_notional.max(EXCHANGE_MIN_NOTIONAL)
    };
    if floor > Decimal::ZERO && target_notional < floor {
        return ExitPlan {
            shares: Decimal::ZERO,
            pending_exit_shares,
            reject: Some(RejectReason::BelowMinimum),
        };
    }
    let want_shares = truncate_lot(target_notional / reference_price);
    let sellable = (open_shares - pending_exit_shares).max(Decimal::ZERO);
    let shares = want_shares.min(sellable);
    if shares <= Decimal::ZERO {
        return ExitPlan {
            shares: Decimal::ZERO,
            pending_exit_shares,
            reject: Some(RejectReason::BelowMinimum),
        };
    }
    let notional = shares * reference_price;
    if floor > Decimal::ZERO && notional < floor {
        return ExitPlan {
            shares: Decimal::ZERO,
            pending_exit_shares,
            reject: Some(RejectReason::BelowMinimum),
        };
    }
    if shares > open_shares {
        return ExitPlan {
            shares: Decimal::ZERO,
            pending_exit_shares,
            reject: Some(RejectReason::SellLargerThanOpen),
        };
    }
    ExitPlan {
        shares,
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

    #[test]
    fn explains_fee_unknown() {
        let (t, _) = decision_reason_explain("fee_unknown").unwrap();
        assert!(t.to_ascii_lowercase().contains("fee"));
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
            available_balance_usd: d("1000"),
            strategy_allocated_capital_usd: None,
            source_trader_equity_usd: None,
            source_open_positions_value_usd: None,
            allow_subminimum_paper_trades: false,
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
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
            available_balance_usd: d("100"),
            strategy_allocated_capital_usd: None,
            source_trader_equity_usd: None,
            source_open_positions_value_usd: None,
            allow_subminimum_paper_trades: false,
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
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
            available_balance_usd: d("10000"),
            strategy_allocated_capital_usd: Some(d("2000")),
            source_trader_equity_usd: Some(d("20000")),
            source_open_positions_value_usd: None,
            allow_subminimum_paper_trades: false,
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
        };
        assert_eq!(target_notional(&input).unwrap(), d("400"));
    }

    #[test]
    fn position_value_ratio_scales_by_allocation_over_open_positions() {
        let input = SizingInput {
            mode: SizingMode::PositionValueRatio,
            fixed_amount_usd: None,
            proportion: None,
            multiplier: None,
            source_notional: d("200"),
            max_order_usd: d("10000"),
            min_order_usd: d("1"),
            strategy_remaining_usd: d("2000"),
            available_balance_usd: d("10000"),
            strategy_allocated_capital_usd: Some(d("2000")),
            source_trader_equity_usd: None,
            source_open_positions_value_usd: Some(d("4000")),
            allow_subminimum_paper_trades: false,
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
        };
        assert_eq!(target_notional(&input).unwrap(), d("100"));
    }

    #[test]
    fn position_value_ratio_small_buy_1to1_before_caps() {
        let input = SizingInput {
            mode: SizingMode::PositionValueRatio,
            fixed_amount_usd: None,
            proportion: None,
            multiplier: Some(d("10")),
            source_notional: d("4.50"),
            max_order_usd: d("10000"),
            min_order_usd: d("1"),
            strategy_remaining_usd: d("2000"),
            available_balance_usd: d("10000"),
            strategy_allocated_capital_usd: Some(d("2000")),
            source_trader_equity_usd: None,
            source_open_positions_value_usd: Some(d("4000")),
            allow_subminimum_paper_trades: false,
            copy_small_buys_1to1: true,
            small_buy_threshold_usd: Some(d("5")),
        };
        assert_eq!(target_notional(&input).unwrap(), d("45"));
    }

    #[test]
    fn equal_copy_exit_respects_open_shares() {
        let plan = plan_equal_copy_exit(
            d("100"),
            Decimal::ZERO,
            d("20"),
            d("0.5"),
            d("1"),
            false,
        );
        assert!(plan.reject.is_none());
        assert_eq!(plan.shares, d("40"));
        let capped = plan_equal_copy_exit(d("10"), Decimal::ZERO, d("20"), d("0.5"), d("1"), false);
        assert_eq!(capped.shares, d("10"));
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
        let fill = walk_book(&book, true, d("0.50"), d("20"), None, true).unwrap();
        assert_eq!(fill.shares, d("4"));
        assert!(!fill.complete);
        assert!(walk_book(&book, true, d("0.40"), d("5"), None, true).is_err());
        assert_eq!(fill.fee_status, "unknown");
    }

    #[test]
    fn exit_leader_fraction_of_follower_open_shares() {
        let plan = plan_exit(d("60"), d("100"), d("50"), Decimal::ZERO, d("0.50"), d("1"), false)
            .unwrap_shares();
        assert_eq!(plan, d("30"));
    }

    #[test]
    fn proportional_small_buy_override_target_notional() {
        let input = SizingInput {
            mode: SizingMode::Proportional,
            fixed_amount_usd: None,
            proportion: Some(d("1")),
            multiplier: None,
            source_notional: d("4"),
            max_order_usd: d("10000"),
            min_order_usd: d("1"),
            strategy_remaining_usd: d("2000"),
            available_balance_usd: d("10000"),
            strategy_allocated_capital_usd: None,
            source_trader_equity_usd: None,
            source_open_positions_value_usd: None,
            allow_subminimum_paper_trades: false,
            copy_small_buys_1to1: true,
            small_buy_threshold_usd: Some(d("5")),
        };
        assert_eq!(target_notional(&input).unwrap(), d("4"));
        let input_high = SizingInput {
            source_notional: d("10"),
            proportion: Some(d("0.1")),
            copy_small_buys_1to1: false,
            small_buy_threshold_usd: None,
            ..input
        };
        assert_eq!(target_notional(&input_high).unwrap(), d("1"));
    }

    #[test]
    fn small_buy_at_threshold_uses_override() {
        let input = SizingInput {
            mode: SizingMode::Proportional,
            fixed_amount_usd: None,
            proportion: Some(d("1")),
            multiplier: None,
            source_notional: d("5"),
            max_order_usd: d("10000"),
            min_order_usd: d("1"),
            strategy_remaining_usd: d("2000"),
            available_balance_usd: d("10000"),
            strategy_allocated_capital_usd: None,
            source_trader_equity_usd: None,
            source_open_positions_value_usd: None,
            allow_subminimum_paper_trades: false,
            copy_small_buys_1to1: true,
            small_buy_threshold_usd: Some(d("5")),
        };
        assert_eq!(target_notional(&input).unwrap(), d("5"));
    }

    #[test]
    fn exit_never_sells_more_than_open_and_parks_dust() {
        let dust = plan_exit(d("10"), d("10"), d("0.1"), d("0"), d("0.40"), d("1"), false);
        assert_eq!(dust.reject, Some(RejectReason::BelowMinimum));
        assert!(dust.pending_exit_shares > Decimal::ZERO);
        let full = plan_exit(d("10"), d("8"), d("4"), d("0"), d("0.50"), d("1"), false).unwrap_shares();
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
            false,
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

    #[test]
    fn paper_sim_uses_source_price_when_clob_asks_above_limit() {
        let thin = OrderBook {
            bids: vec![],
            asks: vec![BookLevel {
                price: d("0.93"),
                size: d("1000"),
            }],
            tick: d("0.01"),
            min_order_size: Decimal::ZERO,
        };
        let book = Ok(thin);
        let fee = FeeSchedule {
            fees_enabled: false,
            rate: Decimal::ZERO,
            exponent: 1,
            taker_only: true,
            rebate_rate: None,
            source: "test".into(),
        };
        assert_eq!(
            walk_book(&book.as_ref().unwrap(), true, d("0.59"), d("17"), Some(&fee), true),
            Err(RejectReason::NoLiquidity)
        );
        let syn = simulate_copy_fill(
            &book,
            true,
            true,
            false,
            10_000,
            true,
            d("0.5841943"),
            d("0.59"),
            d("17"),
            d("9.93"),
            Decimal::ZERO,
            Some(&fee),
        )
        .expect("paper synthetic when CLOB empty within limit");
        assert_eq!(
            syn.paper_execution_mode,
            PaperExecutionMode::SyntheticSourcePrice
        );
        assert!(syn.context.paper_only);
    }

    #[test]
    fn paper_clob_fill_when_liquidity_within_limit() {
        let book = Ok(OrderBook {
            bids: vec![],
            asks: vec![BookLevel {
                price: d("0.55"),
                size: d("1000"),
            }],
            tick: d("0.01"),
            min_order_size: Decimal::ZERO,
        });
        let fee = FeeSchedule {
            fees_enabled: false,
            rate: Decimal::ZERO,
            exponent: 1,
            taker_only: true,
            rebate_rate: None,
            source: "test".into(),
        };
        let sim = simulate_copy_fill(
            &book,
            true,
            false,
            false,
            500,
            true,
            d("0.54"),
            d("0.55"),
            d("10"),
            d("5.5"),
            Decimal::ZERO,
            Some(&fee),
        )
        .unwrap();
        assert_eq!(sim.paper_execution_mode, PaperExecutionMode::Clob);
        assert_eq!(sim.fill.shares, d("10"));
    }

    #[test]
    fn synthetic_fill_respects_max_slippage_vs_source() {
        let source = d("0.50");
        let limit = d("0.50");
        let synthetic = paper_synthetic_fill_book(true, source, d("10"), d("0.01"));
        let fill = walk_book(&synthetic, true, limit, d("10"), None, false).unwrap();
        assert!(slippage_bps(source, fill.vwap) <= 0);
    }

    #[test]
    fn paper_book_unavailable_uses_leader_price_synthetic() {
        let book: Result<OrderBook, String> = Err("unavailable".into());
        let fee = FeeSchedule {
            fees_enabled: false,
            rate: Decimal::ZERO,
            exponent: 1,
            taker_only: true,
            rebate_rate: None,
            source: "test".into(),
        };
        let sim = simulate_copy_fill(
            &book,
            true,
            true,
            false,
            500,
            true,
            d("0.5"),
            d("0.55"),
            d("10"),
            d("5"),
            Decimal::ZERO,
            Some(&fee),
        )
        .expect("synthetic without book");
        assert_eq!(
            sim.paper_execution_mode,
            PaperExecutionMode::SyntheticSourcePrice
        );
        assert_eq!(sim.fill.vwap, d("0.5"));
    }

    #[test]
    fn paper_micro_buy_sub_dollar_notional() {
        let book: Result<OrderBook, String> = Err("unavailable".into());
        let fee = FeeSchedule {
            fees_enabled: false,
            rate: Decimal::ZERO,
            exponent: 1,
            taker_only: true,
            rebate_rate: None,
            source: "test".into(),
        };
        for (notional, price) in [(d("0.50"), d("0.08")), (d("0.10"), d("0.05")), (d("0.01"), d("0.01"))]
        {
            let shares = paper_shares_from_notional(notional, price);
            assert!(shares > Decimal::ZERO);
            let sim = simulate_copy_fill(
                &book,
                true,
                true,
                false,
                500,
                true,
                price,
                price,
                shares,
                notional,
                d("5"),
                Some(&fee),
            )
            .expect("micro paper buy");
            assert_eq!(sim.fill.notional, notional);
            assert_eq!(
                sim.paper_execution_mode,
                PaperExecutionMode::SyntheticSourcePrice
            );
        }
    }

    #[test]
    fn paper_without_subminimum_flag_rejects_micro_fill() {
        let book: Result<OrderBook, String> = Err("unavailable".into());
        let fee = FeeSchedule {
            fees_enabled: false,
            rate: Decimal::ZERO,
            exponent: 1,
            taker_only: true,
            rebate_rate: None,
            source: "test".into(),
        };
        assert_eq!(
            simulate_copy_fill(
                &book,
                true,
                false,
                false,
                500,
                true,
                d("0.08"),
                d("0.08"),
                d("6.25"),
                d("0.50"),
                d("5"),
                Some(&fee),
            ),
            Err(RejectReason::BelowMinimum)
        );
    }

    #[test]
    fn live_still_enforces_exchange_minimum() {
        let book = Ok(OrderBook {
            bids: vec![],
            asks: vec![BookLevel {
                price: d("0.08"),
                size: d("100"),
            }],
            tick: d("0.01"),
            min_order_size: d("5"),
        });
        let fee = FeeSchedule {
            fees_enabled: false,
            rate: Decimal::ZERO,
            exponent: 1,
            taker_only: true,
            rebate_rate: None,
            source: "test".into(),
        };
        assert_eq!(
            simulate_copy_fill(
                &book,
                false,
                false,
                false,
                500,
                true,
                d("0.08"),
                d("0.08"),
                d("2.5"),
                d("0.20"),
                d("5"),
                Some(&fee),
            ),
            Err(RejectReason::NoLiquidity)
        );
    }

    #[test]
    fn clob_gap_guard_blocks_sell_direction() {
        let book = Ok(OrderBook {
            bids: vec![BookLevel {
                price: d("0.40"),
                size: d("1000"),
            }],
            asks: vec![],
            tick: d("0.01"),
            min_order_size: Decimal::ZERO,
        });
        let fee = FeeSchedule {
            fees_enabled: false,
            rate: Decimal::ZERO,
            exponent: 1,
            taker_only: true,
            rebate_rate: None,
            source: "test".into(),
        };
        let sim = simulate_copy_fill(
            &book,
            true,
            true,
            false,
            100,
            false,
            d("0.50"),
            d("0.45"),
            d("10"),
            d("5"),
            Decimal::ZERO,
            Some(&fee),
        )
        .expect("paper sell synthetic when bids below limit");
        assert_eq!(
            sim.paper_execution_mode,
            PaperExecutionMode::SyntheticSourcePrice
        );
    }

    impl ExitPlan {
        fn unwrap_shares(self) -> Decimal {
            assert!(self.reject.is_none());
            self.shares
        }
    }
}
