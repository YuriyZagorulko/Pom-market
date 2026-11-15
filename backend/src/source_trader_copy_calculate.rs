//! Historical copy-capital calculator for source trader analytics trades.

#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::db::begin_as;
use crate::engine::{plan_exit, truncate_lot};
use crate::source_trader_analytics::resolve_trader_period;

/// Default taker-fee estimate when trade rows lack fee data (0.25% of notional).
pub fn default_fee_bps() -> Decimal {
    Decimal::from(25)
}
/// Minimum modeled copy order (Polymarket-style floor for calculator).
pub fn calc_min_order_usd() -> Decimal {
    Decimal::ONE
}

const K_SEARCH_STEPS: u32 = 12;
const CAPITAL_SEARCH_STEPS: u32 = 8;

fn k_search_steps(trade_count: usize) -> u32 {
    if trade_count > 15_000 {
        4
    } else if trade_count > 5_000 {
        6
    } else {
        K_SEARCH_STEPS
    }
}

fn capital_search_steps(trade_count: usize) -> u32 {
    if trade_count > 15_000 {
        4
    } else if trade_count > 5_000 {
        5
    } else {
        CAPITAL_SEARCH_STEPS
    }
}

/// Full K search is O(steps × trades); skip on very large histories (use applied sizing K + snapshot).
fn skip_chronological_k_search(trade_count: usize) -> bool {
    trade_count > 8_000
}
/// Refuse full-history replays above this many rows (use a shorter period).
pub const MAX_CALCULATE_TRADES: usize = 80_000;

fn lit(s: &str) -> Decimal {
    s.parse().unwrap_or(Decimal::ZERO)
}

fn deserialize_decimal_field<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let raw = serde_json::Value::deserialize(deserializer)?;
    match raw {
        serde_json::Value::Number(n) => n
            .as_f64()
            .and_then(rust_decimal::prelude::FromPrimitive::from_f64)
            .ok_or_else(|| D::Error::custom("invalid decimal number")),
        serde_json::Value::String(s) => s
            .parse::<Decimal>()
            .map_err(|e| D::Error::custom(e.to_string())),
        _ => Err(D::Error::custom("expected number or string for decimal")),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CalculateRequest {
    #[serde(deserialize_with = "deserialize_decimal_field")]
    pub available_capital_usd: Decimal,
    pub period: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    /// Target trade-count coverage 0–1 (e.g. 0.9). Optional — defaults from `target_coverage_percent`.
    pub target_coverage: Option<Decimal>,
  #[serde(default)]
  pub target_coverage_percent: Option<u8>,
    #[serde(default)]
    pub slippage_bps: Option<u32>,
    #[serde(default)]
    pub use_fee_estimate: Option<bool>,
    pub min_order_usd: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkipReasonCount {
    pub reason: String,
    pub count: u64,
    pub leader_notional_usd: Decimal,
}

#[derive(Debug, Clone, Serialize)]
pub struct SimulationMetrics {
    pub copy_coefficient: Decimal,
    pub available_capital_usd: Decimal,
    /// Coverage vs copyable-at-K=1 trades (target basis).
    pub trade_coverage: Decimal,
    pub trade_coverage_all: Decimal,
    /// Valid trades only (invalid rows excluded from denominator).
    pub trade_coverage_valid: Decimal,
    pub trade_coverage_copyable_at_k_one: Decimal,
    pub notional_coverage: Decimal,
    pub copied_trades: u64,
    pub skipped_trades: u64,
    pub source_trades: u64,
    pub valid_trades: u64,
    pub copyable_at_k_one_trades: u64,
    /// Deprecated alias for `valid_trades`.
    pub eligible_trades: u64,
    pub copied_leader_notional_usd: Decimal,
    pub skipped_leader_notional_usd: Decimal,
    pub eligible_leader_notional_usd: Decimal,
    pub peak_capital_utilization_usd: Decimal,
    pub max_simultaneous_exposure_usd: Decimal,
    /// Sum of executed copy BUY notional (at this K), not cumulative turnover without sells.
    pub total_buy_volume_usd: Decimal,
    /// Time-weighted mean deployed cost basis over the simulation period.
    pub average_deployed_capital_usd: Decimal,
    pub ending_cash_usd: Decimal,
    pub estimated_fees_usd: Decimal,
    pub estimated_slippage_usd: Decimal,
    pub skip_reasons: Vec<SkipReasonCount>,
    pub open_positions_end: u64,
    pub open_cost_basis_usd: Decimal,
    pub event_breakdown: SimulationEventBreakdown,
}

#[derive(Debug, Clone, Serialize)]
pub struct TradeExclusions {
    pub source_trades: u64,
    pub invalid_trades: u64,
    pub below_minimum_order: u64,
    pub other_excluded: u64,
    pub copyable_at_k_one: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CoverageLimits {
    pub max_all_trades_at_k_one: Decimal,
    pub max_valid_trades_at_k_one: Decimal,
    pub max_copyable_at_k_one_at_k_one: Decimal,
}

#[derive(Debug, Clone, Serialize)]
pub struct AtAvailableCapital {
    /// Largest K in [0, 1] reaching target at this capital (chronological replay).
    pub max_copy_coefficient_for_target: Decimal,
    /// K used for the coverage snapshot (`recommended` sizing when target is ok).
    pub applied_copy_coefficient: Decimal,
    pub trade_coverage_all: Decimal,
    pub trade_coverage_valid: Decimal,
    pub trade_coverage_copyable_at_k_one: Decimal,
}

#[derive(Debug, Clone, Serialize)]
pub struct MinimumOrderDiagnostics {
    pub smallest_buy_notional_usd: Option<Decimal>,
    pub min_k_to_clear_minimum: Option<Decimal>,
    pub min_k_exceeds_one: bool,
    pub required_capital_at_min_k_for_target: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScenarioRow {
    pub target_coverage: Decimal,
    pub required_capital_usd: Decimal,
    pub copy_coefficient: Decimal,
    pub trade_coverage: Decimal,
    pub notional_coverage: Decimal,
}

#[derive(Debug, Clone, Serialize)]
pub struct CoverageCapitalPoint {
    pub target_coverage: Decimal,
    pub required_capital_usd: Decimal,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SimulationEventBreakdown {
    pub buy_count: u64,
    pub sell_count: u64,
    pub winning_redeems: u64,
    pub losing_redeems: u64,
    pub unresolved_redeems: u64,
    pub skipped_unmatched_redeems: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DataQuality {
    pub trades_in_period: u64,
    pub trades_loaded: u64,
    pub cashflows_in_period: u64,
    pub period_from: DateTime<Utc>,
    pub period_to: DateTime<Utc>,
    pub cache_oldest: Option<DateTime<Utc>>,
    pub cache_newest: Option<DateTime<Utc>>,
    pub trades_backfill_complete: bool,
    pub activity_backfill_complete: bool,
    pub buy_count: u64,
    pub sell_count: u64,
    pub redeem_winning_in_period: u64,
    pub redeem_losing_in_period: u64,
    pub redeem_unresolved_in_period: u64,
    pub warnings: Vec<String>,
    pub fee_mode: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SlippageScenarios {
    pub at_source_price: SimulationMetrics,
    pub with_slippage_reserve: SimulationMetrics,
}

#[derive(Debug, Clone, Serialize)]
pub struct CalculateResponse {
    pub calculated_at: DateTime<Utc>,
    pub cache_fingerprint: String,
    pub target_coverage: Decimal,
    pub available_capital_usd: Decimal,
    pub scaling: CalculateScaling,
    pub data_quality: DataQuality,
    pub event_breakdown: SimulationEventBreakdown,
    pub disclaimer: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct HistTrade {
    pub event_ts: DateTime<Utc>,
    pub side: String,
    pub token_id: String,
    pub notional_usd: Decimal,
    pub price: Decimal,
    pub size_shares: Decimal,
}

#[derive(Debug, Clone, FromRow)]
pub struct HistCashflow {
    pub event_ts: DateTime<Utc>,
    pub token_id: String,
    pub size_shares: Decimal,
    pub payout_usd: Option<Decimal>,
    pub resolution_class: String,
    pub dedup_key: String,
}

#[derive(Debug, Clone)]
enum HistEvent {
    Trade(HistTrade),
    Cashflow(HistCashflow),
}

fn chronological_events(trades: &[HistTrade], cashflows: &[HistCashflow]) -> Vec<HistEvent> {
    let mut out: Vec<HistEvent> = trades
        .iter()
        .cloned()
        .map(HistEvent::Trade)
        .chain(cashflows.iter().cloned().map(HistEvent::Cashflow))
        .collect();
    out.sort_by(|a, b| {
        let ta = match a {
            HistEvent::Trade(t) => t.event_ts,
            HistEvent::Cashflow(c) => c.event_ts,
        };
        let tb = match b {
            HistEvent::Trade(t) => t.event_ts,
            HistEvent::Cashflow(c) => c.event_ts,
        };
        ta.cmp(&tb).then_with(|| match (a, b) {
            (HistEvent::Trade(_), HistEvent::Cashflow(_)) => std::cmp::Ordering::Less,
            (HistEvent::Cashflow(_), HistEvent::Trade(_)) => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        })
    });
    out
}

fn sim_period_bounds_trades_cashflows(
    trades: &[HistTrade],
    cashflows: &[HistCashflow],
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    if period_end > period_start {
        return (period_start, period_end);
    }
    let mut start: Option<DateTime<Utc>> = None;
    let mut end: Option<DateTime<Utc>> = None;
    for t in trades {
        start = Some(start.map_or(t.event_ts, |s| s.min(t.event_ts)));
        end = Some(end.map_or(t.event_ts, |e| e.max(t.event_ts)));
    }
    for c in cashflows {
        start = Some(start.map_or(c.event_ts, |s| s.min(c.event_ts)));
        end = Some(end.map_or(c.event_ts, |e| e.max(c.event_ts)));
    }
    match (start, end) {
        (Some(s), Some(e)) => (s, e),
        _ => {
            let t = Utc::now();
            (t, t)
        }
    }
}

#[derive(Default)]
struct PositionLot {
    open_shares: Decimal,
    source_observed_shares: Decimal,
    cost_basis_usd: Decimal,
    pending_exit_shares: Decimal,
}

#[derive(Default, Serialize)]
struct SkipTally {
    insufficient_capital: u64,
    below_minimum: u64,
    no_position: u64,
    invalid_trade: u64,
    insufficient_shares: u64,
}

impl SkipTally {
    fn record(
        &mut self,
        reason: &str,
        leader_notional: Decimal,
        reasons_out: &mut HashMap<String, (u64, Decimal)>,
    ) {
        match reason {
            "insufficient_capital" => self.insufficient_capital += 1,
            "below_minimum" => self.below_minimum += 1,
            "no_position" => self.no_position += 1,
            "invalid_trade" => self.invalid_trade += 1,
            _ => self.insufficient_shares += 1,
        }
        let e = reasons_out.entry(reason.into()).or_insert((0, Decimal::ZERO));
        e.0 += 1;
        e.1 += leader_notional;
    }

    fn into_vec(map: HashMap<String, (u64, Decimal)>) -> Vec<SkipReasonCount> {
        let mut v: Vec<_> = map
            .into_iter()
            .map(|(reason, (count, leader_notional_usd))| SkipReasonCount {
                reason,
                count,
                leader_notional_usd,
            })
            .collect();
        v.sort_by(|a, b| b.count.cmp(&a.count));
        v
    }
}

pub fn target_from_request(req: &CalculateRequest) -> Decimal {
    if let Some(t) = req.target_coverage {
        return t.clamp(Decimal::ZERO, Decimal::ONE);
    }
    let pct = req.target_coverage_percent.unwrap_or(100).clamp(1, 100);
    Decimal::from(pct) / Decimal::from(100)
}

pub fn is_invalid_trade(t: &HistTrade) -> bool {
    t.notional_usd <= Decimal::ZERO || t.price <= Decimal::ZERO || t.size_shares <= Decimal::ZERO
}

pub fn analyze_trade_universe(trades: &[HistTrade], min_order_usd: Decimal) -> TradeExclusions {
    let source_trades = trades.len() as u64;
    let mut invalid_trades = 0u64;
    let mut below_minimum_order = 0u64;
    for t in trades {
        if is_invalid_trade(t) {
            invalid_trades += 1;
            continue;
        }
        if t.side == "buy" && t.notional_usd < min_order_usd {
            below_minimum_order += 1;
        }
    }
    let valid = source_trades.saturating_sub(invalid_trades);
    let copyable_at_k_one = valid.saturating_sub(below_minimum_order);
    TradeExclusions {
        source_trades,
        invalid_trades,
        below_minimum_order,
        other_excluded: 0,
        copyable_at_k_one,
    }
}

fn coverage_vs_target(m: &SimulationMetrics) -> Decimal {
    m.trade_coverage_copyable_at_k_one
}

fn sim_period_bounds(
    trades: &[HistTrade],
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    if period_end > period_start {
        return (period_start, period_end);
    }
    if let (Some(first), Some(last)) = (trades.first(), trades.last()) {
        return (first.event_ts, last.event_ts);
    }
    let t = Utc::now();
    (t, t)
}

fn empty_reference_metrics() -> ReferenceCapitalMetrics {
    ReferenceCapitalMetrics {
        required_capital_usd: Decimal::ZERO,
        total_buy_volume_usd: Decimal::ZERO,
        peak_deployed_capital_usd: Decimal::ZERO,
        peak_cash_requirement_usd: Decimal::ZERO,
        average_deployed_capital_usd: Decimal::ZERO,
        ending_cash_usd: Decimal::ZERO,
        ending_open_exposure_usd: Decimal::ZERO,
    }
}

fn reference_from_simulation(required: Decimal, m: &SimulationMetrics) -> ReferenceCapitalMetrics {
    ReferenceCapitalMetrics {
        required_capital_usd: required,
        total_buy_volume_usd: m.total_buy_volume_usd,
        peak_deployed_capital_usd: m.max_simultaneous_exposure_usd,
        peak_cash_requirement_usd: m.peak_capital_utilization_usd,
        average_deployed_capital_usd: m.average_deployed_capital_usd,
        ending_cash_usd: m.ending_cash_usd,
        ending_open_exposure_usd: m.open_cost_basis_usd,
    }
}

/// Upper bound for binary search / unbounded K=1 reference replay (peak concurrent need, not buy turnover).
pub fn probe_capital(
    trades: &[HistTrade],
    cashflows: &[HistCashflow],
    slippage_bps: u32,
    fee_bps: Decimal,
    min_order_usd: Decimal,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> Decimal {
    let (p0, p1) =
        sim_period_bounds_trades_cashflows(trades, cashflows, period_start, period_end);
    let buy_sum: Decimal = trades
        .iter()
        .filter(|t| t.side == "buy")
        .map(|t| t.notional_usd)
        .sum();
    let loose = (buy_sum + Decimal::from(50_000)).max(Decimal::from(10_000));
    let m = simulate_trades(
        trades,
        cashflows,
        loose,
        Decimal::ONE,
        slippage_bps,
        fee_bps,
        min_order_usd,
        p0,
        p1,
    );
    let peak = m
        .peak_capital_utilization_usd
        .max(m.max_simultaneous_exposure_usd);
    (peak * lit("1.25") + Decimal::from(500)).max(Decimal::from(100))
}

/// Limits at K = 1.0 with unbounded starting cash.
pub fn coverage_limits_at_k_one(
    trades: &[HistTrade],
    cashflows: &[HistCashflow],
    slippage_bps: u32,
    fee_bps: Decimal,
    min_order_usd: Decimal,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> CoverageLimits {
    if trades.is_empty() && cashflows.is_empty() {
        return CoverageLimits {
            max_all_trades_at_k_one: Decimal::ZERO,
            max_valid_trades_at_k_one: Decimal::ZERO,
            max_copyable_at_k_one_at_k_one: Decimal::ZERO,
        };
    }
    let (p0, p1) =
        sim_period_bounds_trades_cashflows(trades, cashflows, period_start, period_end);
    let m = simulate_trades(
        trades,
        cashflows,
        probe_capital(
            trades,
            cashflows,
            slippage_bps,
            fee_bps,
            min_order_usd,
            period_start,
            period_end,
        ),
        Decimal::ONE,
        slippage_bps,
        fee_bps,
        min_order_usd,
        p0,
        p1,
    );
    CoverageLimits {
        max_all_trades_at_k_one: m.trade_coverage_all,
        max_valid_trades_at_k_one: m.trade_coverage_valid,
        max_copyable_at_k_one_at_k_one: m.trade_coverage_copyable_at_k_one,
    }
}

/// Minimum starting capital so chronological replay at **K = 1.0** reaches `target` trade coverage.
pub fn min_capital_at_k_one(
    trades: &[HistTrade],
    cashflows: &[HistCashflow],
    target: Decimal,
    slippage_bps: u32,
    fee_bps: Decimal,
    min_order_usd: Decimal,
    max_cov_at_k_one: Decimal,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> (Decimal, SimulationMetrics) {
    const K_ONE: Decimal = Decimal::ONE;
    let (p0, p1) =
        sim_period_bounds_trades_cashflows(trades, cashflows, period_start, period_end);
    let mut hi = probe_capital(
        trades,
        cashflows,
        slippage_bps,
        fee_bps,
        min_order_usd,
        period_start,
        period_end,
    );
    let mut lo = Decimal::ZERO;

    if max_cov_at_k_one + lit("0.000001") < target {
        let at_hi =
            simulate_trades(trades, cashflows, hi, K_ONE, slippage_bps, fee_bps, min_order_usd, p0, p1);
        return (Decimal::ZERO, at_hi);
    }

    let steps = capital_search_steps(trades.len());
    for _ in 0..steps {
        let mid = ((lo + hi) / Decimal::TWO).round_dp(2);
        let m = simulate_trades(trades, cashflows, mid, K_ONE, slippage_bps, fee_bps, min_order_usd, p0, p1);
        if coverage_vs_target(&m) + lit("0.000001") >= target {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let capital = hi.round_dp(2);
    let metrics =
        simulate_trades(trades, cashflows, capital, K_ONE, slippage_bps, fee_bps, min_order_usd, p0, p1);
    (capital, metrics)
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CalculateStatus {
    Ok,
    TargetUnreachable,
    NoTrades,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReferenceCapitalMetrics {
    /// Minimum starting cash at K = 1.0 to reach target coverage (chronological replay).
    pub required_capital_usd: Decimal,
    /// Executed copy BUY notional at K = 1.0 with unbounded cash (turnover; cash can be reused).
    pub total_buy_volume_usd: Decimal,
    /// Max simultaneous cost basis in open positions at K = 1.0.
    pub peak_deployed_capital_usd: Decimal,
    /// Peak initial cash tied up (starting capital − cash on hand) at K = 1.0.
    pub peak_cash_requirement_usd: Decimal,
    pub average_deployed_capital_usd: Decimal,
    pub ending_cash_usd: Decimal,
    pub ending_open_exposure_usd: Decimal,
}

#[derive(Debug, Clone, Serialize)]
pub struct CalculateScaling {
    pub status: CalculateStatus,
    /// `required_capital / available_capital` when scaling down; `None` if target unreachable.
    pub capital_divisor: Option<Decimal>,
    /// `available_capital / required_capital` at 1.0x leader sizing semantics; capped at 1 when capital suffices.
    pub recommended_copy_coefficient: Option<Decimal>,
    pub required_capital_usd: Decimal,
    pub reference_at_k_one: ReferenceCapitalMetrics,
    pub event_breakdown: SimulationEventBreakdown,
    /// Target basis: copyable trades at K = 1.0 with unbounded cash.
    pub max_achievable_trade_coverage: Decimal,
    pub user_message: Option<String>,
    pub exclusions: TradeExclusions,
    pub coverage_limits: CoverageLimits,
    pub at_available_capital: AtAvailableCapital,
    pub minimum_order_diagnostics: MinimumOrderDiagnostics,
}

pub fn build_cache_fingerprint(
    backfill_complete: bool,
    cache_newest: Option<DateTime<Utc>>,
    trades_in_period: u64,
) -> String {
    format!(
        "backfill={backfill_complete};newest={};trades={trades_in_period}",
        cache_newest.map(|t| t.timestamp()).unwrap_or(0)
    )
}

pub fn min_capital_for_coefficient(
    trades: &[HistTrade],
    cashflows: &[HistCashflow],
    target: Decimal,
    coefficient: Decimal,
    slippage_bps: u32,
    fee_bps: Decimal,
    min_order_usd: Decimal,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> Decimal {
    let k = coefficient.max(Decimal::ZERO);
    let (p0, p1) =
        sim_period_bounds_trades_cashflows(trades, cashflows, period_start, period_end);
    let mut hi = probe_capital(
        trades,
        cashflows,
        slippage_bps,
        fee_bps,
        min_order_usd,
        period_start,
        period_end,
    ) * k;
    let mut lo = Decimal::ZERO;
    let steps = capital_search_steps(trades.len());
    for _ in 0..steps {
        let mid = ((lo + hi) / Decimal::TWO).round_dp(2);
        let m = simulate_trades(trades, cashflows, mid, k, slippage_bps, fee_bps, min_order_usd, p0, p1);
        if coverage_vs_target(&m) + lit("0.000001") >= target {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi.round_dp(2)
}

fn build_minimum_order_diagnostics(trades: &[HistTrade], min_order_usd: Decimal) -> MinimumOrderDiagnostics {
    let smallest = trades
        .iter()
        .filter(|t| !is_invalid_trade(t) && t.side == "buy")
        .map(|t| t.notional_usd)
        .min();
    let (min_k, exceeds) = match smallest {
        Some(s) if s > Decimal::ZERO => {
            let k = (min_order_usd / s).round_dp(6);
            (Some(k), k > Decimal::ONE)
        }
        _ => (None, false),
    };
    // Do not run capital search at K>1 — too slow on large histories and not used for recommendations.
    let required_at_min_k = None;
    MinimumOrderDiagnostics {
        smallest_buy_notional_usd: smallest,
        min_k_to_clear_minimum: min_k,
        min_k_exceeds_one: exceeds,
        required_capital_at_min_k_for_target: required_at_min_k,
    }
}

fn empty_at_capital() -> AtAvailableCapital {
    AtAvailableCapital {
        max_copy_coefficient_for_target: Decimal::ZERO,
        applied_copy_coefficient: Decimal::ZERO,
        trade_coverage_all: Decimal::ZERO,
        trade_coverage_valid: Decimal::ZERO,
        trade_coverage_copyable_at_k_one: Decimal::ZERO,
    }
}

fn at_capital_snapshot(
    trades: &[HistTrade],
    cashflows: &[HistCashflow],
    available: Decimal,
    max_k: Decimal,
    applied_k: Decimal,
    slippage_bps: u32,
    fee_bps: Decimal,
    min_order_usd: Decimal,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> AtAvailableCapital {
    let (p0, p1) =
        sim_period_bounds_trades_cashflows(trades, cashflows, period_start, period_end);
    let m = simulate_trades(
        trades,
        cashflows,
        available,
        applied_k,
        slippage_bps,
        fee_bps,
        min_order_usd,
        p0,
        p1,
    );
    AtAvailableCapital {
        max_copy_coefficient_for_target: max_k,
        applied_copy_coefficient: applied_k,
        trade_coverage_all: m.trade_coverage_all,
        trade_coverage_valid: m.trade_coverage_valid,
        trade_coverage_copyable_at_k_one: m.trade_coverage_copyable_at_k_one,
    }
}

pub fn compute_scaling(
    trades: &[HistTrade],
    cashflows: &[HistCashflow],
    available_capital: Decimal,
    target: Decimal,
    slippage_bps: u32,
    fee_bps: Decimal,
    min_order_usd: Decimal,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> CalculateScaling {
    let (p0, p1) =
        sim_period_bounds_trades_cashflows(trades, cashflows, period_start, period_end);
    let exclusions = analyze_trade_universe(trades, min_order_usd);
    let probe_cap = probe_capital(
        trades,
        cashflows,
        slippage_bps,
        fee_bps,
        min_order_usd,
        period_start,
        period_end,
    );
    let probe_metrics = simulate_trades(
        trades,
        cashflows,
        probe_cap,
        Decimal::ONE,
        slippage_bps,
        fee_bps,
        min_order_usd,
        p0,
        p1,
    );
    let coverage_limits = CoverageLimits {
        max_all_trades_at_k_one: probe_metrics.trade_coverage_all,
        max_valid_trades_at_k_one: probe_metrics.trade_coverage_valid,
        max_copyable_at_k_one_at_k_one: probe_metrics.trade_coverage_copyable_at_k_one,
    };
    let min_diag = build_minimum_order_diagnostics(trades, min_order_usd);
    let available = available_capital.max(Decimal::ZERO);

    if trades.is_empty() {
        return CalculateScaling {
            status: CalculateStatus::NoTrades,
            capital_divisor: None,
            recommended_copy_coefficient: None,
            required_capital_usd: Decimal::ZERO,
            reference_at_k_one: empty_reference_metrics(),
            event_breakdown: SimulationEventBreakdown::default(),
            max_achievable_trade_coverage: Decimal::ZERO,
            user_message: Some("No trades in the selected period.".into()),
            exclusions,
            coverage_limits,
            at_available_capital: empty_at_capital(),
            minimum_order_diagnostics: min_diag,
        };
    }

  if exclusions.copyable_at_k_one == 0 {
    return CalculateScaling {
      status: CalculateStatus::TargetUnreachable,
      capital_divisor: None,
      recommended_copy_coefficient: None,
      required_capital_usd: Decimal::ZERO,
      reference_at_k_one: reference_from_simulation(Decimal::ZERO, &probe_metrics),
      event_breakdown: probe_metrics.event_breakdown.clone(),
      max_achievable_trade_coverage: Decimal::ZERO,
      user_message: Some(
        "No copyable trades at 1.0× (all rows are invalid or below the minimum order size)."
          .into(),
      ),
      exclusions,
      coverage_limits,
      at_available_capital: empty_at_capital(),
      minimum_order_diagnostics: min_diag,
    };
  }

    let max_cov = probe_metrics.trade_coverage_copyable_at_k_one;
    let tol = lit("0.000001");
    let max_k = if skip_chronological_k_search(trades.len()) {
        Decimal::ZERO
    } else {
        max_coefficient_for_target(
            trades,
            cashflows,
            available,
            target,
            slippage_bps,
            fee_bps,
            min_order_usd,
            period_start,
            period_end,
        )
    };

    if max_cov + tol < target {
        let msg = format!(
            "Target {:.0}% of copyable-at-1.0× trades is not reachable at K ≤ 1: maximum is {:.1}% \
             ({:.1}% of all source trades). Excluded: {} invalid, {} below ${min_order_usd} minimum.",
            (target * Decimal::from(100)).round_dp(0),
            (max_cov * Decimal::from(100)).round_dp(1),
            (coverage_limits.max_all_trades_at_k_one * Decimal::from(100)).round_dp(1),
            exclusions.invalid_trades,
            exclusions.below_minimum_order,
        );
        let at_available_capital = at_capital_snapshot(
            trades,
            cashflows,
            available,
            max_k,
            max_k,
            slippage_bps,
            fee_bps,
            min_order_usd,
            period_start,
            period_end,
        );
        return CalculateScaling {
            status: CalculateStatus::TargetUnreachable,
            capital_divisor: None,
            recommended_copy_coefficient: None,
            required_capital_usd: Decimal::ZERO,
            reference_at_k_one: reference_from_simulation(Decimal::ZERO, &probe_metrics),
            event_breakdown: probe_metrics.event_breakdown.clone(),
            max_achievable_trade_coverage: max_cov.round_dp(6),
            user_message: Some(msg),
            exclusions,
            coverage_limits,
            at_available_capital,
            minimum_order_diagnostics: min_diag,
        };
    }

    let (required, req_metrics) = min_capital_at_k_one(
        trades,
        cashflows,
        target,
        slippage_bps,
        fee_bps,
        min_order_usd,
        max_cov,
        period_start,
        period_end,
    );
    let required = required.max(Decimal::ZERO).round_dp(2);

    if required <= Decimal::ZERO {
        let at_available_capital = at_capital_snapshot(
            trades,
            cashflows,
            available,
            max_k,
            max_k,
            slippage_bps,
            fee_bps,
            min_order_usd,
            period_start,
            period_end,
        );
        return CalculateScaling {
            status: CalculateStatus::TargetUnreachable,
            capital_divisor: None,
            recommended_copy_coefficient: None,
            required_capital_usd: Decimal::ZERO,
            reference_at_k_one: reference_from_simulation(Decimal::ZERO, &probe_metrics),
            event_breakdown: probe_metrics.event_breakdown.clone(),
            max_achievable_trade_coverage: max_cov.round_dp(6),
            user_message: Some(
                "Could not estimate required capital for this target.".into(),
            ),
            exclusions,
            coverage_limits,
            at_available_capital,
            minimum_order_diagnostics: min_diag,
        };
    }

    let mut reference_at_k_one = reference_from_simulation(required, &probe_metrics);
    reference_at_k_one.required_capital_usd = required;
    let _ = req_metrics;

    let coeff = if available >= required {
        Decimal::ONE
    } else {
        (available / required).round_dp(6)
    };
    let divisor = if available > Decimal::ZERO && available < required {
        Some((required / available).round_dp(2))
    } else {
        Some(Decimal::ONE)
    };

    let at_available_capital = at_capital_snapshot(
        trades,
        cashflows,
        available,
        max_k,
        coeff,
        slippage_bps,
        fee_bps,
        min_order_usd,
        period_start,
        period_end,
    );

    CalculateScaling {
        status: CalculateStatus::Ok,
        capital_divisor: divisor,
        recommended_copy_coefficient: Some(coeff),
        required_capital_usd: required,
        reference_at_k_one,
        event_breakdown: req_metrics.event_breakdown.clone(),
        max_achievable_trade_coverage: max_cov.round_dp(6),
        user_message: None,
        exclusions,
        coverage_limits,
        at_available_capital,
        minimum_order_diagnostics: min_diag,
    }
}

pub fn simulate_trades(
    trades: &[HistTrade],
    cashflows: &[HistCashflow],
    capital: Decimal,
    coefficient: Decimal,
    slippage_bps: u32,
    fee_bps: Decimal,
    min_order_usd: Decimal,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> SimulationMetrics {
    let mut cash = capital.max(Decimal::ZERO);
    let mut lots: HashMap<String, PositionLot> = HashMap::new();
    let mut peak_util = Decimal::ZERO;
    let mut max_exposure = Decimal::ZERO;
    let mut copied = 0u64;
    let mut skipped = 0u64;
    let mut copied_notional = Decimal::ZERO;
    let mut buy_volume = Decimal::ZERO;
    let mut skipped_notional = Decimal::ZERO;
    let mut fees = Decimal::ZERO;
    let mut slip_cost = Decimal::ZERO;
    let mut reason_map: HashMap<String, (u64, Decimal)> = HashMap::new();
    let mut _tally = SkipTally::default();
    let k = coefficient.max(Decimal::ZERO);

    let slip_rate = Decimal::from(slippage_bps) / Decimal::from(10_000);
    let fee_rate = fee_bps / Decimal::from(10_000);

    let (avg_start, avg_end) =
        sim_period_bounds_trades_cashflows(trades, cashflows, period_start, period_end);
    let period_secs = (avg_end - avg_start).num_seconds().max(1);
    let mut last_ts = avg_start;
    let mut deployed_area = Decimal::ZERO;
    let mut deployed = Decimal::ZERO;
    let mut breakdown = SimulationEventBreakdown::default();
    let mut seen_cashflow_keys: HashMap<String, ()> = HashMap::new();

    fn accrue_deployed(
        deployed: Decimal,
        last_ts: &mut DateTime<Utc>,
        until: DateTime<Utc>,
        area: &mut Decimal,
    ) {
        if until <= *last_ts {
            return;
        }
        let secs = (until - *last_ts).num_seconds().max(0);
        if secs > 0 {
            *area += deployed * Decimal::from(secs);
            *last_ts = until;
        }
    }

    for ev in chronological_events(trades, cashflows) {
        let event_ts = match &ev {
            HistEvent::Trade(t) => t.event_ts,
            HistEvent::Cashflow(c) => c.event_ts,
        }
        .max(avg_start)
        .min(avg_end);
        accrue_deployed(deployed, &mut last_ts, event_ts, &mut deployed_area);

        if let HistEvent::Cashflow(cf) = &ev {
            if !cf.dedup_key.is_empty() && seen_cashflow_keys.contains_key(&cf.dedup_key) {
                continue;
            }
            if !cf.dedup_key.is_empty() {
                seen_cashflow_keys.insert(cf.dedup_key.clone(), ());
            }
            if cf.resolution_class == "unresolved" || cf.payout_usd.is_none() {
                breakdown.unresolved_redeems += 1;
                continue;
            }
            let token = cf.token_id.clone();
            let plan = {
                let lot = lots.get(&token);
                plan_exit(
                    lot.map(|l| l.open_shares).unwrap_or(Decimal::ZERO),
                    lot.map(|l| l.source_observed_shares).unwrap_or(Decimal::ZERO),
                    cf.size_shares,
                    lot.map(|l| l.pending_exit_shares).unwrap_or(Decimal::ZERO),
                    Decimal::ONE,
                    min_order_usd,
                    true,
                )
            };
            if let Some(reject) = plan.reject {
                skipped += 1;
                _tally.record(reject.as_str(), Decimal::ZERO, &mut reason_map);
                breakdown.skipped_unmatched_redeems += 1;
                continue;
            }
            let shares = plan.shares;
            if shares <= Decimal::ZERO {
                breakdown.skipped_unmatched_redeems += 1;
                continue;
            }
            let payout_total = cf.payout_usd.unwrap_or(Decimal::ZERO);
            let proceeds = if cf.size_shares > Decimal::ZERO {
                (payout_total * shares / cf.size_shares).round_dp(8)
            } else {
                Decimal::ZERO
            };
            if let Some(lot) = lots.get_mut(&token) {
                let sold = shares.min(lot.open_shares);
                let released = if lot.open_shares > Decimal::ZERO {
                    (lot.cost_basis_usd * sold / lot.open_shares).round_dp(8)
                } else {
                    Decimal::ZERO
                };
                lot.open_shares -= sold;
                lot.cost_basis_usd = (lot.cost_basis_usd - released).max(Decimal::ZERO);
                lot.source_observed_shares =
                    (lot.source_observed_shares - cf.size_shares.min(lot.source_observed_shares))
                        .max(Decimal::ZERO);
                lot.pending_exit_shares = plan.pending_exit_shares;
            }
            cash += proceeds;
            copied += 1;
            if cf.resolution_class == "winning" {
                breakdown.winning_redeems += 1;
            } else {
                breakdown.losing_redeems += 1;
            }
            deployed = lots.values().map(|l| l.cost_basis_usd).sum();
            let utilized = (capital - cash).max(Decimal::ZERO);
            if utilized > peak_util {
                peak_util = utilized;
            }
            if deployed > max_exposure {
                max_exposure = deployed;
            }
            continue;
        }

        let HistEvent::Trade(t) = ev else {
            continue;
        };
        if t.notional_usd <= Decimal::ZERO || t.price <= Decimal::ZERO || t.size_shares <= Decimal::ZERO {
            skipped += 1;
            skipped_notional += t.notional_usd;
            _tally.record("invalid_trade", t.notional_usd, &mut reason_map);
            continue;
        }

        let side_buy = t.side == "buy";
        let raw_copy = (t.notional_usd * k).round_dp(8);

        if side_buy {
            if raw_copy < min_order_usd {
                skipped += 1;
                skipped_notional += t.notional_usd;
                _tally.record("below_minimum", t.notional_usd, &mut reason_map);
                continue;
            }
            let slip = (raw_copy * slip_rate).round_dp(8);
            let fee = (raw_copy * fee_rate).round_dp(8);
            let total_cost = raw_copy + slip + fee;
            if cash < total_cost {
                skipped += 1;
                skipped_notional += t.notional_usd;
                _tally.record("insufficient_capital", t.notional_usd, &mut reason_map);
                continue;
            }
            let shares = truncate_lot(raw_copy / t.price);
            if shares <= Decimal::ZERO {
                skipped += 1;
                skipped_notional += t.notional_usd;
                _tally.record("below_minimum", t.notional_usd, &mut reason_map);
                continue;
            }
            cash -= total_cost;
            fees += fee;
            slip_cost += slip;
            let lot = lots.entry(t.token_id.clone()).or_default();
            lot.open_shares += shares;
            lot.source_observed_shares += t.size_shares;
            lot.cost_basis_usd += raw_copy;
            buy_volume += raw_copy;
            copied += 1;
            copied_notional += t.notional_usd;
            breakdown.buy_count += 1;
        } else {
            let token = t.token_id.clone();
            let plan = {
                let lot = lots.get(&token);
                plan_exit(
                    lot.map(|l| l.open_shares).unwrap_or(Decimal::ZERO),
                    lot.map(|l| l.source_observed_shares).unwrap_or(Decimal::ZERO),
                    t.size_shares,
                    lot.map(|l| l.pending_exit_shares).unwrap_or(Decimal::ZERO),
                    t.price,
                    min_order_usd,
                    true,
                )
            };
            if let Some(reject) = plan.reject {
                skipped += 1;
                skipped_notional += t.notional_usd;
                let r = reject.as_str();
                _tally.record(r, t.notional_usd, &mut reason_map);
                if let Some(lot) = lots.get_mut(&token) {
                    lot.pending_exit_shares = plan.pending_exit_shares;
                }
                continue;
            }
            let shares = plan.shares;
            let gross = (shares * t.price).round_dp(8);
            let slip = (gross * slip_rate).round_dp(8);
            let fee = (gross * fee_rate).round_dp(8);
            let proceeds = (gross - slip - fee).max(Decimal::ZERO);
            if let Some(lot) = lots.get_mut(&token) {
                let sold = shares.min(lot.open_shares);
                let released = if lot.open_shares > Decimal::ZERO {
                    (lot.cost_basis_usd * sold / lot.open_shares).round_dp(8)
                } else {
                    Decimal::ZERO
                };
                lot.open_shares -= sold;
                lot.cost_basis_usd = (lot.cost_basis_usd - released).max(Decimal::ZERO);
                lot.source_observed_shares =
                    (lot.source_observed_shares - t.size_shares).max(Decimal::ZERO);
                lot.pending_exit_shares = plan.pending_exit_shares;
            }
            cash += proceeds;
            fees += fee;
            slip_cost += slip;
            copied += 1;
            copied_notional += t.notional_usd;
            breakdown.sell_count += 1;
        }

        deployed = lots.values().map(|l| l.cost_basis_usd).sum();
        let utilized = (capital - cash).max(Decimal::ZERO);
        if utilized > peak_util {
            peak_util = utilized;
        }
        if deployed > max_exposure {
            max_exposure = deployed;
        }
    }

    accrue_deployed(deployed, &mut last_ts, avg_end, &mut deployed_area);
    let average_deployed = (deployed_area / Decimal::from(period_secs)).round_dp(2);

    let universe = analyze_trade_universe(trades, min_order_usd);
    let eligible_notional: Decimal = trades.iter().map(|t| t.notional_usd).sum();
    let trade_cov_all = if universe.source_trades > 0 {
        Decimal::from(copied) / Decimal::from(universe.source_trades)
    } else {
        Decimal::ZERO
    };
    let valid = universe.source_trades.saturating_sub(universe.invalid_trades);
    let trade_cov_valid = if valid > 0 {
        Decimal::from(copied) / Decimal::from(valid)
    } else {
        Decimal::ZERO
    };
    let trade_cov_copyable = if universe.copyable_at_k_one > 0 {
        Decimal::from(copied) / Decimal::from(universe.copyable_at_k_one)
    } else {
        Decimal::ZERO
    };
    let notional_cov = if eligible_notional > Decimal::ZERO {
        copied_notional / eligible_notional
    } else {
        Decimal::ZERO
    };
    let open_positions_end = lots.values().filter(|l| l.open_shares > Decimal::ZERO).count() as u64;
    let open_cost: Decimal = lots.values().map(|l| l.cost_basis_usd).sum();

    SimulationMetrics {
        copy_coefficient: k,
        available_capital_usd: capital,
        trade_coverage: trade_cov_copyable.round_dp(6),
        trade_coverage_all: trade_cov_all.round_dp(6),
        trade_coverage_valid: trade_cov_valid.round_dp(6),
        trade_coverage_copyable_at_k_one: trade_cov_copyable.round_dp(6),
        notional_coverage: notional_cov.round_dp(6),
        copied_trades: copied,
        skipped_trades: skipped,
        source_trades: universe.source_trades,
        valid_trades: valid,
        copyable_at_k_one_trades: universe.copyable_at_k_one,
        eligible_trades: valid,
        copied_leader_notional_usd: copied_notional.round_dp(2),
        skipped_leader_notional_usd: skipped_notional.round_dp(2),
        eligible_leader_notional_usd: eligible_notional.round_dp(2),
        peak_capital_utilization_usd: peak_util.round_dp(2),
        max_simultaneous_exposure_usd: max_exposure.round_dp(2),
        total_buy_volume_usd: buy_volume.round_dp(2),
        average_deployed_capital_usd: average_deployed,
        ending_cash_usd: cash.round_dp(2),
        estimated_fees_usd: fees.round_dp(2),
        estimated_slippage_usd: slip_cost.round_dp(2),
        skip_reasons: SkipTally::into_vec(reason_map),
        open_positions_end,
        open_cost_basis_usd: open_cost.round_dp(2),
        event_breakdown: breakdown,
    }
}

/// Largest K in [0,1] such that trade coverage ≥ `target` with fixed capital.
pub fn max_coefficient_for_target(
    trades: &[HistTrade],
    cashflows: &[HistCashflow],
    capital: Decimal,
    target: Decimal,
    slippage_bps: u32,
    fee_bps: Decimal,
    min_order_usd: Decimal,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> Decimal {
    let (p0, p1) =
        sim_period_bounds_trades_cashflows(trades, cashflows, period_start, period_end);
    let mut lo = Decimal::ZERO;
    let mut hi = Decimal::ONE;
    let steps = k_search_steps(trades.len());
    for _ in 0..steps {
        let mid = ((lo + hi) / Decimal::TWO).round_dp(8);
        let m = simulate_trades(
            trades,
            cashflows,
            capital,
            mid,
            slippage_bps,
            fee_bps,
            min_order_usd,
            p0,
            p1,
        );
        if coverage_vs_target(&m) >= target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo.round_dp(6)
}

pub fn min_capital_for_target(
    trades: &[HistTrade],
    cashflows: &[HistCashflow],
    target: Decimal,
    slippage_bps: u32,
    fee_bps: Decimal,
    min_order_usd: Decimal,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> (Decimal, Decimal, SimulationMetrics) {
    let (p0, p1) =
        sim_period_bounds_trades_cashflows(trades, cashflows, period_start, period_end);
    let mut hi = probe_capital(
        trades,
        cashflows,
        slippage_bps,
        fee_bps,
        min_order_usd,
        period_start,
        period_end,
    );
    let mut lo = Decimal::ZERO;
    let steps = capital_search_steps(trades.len());
    for _ in 0..steps {
        let mid = ((lo + hi) / Decimal::TWO).round_dp(2);
        let k = max_coefficient_for_target(
            trades,
            cashflows,
            mid,
            target,
            slippage_bps,
            fee_bps,
            min_order_usd,
            period_start,
            period_end,
        );
        let metrics = simulate_trades(
            trades,
            cashflows,
            mid,
            k,
            slippage_bps,
            fee_bps,
            min_order_usd,
            p0,
            p1,
        );
        if coverage_vs_target(&metrics) + lit("0.0001") >= target {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let capital = hi.round_dp(2);
    let k = max_coefficient_for_target(
        trades,
        cashflows,
        capital,
        target,
        slippage_bps,
        fee_bps,
        min_order_usd,
        period_start,
        period_end,
    );
    let metrics = simulate_trades(
        trades,
        cashflows,
        capital,
        k,
        slippage_bps,
        fee_bps,
        min_order_usd,
        p0,
        p1,
    );
    (capital, k, metrics)
}

pub fn build_scenarios(
    trades: &[HistTrade],
    cashflows: &[HistCashflow],
    slippage_bps: u32,
    fee_bps: Decimal,
    min_order_usd: Decimal,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> Vec<ScenarioRow> {
    [lit("0.90"), lit("0.95"), lit("0.99"), lit("1.0")]
        .iter()
        .map(|&target| {
            let (cap, k, m) = min_capital_for_target(
                trades,
                cashflows,
                target,
                slippage_bps,
                fee_bps,
                min_order_usd,
                period_start,
                period_end,
            );
            ScenarioRow {
                target_coverage: target,
                required_capital_usd: cap,
                copy_coefficient: k,
                trade_coverage: m.trade_coverage,
                notional_coverage: m.notional_coverage,
            }
        })
        .collect()
}

/// Chart points: scenario table (90–100%) plus two lighter anchors when affordable.
pub fn coverage_capital_curve_from_scenarios(
    trades: &[HistTrade],
    cashflows: &[HistCashflow],
    scenarios: &[ScenarioRow],
    slippage_bps: u32,
    fee_bps: Decimal,
    min_order_usd: Decimal,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> Vec<CoverageCapitalPoint> {
    let mut pts: Vec<CoverageCapitalPoint> = scenarios
        .iter()
        .map(|s| CoverageCapitalPoint {
            target_coverage: s.target_coverage,
            required_capital_usd: s.required_capital_usd,
        })
        .collect();
    for pct in [50i32, 70] {
        let target = Decimal::from(pct) / Decimal::from(100);
        let (cap, _, _) = min_capital_for_target(
            trades,
            cashflows,
            target,
            slippage_bps,
            fee_bps,
            min_order_usd,
            period_start,
            period_end,
        );
        pts.push(CoverageCapitalPoint {
            target_coverage: target,
            required_capital_usd: cap,
        });
    }
    pts.sort_by(|a, b| {
        a.target_coverage
            .partial_cmp(&b.target_coverage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    pts
}

pub fn compute_calculate_response(
    trades: Vec<HistTrade>,
    cashflows: Vec<HistCashflow>,
    trades_backfill_complete: bool,
    activity_backfill_complete: bool,
    cache_bounds: Option<(DateTime<Utc>, DateTime<Utc>)>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    capital: Decimal,
    target: Decimal,
    slip_apply: u32,
    fee_bps: Decimal,
    fee_mode: String,
    min_order: Decimal,
) -> CalculateResponse {
    let scaling = compute_scaling(
        &trades,
        &cashflows,
        capital,
        target,
        slip_apply,
        fee_bps,
        min_order,
        start,
        end,
    );

    let (buy_count, sell_count) = trades.iter().fold((0u64, 0u64), |(b, s), t| {
        if t.side == "buy" {
            (b + 1, s)
        } else {
            (b, s + 1)
        }
    });
    let (redeem_win, redeem_lose, redeem_unresolved) =
        cashflows.iter().fold((0u64, 0u64, 0u64), |(w, l, u), c| {
            match c.resolution_class.as_str() {
                "winning" => (w + 1, l, u),
                "losing" => (w, l + 1, u),
                _ => (w, l, u + 1),
            }
        });

    let mut warnings = Vec::new();
    if !trades_backfill_complete {
        warnings
            .push("Trade history backfill is incomplete — results use cached pages only.".into());
    }
    if !activity_backfill_complete {
        warnings.push(
            "Activity (REDEEM) backfill is incomplete — resolution payouts may be missing.".into(),
        );
    }
    let has_redeem = !cashflows.is_empty();
    if sell_count == 0 && buy_count > 0 && !has_redeem {
        warnings.push(
            "No SELL or REDEEM events in cache — capital reuse cannot be modeled; Required Capital reflects stacked open BUYs."
                .into(),
        );
    }
    if redeem_unresolved > 0 {
        warnings.push(format!(
            "{redeem_unresolved} REDEEM row(s) lack payout data — those positions stay open in the simulator."
        ));
    }
    if let Some((oldest, newest)) = cache_bounds {
        if oldest > start {
            warnings.push(format!(
                "Cache starts at {} (later than period start).",
                oldest.to_rfc3339()
            ));
        }
        if newest < end {
            warnings.push(format!(
                "Cache ends at {} (earlier than period end).",
                newest.to_rfc3339()
            ));
        }
    }
    let resolution_incomplete = redeem_unresolved > 0 || !activity_backfill_complete;
    let data_incomplete =
        !trades_backfill_complete || (!has_redeem && sell_count == 0) || resolution_incomplete;
    if data_incomplete {
        warnings.push(
            "Partial history — treat required capital and coverage as estimates, not exact forecasts."
                .into(),
        );
    }

    let cache_fingerprint = build_cache_fingerprint(
        trades_backfill_complete,
        cache_bounds.map(|b| b.1),
        trades.len() as u64,
    );

    let data_quality = DataQuality {
        trades_in_period: trades.len() as u64,
        trades_loaded: trades.len() as u64,
        cashflows_in_period: cashflows.len() as u64,
        period_from: start,
        period_to: end,
        cache_oldest: cache_bounds.map(|b| b.0),
        cache_newest: cache_bounds.map(|b| b.1),
        trades_backfill_complete,
        activity_backfill_complete,
        buy_count,
        sell_count,
        redeem_winning_in_period: redeem_win,
        redeem_losing_in_period: redeem_lose,
        redeem_unresolved_in_period: redeem_unresolved,
        warnings,
        fee_mode,
    };

    let event_breakdown = scaling.event_breakdown.clone();

    CalculateResponse {
        calculated_at: Utc::now(),
        cache_fingerprint,
        target_coverage: target,
        available_capital_usd: capital,
        scaling,
        data_quality,
        event_breakdown,
        disclaimer: if data_incomplete {
            "Partial cached history (missing exits, REDEEM pages, or unresolved payouts). Simulation uses available rows only."
                .into()
        } else {
            "Historical simulation on cached leader trades and REDEEM activity — not a guarantee of future fills.".into()
        },
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CalculateCacheMeta {
    pub cache_fingerprint: String,
    pub backfill_complete: bool,
    pub cache_newest: Option<DateTime<Utc>>,
    pub trades_in_period: u64,
}

pub async fn fetch_calculate_cache_meta(
    pool: &PgPool,
    user_id: Uuid,
    source_trader_id: Uuid,
    period: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> anyhow::Result<CalculateCacheMeta> {
    let (start, end, _) =
        resolve_trader_period(period, from, to)?;

    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM source_traders WHERE id = $1 AND user_id = $2)",
    )
    .bind(source_trader_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !exists {
        anyhow::bail!("trader not found");
    }

    let backfill_complete: bool = sqlx::query_scalar(
        "SELECT COALESCE(trades_backfill_complete, false) FROM source_trader_analytics_sync
         WHERE source_trader_id = $1 AND user_id = $2",
    )
    .bind(source_trader_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(false);

    let trades_in_period: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM source_trader_analytics_trades
         WHERE source_trader_id = $1 AND event_ts >= $2 AND event_ts <= $3",
    )
    .bind(source_trader_id)
    .bind(start)
    .bind(end)
    .fetch_one(&mut *tx)
    .await?;

    let cache_newest: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT MAX(event_ts) FROM source_trader_analytics_trades
         WHERE source_trader_id = $1 AND event_ts >= $2 AND event_ts <= $3",
    )
    .bind(source_trader_id)
    .bind(start)
    .bind(end)
    .fetch_optional(&mut *tx)
    .await?;

    tx.commit().await?;

    let trades_in_period = trades_in_period.max(0) as u64;
    Ok(CalculateCacheMeta {
        cache_fingerprint: build_cache_fingerprint(backfill_complete, cache_newest, trades_in_period),
        backfill_complete,
        cache_newest,
        trades_in_period,
    })
}

pub async fn run_calculate(
    pool: &PgPool,
    user_id: Uuid,
    source_trader_id: Uuid,
    req: &CalculateRequest,
) -> anyhow::Result<CalculateResponse> {
    let (start, end, _label) =
        resolve_trader_period(req.period.as_deref(), req.from.as_deref(), req.to.as_deref())?;
    let target = target_from_request(req);
    let slip_apply = req.slippage_bps.unwrap_or(500);
    let fee_bps = if req.use_fee_estimate.unwrap_or(true) {
        default_fee_bps()
    } else {
        Decimal::ZERO
    };
    let fee_mode = if fee_bps > Decimal::ZERO {
        format!("estimated_{}bps_per_notional", fee_bps)
    } else {
        "none".into()
    };
    let min_order = req
        .min_order_usd
        .unwrap_or_else(calc_min_order_usd)
        .max(Decimal::ZERO);

    let mut tx = begin_as(pool, Some(user_id), "user").await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM source_traders WHERE id = $1 AND user_id = $2)",
    )
    .bind(source_trader_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !exists {
        anyhow::bail!("trader not found");
    }

    let trade_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM source_trader_analytics_trades
         WHERE source_trader_id = $1 AND event_ts >= $2 AND event_ts <= $3",
    )
    .bind(source_trader_id)
    .bind(start)
    .bind(end)
    .fetch_one(&mut *tx)
    .await?;
    if trade_count as usize > MAX_CALCULATE_TRADES {
        anyhow::bail!(
            "too many trades in period ({trade_count}); use 90D or a shorter custom range (max {MAX_CALCULATE_TRADES})"
        );
    }

    let trades_backfill_complete: bool = sqlx::query_scalar(
        "SELECT COALESCE(trades_backfill_complete, false) FROM source_trader_analytics_sync
         WHERE source_trader_id = $1 AND user_id = $2",
    )
    .bind(source_trader_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(false);

    let activity_backfill_complete: bool = sqlx::query_scalar(
        "SELECT COALESCE(activity_backfill_complete, false) FROM source_trader_analytics_sync
         WHERE source_trader_id = $1 AND user_id = $2",
    )
    .bind(source_trader_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(false);

    let cache_bounds: Option<(DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT MIN(event_ts), MAX(event_ts) FROM source_trader_analytics_trades
         WHERE source_trader_id = $1",
    )
    .bind(source_trader_id)
    .fetch_optional(&mut *tx)
    .await?;

    let trades: Vec<HistTrade> = sqlx::query_as(
        "SELECT event_ts, side, COALESCE(token_id, '') AS token_id, notional_usd, price, size_shares
         FROM source_trader_analytics_trades
         WHERE source_trader_id = $1 AND event_ts >= $2 AND event_ts <= $3
         ORDER BY event_ts ASC, id ASC",
    )
    .bind(source_trader_id)
    .bind(start)
    .bind(end)
    .fetch_all(&mut *tx)
    .await?;

    let cashflows: Vec<HistCashflow> = sqlx::query_as(
        "SELECT event_ts, token_id, size_shares, payout_usd, resolution_class, dedup_key
         FROM source_trader_analytics_cashflows
         WHERE source_trader_id = $1 AND event_ts >= $2 AND event_ts <= $3
         ORDER BY event_ts ASC, id ASC",
    )
    .bind(source_trader_id)
    .bind(start)
    .bind(end)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;

    let capital = req.available_capital_usd.max(Decimal::ZERO);
  let response = tokio::task::spawn_blocking(move || {
        compute_calculate_response(
            trades,
            cashflows,
            trades_backfill_complete,
            activity_backfill_complete,
            cache_bounds,
            start,
            end,
            capital,
            target,
            slip_apply,
            fee_bps,
            fee_mode,
            min_order,
        )
    })
    .await
    .map_err(|e| anyhow::anyhow!("calculate worker: {e}"))?;

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(s: &str) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap() + chrono::Duration::seconds(s.parse().unwrap())
    }

    fn mk(side: &str, token: &str, usd: Decimal, price: Decimal, shares: Decimal, sec: i64) -> HistTrade {
        HistTrade {
            event_ts: ts(&sec.to_string()),
            side: side.into(),
            token_id: token.into(),
            notional_usd: usd,
            price,
            size_shares: shares,
        }
    }

    fn period(trades: &[HistTrade], cashflows: &[HistCashflow]) -> (DateTime<Utc>, DateTime<Utc>) {
        let sentinel = Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap();
        sim_period_bounds_trades_cashflows(trades, cashflows, sentinel, sentinel)
    }

    fn mk_cf(
        token: &str,
        shares: Decimal,
        payout: Option<Decimal>,
        class: &str,
        sec: i64,
        dedup: &str,
    ) -> HistCashflow {
        HistCashflow {
            event_ts: ts(&sec.to_string()),
            token_id: token.into(),
            size_shares: shares,
            payout_usd: payout,
            resolution_class: class.into(),
            dedup_key: dedup.into(),
        }
    }

    const NO_CF: &[HistCashflow] = &[];

    fn sim(
        trades: &[HistTrade],
        cashflows: &[HistCashflow],
        capital: Decimal,
        k: Decimal,
        slip: u32,
        fee: Decimal,
        min: Decimal,
    ) -> SimulationMetrics {
        let (p0, p1) = period(trades, cashflows);
        simulate_trades(trades, cashflows, capital, k, slip, fee, min, p0, p1)
    }

    fn scale(
        trades: &[HistTrade],
        cashflows: &[HistCashflow],
        available: Decimal,
        target: Decimal,
    ) -> CalculateScaling {
        let (p0, p1) = period(trades, cashflows);
        compute_scaling(
            trades,
            cashflows,
            available,
            target,
            0,
            Decimal::ZERO,
            Decimal::ONE,
            p0,
            p1,
        )
    }

    #[test]
    fn buy_buy_sell_releases_capital() {
        let trades = vec![
            mk("buy", "t1", lit("100"), lit("0.5"), lit("200"), 1),
            mk("buy", "t1", lit("50"), lit("0.5"), lit("100"), 2),
            mk("sell", "t1", lit("75"), lit("0.5"), lit("150"), 3),
        ];
        let m = sim(&trades, NO_CF, lit("1000"), lit("0.1"), 0, Decimal::ZERO, Decimal::ONE);
        assert!(m.copied_trades >= 2);
        assert!(m.peak_capital_utilization_usd > Decimal::ZERO);
        assert!(m.trade_coverage > Decimal::ZERO);
    }

    #[test]
    fn insufficient_capital_skips() {
        let trades = vec![mk("buy", "t1", lit("500"), lit("0.5"), lit("1000"), 1)];
        let m = sim(&trades, NO_CF, lit("10"), lit("0.5"), 0, Decimal::ZERO, Decimal::ONE);
        assert_eq!(m.copied_trades, 0);
        assert_eq!(m.skipped_trades, 1);
        assert!(m
            .skip_reasons
            .iter()
            .any(|r| r.reason == "insufficient_capital"));
    }

    #[test]
    fn trade_vs_notional_coverage_differ() {
        let trades = vec![
            mk("buy", "t1", lit("10"), lit("0.5"), lit("20"), 1),
            mk("buy", "t2", lit("1000"), lit("0.5"), lit("2000"), 2),
        ];
        let m = sim(&trades, NO_CF, lit("50"), Decimal::ONE, 0, Decimal::ZERO, Decimal::ONE);
        assert!(m.trade_coverage != m.notional_coverage || m.copied_trades == 1);
    }

    #[test]
    fn max_k_monotonic_with_capital() {
        let trades = vec![
            mk("buy", "t1", lit("100"), lit("0.5"), lit("200"), 1),
            mk("buy", "t2", lit("100"), lit("0.5"), lit("200"), 2),
        ];
        let (p0, p1) = period(&trades, NO_CF);
        let k_small = max_coefficient_for_target(
            &trades,
            NO_CF,
            lit("5"),
            Decimal::ONE,
            0,
            Decimal::ZERO,
            Decimal::ONE,
            p0,
            p1,
        );
        let k_large = max_coefficient_for_target(
            &trades,
            NO_CF,
            lit("500"),
            Decimal::ONE,
            0,
            Decimal::ZERO,
            Decimal::ONE,
            p0,
            p1,
        );
        assert!(k_large >= k_small);
    }

    #[test]
    fn no_lookahead_uses_running_cash() {
        let trades = vec![
            mk("buy", "t1", lit("400"), lit("0.5"), lit("800"), 1),
            mk("buy", "t2", lit("400"), lit("0.5"), lit("800"), 2),
        ];
        let m = sim(&trades, NO_CF, lit("250"), lit("0.5"), 0, Decimal::ZERO, Decimal::ONE);
        assert_eq!(m.copied_trades, 1);
    }

    #[test]
    fn slippage_reduces_coverage() {
        let trades = vec![mk("buy", "t1", lit("100"), lit("0.5"), lit("200"), 1)];
        let plain = sim(&trades, NO_CF, lit("100"), lit("0.5"), 0, Decimal::ZERO, Decimal::ONE);
        let slip = sim(&trades, NO_CF, lit("100"), lit("0.5"), 500, Decimal::ZERO, Decimal::ONE);
        assert!(slip.estimated_slippage_usd > Decimal::ZERO);
        assert!(slip.copied_trades <= plain.copied_trades);
    }

    #[test]
    fn multiple_markets_concurrent_exposure() {
        let trades = vec![
            mk("buy", "t1", lit("100"), lit("0.5"), lit("200"), 1),
            mk("buy", "t2", lit("100"), lit("0.5"), lit("200"), 2),
            mk("buy", "t3", lit("100"), lit("0.5"), lit("200"), 3),
        ];
        let m = sim(&trades, NO_CF, lit("150"), Decimal::ONE, 0, Decimal::ZERO, Decimal::ONE);
        assert_eq!(m.copied_trades, 1);
        assert!(m.max_simultaneous_exposure_usd > Decimal::ZERO);
    }

    #[test]
    fn partial_sell_fraction() {
        let trades = vec![
            mk("buy", "t1", lit("200"), lit("0.5"), lit("400"), 1),
            mk("sell", "t1", lit("100"), lit("0.5"), lit("200"), 2),
        ];
        let m = sim(&trades, NO_CF, lit("500"), lit("0.5"), 0, Decimal::ZERO, Decimal::ONE);
        assert_eq!(m.copied_trades, 2);
        assert!(m.peak_capital_utilization_usd > Decimal::ZERO);
    }

    #[test]
  #[ignore = "local perf smoke — not CI"]
  fn perf_three_thousand_trades_completes_under_budget() {
    use std::time::Instant;
    let mut trades = Vec::new();
    for i in 0..3000 {
      let token = if i % 2 == 0 { "t1" } else { "t2" };
      trades.push(mk("buy", token, lit("50"), lit("0.5"), lit("100"), i as i64));
    }
    let t0 = Instant::now();
    let _ = compute_calculate_response(
      trades,
      vec![],
      true,
      true,
      None,
      Utc::now() - chrono::Duration::days(30),
      Utc::now(),
      lit("8000"),
      lit("0.9"),
      500,
      Decimal::ZERO,
      "none".into(),
      Decimal::ONE,
    );
    assert!(
      t0.elapsed().as_secs() < 30,
      "calculate should finish within 30s for ~3k trades, took {:?}",
      t0.elapsed()
    );
  }

    #[test]
    fn min_capital_k_one_hundred_percent() {
        let trades = vec![
            mk("buy", "t1", lit("50"), lit("0.5"), lit("100"), 1),
            mk("sell", "t1", lit("25"), lit("0.5"), lit("50"), 2),
        ];
        let (p0, p1) = period(&trades, NO_CF);
        let lim = coverage_limits_at_k_one(&trades, NO_CF, 0, Decimal::ZERO, Decimal::ONE, p0, p1);
        let (cap, m) = min_capital_at_k_one(
            &trades,
            NO_CF,
            Decimal::ONE,
            0,
            Decimal::ZERO,
            Decimal::ONE,
            lim.max_copyable_at_k_one_at_k_one,
            p0,
            p1,
        );
        assert!(cap > Decimal::ZERO);
        assert!(m.trade_coverage >= lit("0.99"));
        assert_eq!(m.copy_coefficient, Decimal::ONE);
    }

    #[test]
    fn target_coverage_defaults_to_hundred_percent() {
        let req = CalculateRequest {
            available_capital_usd: Decimal::ONE,
            period: None,
            from: None,
            to: None,
            target_coverage: None,
            target_coverage_percent: None,
            slippage_bps: None,
            use_fee_estimate: None,
            min_order_usd: None,
        };
        assert_eq!(target_from_request(&req), Decimal::ONE);
    }

    #[test]
    fn scaling_divisor_matches_available_over_required() {
        let trades = vec![
            mk("buy", "t1", lit("5000"), lit("0.5"), lit("10000"), 1),
            mk("buy", "t2", lit("5000"), lit("0.5"), lit("10000"), 2),
            mk("buy", "t3", lit("5000"), lit("0.5"), lit("10000"), 3),
        ];
        let available = lit("8000");
        let s = scale(&trades, NO_CF, available, Decimal::ONE);
        assert_eq!(s.status, CalculateStatus::Ok);
        let req = s.required_capital_usd;
        let coeff = s.recommended_copy_coefficient.unwrap();
        let div = s.capital_divisor.unwrap();
        assert!(req > available);
        assert_eq!(coeff, (available / req).round_dp(6));
        assert_eq!(div, (req / available).round_dp(2));
    }

    #[test]
    fn scaling_no_divisor_when_capital_suffices() {
        let trades = vec![mk("buy", "t1", lit("50"), lit("0.5"), lit("100"), 1)];
        let s = scale(&trades, NO_CF, lit("10000"), Decimal::ONE);
        assert_eq!(s.status, CalculateStatus::Ok);
        assert_eq!(s.recommended_copy_coefficient.unwrap(), Decimal::ONE);
        assert_eq!(s.capital_divisor.unwrap(), Decimal::ONE);
    }

    #[test]
    fn unreachable_when_every_buy_below_minimum() {
        let trades = vec![
            mk("buy", "t1", lit("0.25"), lit("0.5"), lit("0.5"), 1),
            mk("buy", "t2", lit("0.40"), lit("0.5"), lit("0.8"), 2),
        ];
        let s = scale(&trades, NO_CF, lit("8000"), Decimal::ONE);
        assert_eq!(s.status, CalculateStatus::TargetUnreachable);
        assert!(s.capital_divisor.is_none());
        assert!(s.recommended_copy_coefficient.is_none());
        assert!(s.max_achievable_trade_coverage < Decimal::ONE);
    }

    #[test]
    fn custom_target_coverage_percent() {
        let req = CalculateRequest {
            available_capital_usd: Decimal::ONE,
            period: None,
            from: None,
            to: None,
            target_coverage: None,
            target_coverage_percent: Some(75),
            slippage_bps: None,
            use_fee_estimate: None,
            min_order_usd: None,
        };
        assert_eq!(target_from_request(&req), lit("0.75"));
    }

    #[test]
    fn invalid_trades_do_not_block_target_on_copyable_set() {
        let trades = vec![
            mk("buy", "t1", lit("0"), lit("0"), lit("0"), 0),
            mk("buy", "t2", lit("50"), lit("0.5"), lit("100"), 1),
        ];
        let s = scale(&trades, NO_CF, lit("10000"), Decimal::ONE);
        assert_eq!(s.status, CalculateStatus::Ok);
        assert_eq!(s.exclusions.invalid_trades, 1);
        assert_eq!(s.exclusions.below_minimum_order, 0);
        assert_eq!(s.exclusions.copyable_at_k_one, 1);
    }

    #[test]
    fn below_minimum_excluded_from_copyable_counts() {
        let trades = vec![
            mk("buy", "t1", lit("0.50"), lit("0.5"), lit("1"), 1),
            mk("buy", "t2", lit("50"), lit("0.5"), lit("100"), 2),
        ];
        let uni = analyze_trade_universe(&trades, Decimal::ONE);
        assert_eq!(uni.invalid_trades, 0);
        assert_eq!(uni.below_minimum_order, 1);
        assert_eq!(uni.copyable_at_k_one, 1);
        let (p0, p1) = period(&trades, NO_CF);
        let limits = coverage_limits_at_k_one(&trades, NO_CF, 0, Decimal::ZERO, Decimal::ONE, p0, p1);
        assert_eq!(limits.max_copyable_at_k_one_at_k_one, Decimal::ONE);
        assert!(limits.max_all_trades_at_k_one < Decimal::ONE);
    }

    #[test]
    fn available_capital_changes_max_k_for_target() {
        let trades = vec![
            mk("buy", "t1", lit("1000"), lit("0.5"), lit("2000"), 1),
            mk("buy", "t2", lit("1000"), lit("0.5"), lit("2000"), 2),
        ];
        let s_low = scale(&trades, NO_CF, lit("500"), Decimal::ONE);
        let s_high = scale(&trades, NO_CF, lit("5000"), Decimal::ONE);
        assert!(
            s_high.at_available_capital.max_copy_coefficient_for_target
                >= s_low.at_available_capital.max_copy_coefficient_for_target
        );
        assert!(s_low.at_available_capital.trade_coverage_copyable_at_k_one
            <= s_high.at_available_capital.trade_coverage_copyable_at_k_one);
    }

    #[test]
    fn min_k_diagnostics_when_micro_buys_present() {
        let trades = vec![
            mk("buy", "t1", lit("0.50"), lit("0.5"), lit("1"), 1),
            mk("buy", "t2", lit("80"), lit("0.5"), lit("160"), 2),
        ];
        let s = scale(&trades, NO_CF, lit("8000"), Decimal::ONE);
        assert_eq!(s.status, CalculateStatus::Ok);
        let diag = &s.minimum_order_diagnostics;
        assert_eq!(diag.smallest_buy_notional_usd, Some(lit("0.50")));
        assert!(diag.min_k_to_clear_minimum.unwrap() > Decimal::ONE);
        assert!(diag.min_k_exceeds_one);
    }

    #[test]
    fn case_a_repeated_buy_sell_reuses_capital() {
        let mut trades = Vec::new();
        for i in 0..4 {
            let sec = i * 2;
            trades.push(mk("buy", "t1", lit("4000"), lit("0.5"), lit("8000"), sec));
            trades.push(mk("sell", "t1", lit("4000"), lit("0.5"), lit("8000"), sec + 1));
        }
        let (p0, p1) = period(&trades, NO_CF);
        let ref_m = sim(
            &trades,
            NO_CF,
            Decimal::from(1_000_000),
            Decimal::ONE,
            0,
            Decimal::ZERO,
            Decimal::ONE,
        );
        assert_eq!(ref_m.total_buy_volume_usd, lit("16000"));
        assert_eq!(ref_m.max_simultaneous_exposure_usd, lit("4000"));
        let lim = coverage_limits_at_k_one(&trades, NO_CF, 0, Decimal::ZERO, Decimal::ONE, p0, p1);
        let (required, _) = min_capital_at_k_one(
            &trades,
            NO_CF,
            Decimal::ONE,
            0,
            Decimal::ZERO,
            Decimal::ONE,
            lim.max_copyable_at_k_one_at_k_one,
            p0,
            p1,
        );
        assert!(required >= lit("4000") && required <= lit("4100"));
        assert!(ref_m.average_deployed_capital_usd < lit("2500"));
    }

    #[test]
    fn case_b_simultaneous_positions_peak() {
        let trades = vec![
            mk("buy", "a", lit("4000"), lit("0.5"), lit("8000"), 1),
            mk("buy", "b", lit("3000"), lit("0.5"), lit("6000"), 2),
            mk("sell", "a", lit("4000"), lit("0.5"), lit("8000"), 3),
        ];
        let m = sim(
            &trades,
            NO_CF,
            Decimal::from(1_000_000),
            Decimal::ONE,
            0,
            Decimal::ZERO,
            Decimal::ONE,
        );
        assert_eq!(m.max_simultaneous_exposure_usd, lit("7000"));
        assert!(m.max_simultaneous_exposure_usd > lit("3000"));
    }

    #[test]
    fn case_c_partial_sell_releases_quarter() {
        let trades = vec![
            mk("buy", "t1", lit("4000"), lit("0.5"), lit("8000"), 1),
            mk("sell", "t1", lit("1000"), lit("0.5"), lit("2000"), 2),
        ];
        let m = sim(&trades, NO_CF, lit("5000"), Decimal::ONE, 0, Decimal::ZERO, Decimal::ONE);
        assert_eq!(m.copied_trades, 2);
        assert_eq!(m.open_cost_basis_usd, lit("3000"));
        assert!(m.ending_cash_usd > Decimal::ZERO);
    }

    #[test]
    fn case_f_open_position_end_counts_deployed() {
        let trades = vec![mk("buy", "t1", lit("2500"), lit("0.5"), lit("5000"), 1)];
        let m = sim(&trades, NO_CF, lit("5000"), Decimal::ONE, 0, Decimal::ZERO, Decimal::ONE);
        assert_eq!(m.open_cost_basis_usd, lit("2500"));
        assert_eq!(m.open_cost_basis_usd, lit("2500"));
    }

    #[test]
    fn case_g_insufficient_starting_capital_detected() {
        let trades = vec![mk("buy", "t1", lit("500"), lit("0.5"), lit("1000"), 1)];
        let poor = sim(&trades, NO_CF, lit("50"), Decimal::ONE, 0, Decimal::ZERO, Decimal::ONE);
        assert_eq!(poor.copied_trades, 0);
        let (p0, p1) = period(&trades, NO_CF);
        let lim = coverage_limits_at_k_one(&trades, NO_CF, 0, Decimal::ZERO, Decimal::ONE, p0, p1);
        let (need, _) = min_capital_at_k_one(
            &trades,
            NO_CF,
            Decimal::ONE,
            0,
            Decimal::ZERO,
            Decimal::ONE,
            lim.max_copyable_at_k_one_at_k_one,
            p0,
            p1,
        );
        assert!(need >= lit("500") && need <= lit("520"));
        let rich = sim(&trades, NO_CF, need, Decimal::ONE, 0, Decimal::ZERO, Decimal::ONE);
        assert_eq!(rich.copied_trades, 1);
    }

    #[test]
    fn case_e_fees_increase_required_not_double_counted() {
        let trades = vec![
            mk("buy", "t1", lit("4000"), lit("0.5"), lit("8000"), 1),
            mk("sell", "t1", lit("4000"), lit("0.5"), lit("8000"), 2),
        ];
        let no_fee = sim(&trades, NO_CF, lit("10000"), Decimal::ONE, 0, Decimal::ZERO, Decimal::ONE);
        let with_fee = sim(
            &trades,
            NO_CF,
            lit("10000"),
            Decimal::ONE,
            0,
            lit("100"),
            Decimal::ONE,
        );
        assert!(with_fee.estimated_fees_usd > Decimal::ZERO);
        let (p0, p1) = period(&trades, NO_CF);
        let lim = coverage_limits_at_k_one(&trades, NO_CF, 0, lit("100"), Decimal::ONE, p0, p1);
        let (need_fee, _) = min_capital_at_k_one(
            &trades,
            NO_CF,
            Decimal::ONE,
            0,
            lit("100"),
            Decimal::ONE,
            lim.max_copyable_at_k_one_at_k_one,
            p0,
            p1,
        );
        let (need_plain, _) = min_capital_at_k_one(
            &trades,
            NO_CF,
            Decimal::ONE,
            0,
            Decimal::ZERO,
            Decimal::ONE,
            coverage_limits_at_k_one(&trades, NO_CF, 0, Decimal::ZERO, Decimal::ONE, p0, p1)
                .max_copyable_at_k_one_at_k_one,
            p0,
            p1,
        );
        assert!(need_fee >= need_plain);
        assert!(no_fee.ending_cash_usd >= with_fee.ending_cash_usd);
    }

    #[test]
    fn redeem_a_buy_winning_redeem_releases_capital() {
        let trades = vec![mk(
            "buy",
            "t1",
            lit("529.15"),
            lit("0.95"),
            lit("557"),
            1,
        )];
        let cfs = vec![mk_cf(
            "t1",
            lit("557"),
            Some(lit("557")),
            "winning",
            2,
            "r1",
        )];
        let m = sim(&trades, &cfs, lit("600"), Decimal::ONE, 0, Decimal::ZERO, Decimal::ONE);
        assert_eq!(m.event_breakdown.winning_redeems, 1);
        assert_eq!(m.open_cost_basis_usd, Decimal::ZERO);
        assert!(m.ending_cash_usd >= lit("557"));
        assert!(m.peak_capital_utilization_usd <= lit("529.15"));
    }

    #[test]
    fn redeem_b_buy_losing_redeem_closes_without_credit() {
        let trades = vec![mk(
            "buy",
            "t1",
            lit("529.15"),
            lit("0.95"),
            lit("557"),
            1,
        )];
        let cfs = vec![mk_cf("t1", lit("557"), Some(Decimal::ZERO), "losing", 2, "r1")];
        let start = lit("600");
        let m = sim(&trades, &cfs, start, Decimal::ONE, 0, Decimal::ZERO, Decimal::ONE);
        assert_eq!(m.event_breakdown.losing_redeems, 1);
        assert_eq!(m.open_cost_basis_usd, Decimal::ZERO);
        assert_eq!(m.ending_cash_usd, start - lit("529.15"));
    }

    #[test]
    fn redeem_c_partial_sell_then_redeem_remaining_only() {
        let trades = vec![
            mk("buy", "t1", lit("529.15"), lit("0.95"), lit("557"), 1),
            mk("sell", "t1", lit("264.575"), lit("0.95"), lit("278.5"), 2),
        ];
        let cfs = vec![mk_cf(
            "t1",
            lit("278.5"),
            Some(lit("278.5")),
            "winning",
            3,
            "r1",
        )];
        let m = sim(&trades, &cfs, lit("1000"), Decimal::ONE, 0, Decimal::ZERO, Decimal::ONE);
        assert_eq!(m.event_breakdown.winning_redeems, 1);
        assert_eq!(m.open_cost_basis_usd, Decimal::ZERO);
    }

    #[test]
    fn redeem_d_redeem_proceeds_fund_later_buy() {
        let trades = vec![
            mk("buy", "t1", lit("95"), lit("0.95"), lit("100"), 1),
            mk("buy", "t2", lit("50"), lit("0.5"), lit("100"), 4),
        ];
        let cfs = vec![mk_cf(
            "t1",
            lit("100"),
            Some(lit("100")),
            "winning",
            2,
            "r1",
        )];
        let m = sim(&trades, &cfs, lit("100"), Decimal::ONE, 0, Decimal::ZERO, Decimal::ONE);
        assert_eq!(m.event_breakdown.buy_count, 2);
        assert!(m.open_cost_basis_usd > Decimal::ZERO);
    }

    #[test]
    fn redeem_e_chronology_no_lookahead_from_future_redeem() {
        let trades = vec![
            mk("buy", "t2", lit("80"), lit("0.5"), lit("160"), 1),
            mk("buy", "t1", lit("95"), lit("0.95"), lit("100"), 3),
        ];
        let cfs = vec![mk_cf(
            "t1",
            lit("100"),
            Some(lit("100")),
            "winning",
            2,
            "r1",
        )];
        let m = sim(&trades, &cfs, lit("100"), Decimal::ONE, 0, Decimal::ZERO, Decimal::ONE);
        assert_eq!(m.event_breakdown.buy_count, 1);
        assert!(m
            .skip_reasons
            .iter()
            .any(|r| r.reason == "insufficient_capital"));
    }

    #[test]
    fn redeem_f_duplicate_dedup_key_no_double_credit() {
        let trades = vec![mk("buy", "t1", lit("95"), lit("0.95"), lit("100"), 1)];
        let dup = mk_cf("t1", lit("100"), Some(lit("100")), "winning", 2, "same");
        let cfs = vec![dup.clone(), dup];
        let m = sim(&trades, &cfs, lit("200"), Decimal::ONE, 0, Decimal::ZERO, Decimal::ONE);
        assert_eq!(m.event_breakdown.winning_redeems, 1);
        assert_eq!(m.ending_cash_usd, lit("200") - lit("95") + lit("100"));
    }

    #[test]
    fn redeem_g_unresolved_skips_payout_keeps_position() {
        let trades = vec![mk("buy", "t1", lit("95"), lit("0.95"), lit("100"), 1)];
        let cfs = vec![mk_cf("t1", lit("100"), None, "unresolved", 2, "r1")];
        let m = sim(&trades, &cfs, lit("200"), Decimal::ONE, 0, Decimal::ZERO, Decimal::ONE);
        assert_eq!(m.event_breakdown.unresolved_redeems, 1);
        assert_eq!(m.open_cost_basis_usd, lit("95"));
    }

    #[test]
    fn redeem_h_multiple_markets_independent() {
        let trades = vec![
            mk("buy", "w", lit("50"), lit("0.5"), lit("100"), 1),
            mk("buy", "l", lit("50"), lit("0.5"), lit("100"), 1),
        ];
        let cfs = vec![
            mk_cf("w", lit("100"), Some(lit("100")), "winning", 2, "rw"),
            mk_cf("l", lit("100"), Some(Decimal::ZERO), "losing", 2, "rl"),
        ];
        let start = lit("200");
        let m = sim(&trades, &cfs, start, Decimal::ONE, 0, Decimal::ZERO, Decimal::ONE);
        assert_eq!(m.open_cost_basis_usd, Decimal::ZERO);
        assert_eq!(m.ending_cash_usd, start - lit("100") + lit("100"));
    }
}