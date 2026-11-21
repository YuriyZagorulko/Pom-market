use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::copy_sizing::{
    capped_copy_notional, raw_copy_notional, sizing_rule_label, BuyNotionalLimits,
    StrategyModeParams,
};
use crate::db::begin_as;
use crate::engine::{
    available_after_reserve, conservative_fee_price, limit_price, order_reserves, paper_clob_diagnostic,
    paper_shares_from_notional, plan_exit, reconcile_live_status,
    reserves_after_execution,
    simulate_copy_fill, slippage_bps,
    trace_buy_size, truncate_lot, walk_book, FeeSchedule, OrderBook, RejectReason,
    RemoteObservation, EXCHANGE_MIN_NOTIONAL,
};
use crate::polymarket::live::{place_fak, LiveOrderRequest};
use crate::polymarket::RawTrade;
use crate::security::{credential_aad, redact, Vault};
use crate::strategy_budget::strategy_exposure_with_cap;
use crate::liquidity_retry::{initial_retry_blob, merge_retry_checks};
use crate::trading_control;
use secrecy::SecretString;

#[async_trait]
pub trait Books: Send + Sync {
    async fn book(&self, token_id: &str) -> Result<OrderBook, String>;
    async fn fee(&self, token_id: &str) -> Option<FeeSchedule>;
}

#[derive(Clone)]
pub struct PipelineConfig {
    pub clob_host: String,
    pub confirm_live: bool,
    pub default_fee_rate: Decimal,
    pub default_fee_exponent: u32,
    pub paper_allow_synthetic_fills: bool,
}

pub async fn ingest(pool: &PgPool, trade: &RawTrade) -> anyhow::Result<IngestOutcome> {
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(&trade.dedup_key)
        .execute(&mut *tx)
        .await?;
    // Atomic insert: if the dedup_key already exists (inserted by this or a
    // concurrent transaction), ON CONFLICT DO NOTHING returns no row and we
    // fall through to SELECT the existing id instead of failing with a unique
    // constraint violation.
    let created_id: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO source_events (
                dedup_key, source_wallet, tx_hash, condition_id, token_id, outcome, market_title,
                market_slug, event_title, event_slug, market_icon, activity_type, usdc_size,
                side, price, size_shares, notional_usd, event_ts, origin, status, raw
             ) VALUES (
                $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,'normalized',$20
             )
             ON CONFLICT (dedup_key) DO NOTHING
             RETURNING id",
    )
    .bind(&trade.dedup_key)
    .bind(&trade.source_wallet)
    .bind(&trade.tx_hash)
    .bind(&trade.condition_id)
    .bind(&trade.token_id)
    .bind(&trade.outcome)
    .bind(&trade.market_title)
    .bind(&trade.market_slug)
    .bind(&trade.event_title)
    .bind(&trade.event_slug)
    .bind(&trade.market_icon)
    .bind(&trade.activity_type)
    .bind(trade.usdc_size)
    .bind(&trade.side)
    .bind(trade.price)
    .bind(trade.size_shares)
    .bind(trade.notional_usd)
    .bind(trade.event_ts)
    .bind(&trade.origin)
    .bind(&trade.raw)
    .fetch_optional(&mut *tx)
    .await?;

    let (event_id, created) = match created_id {
        Some(id) => (id, true),
        None => {
            let id: Uuid = sqlx::query_scalar("SELECT id FROM source_events WHERE dedup_key = $1")
                .bind(&trade.dedup_key)
                .fetch_one(&mut *tx)
                .await?;
            sqlx::query(
                "UPDATE source_events SET
                    market_icon = COALESCE(market_icon, $2),
                    event_slug = COALESCE(event_slug, $3),
                    usdc_size = COALESCE(usdc_size, $4),
                    event_title = COALESCE(event_title, $5),
                    market_title = COALESCE(market_title, $6),
                    market_slug = COALESCE(market_slug, $7)
                 WHERE id = $1",
            )
            .bind(id)
            .bind(&trade.market_icon)
            .bind(&trade.event_slug)
            .bind(trade.usdc_size)
            .bind(&trade.event_title)
            .bind(&trade.market_title)
            .bind(&trade.market_slug)
            .execute(&mut *tx)
            .await?;
            (id, false)
        }
    };

    let inserted: Option<bool> = sqlx::query_scalar::<_, bool>(
        "INSERT INTO source_event_fragments (source_event_id, fragment_key, origin, size_shares, price, raw)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (fragment_key) DO NOTHING
         RETURNING true",
    )
    .bind(event_id)
    .bind(&trade.fragment_key)
    .bind(&trade.origin)
    .bind(trade.size_shares)
    .bind(trade.price)
    .bind(&trade.raw)
    .fetch_optional(&mut *tx)
    .await?;
    let inserted = inserted.unwrap_or(false);

    // The event already existed and this trade contributed a new fragment:
    // accumulate size into the parent event.
    if !created && inserted {
        sqlx::query(
            "UPDATE source_events
             SET size_shares = size_shares + $2,
                 notional_usd = price * (size_shares + $2),
                 fragment_count = fragment_count + 1
             WHERE id = $1",
        )
        .bind(event_id)
        .bind(trade.size_shares)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(IngestOutcome {
        event_id,
        created,
        duplicate_fragment: !created && !inserted,
    })
}

#[derive(Clone, Debug)]
pub struct IngestOutcome {
    pub event_id: Uuid,
    pub created: bool,
    pub duplicate_fragment: bool,
}

pub async fn process_event(
    pool: &PgPool,
    vault: &Vault,
    books: &dyn Books,
    cfg: &PipelineConfig,
    data: &crate::polymarket::http::PublicClient,
    app_cfg: &crate::config::Config,
    event_id: Uuid,
) -> anyhow::Result<Vec<Uuid>> {
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query("UPDATE source_events SET status = 'processing' WHERE id = $1 AND status IN ('normalized', 'detected')")
        .bind(event_id)
        .execute(&mut *tx)
        .await?;
    let event = sqlx::query_as::<_, EventRow>(
        "SELECT id, source_wallet, token_id, condition_id, outcome, market_title, market_slug, side, price, size_shares, notional_usd, usdc_size, event_ts, received_at, origin
         FROM source_events WHERE id = $1",
    )
    .bind(event_id)
    .fetch_one(&mut *tx)
    .await?;
    let strategies = sqlx::query_as::<_, StrategyRow>(
        "SELECT s.id, s.user_id, s.trading_account_id, s.source_trader_id, s.enabled, s.sizing_mode,
                s.allocation_mode, s.allocation_percent,
                s.fixed_amount_usd, s.proportion, s.multiplier, s.strategy_budget_usd, s.max_position_usd,
                s.max_exposure_usd, s.max_open_positions, s.max_slippage_bps, s.min_order_usd,
                s.allow_subminimum_paper_trades,
                s.copy_small_buys_1to1, s.small_buy_threshold_usd,
                s.copy_buy, s.copy_sell, s.cooldown_seconds, s.allowed_markets, s.created_at,
                s.operating_mode,
                a.operating_mode AS account_operating_mode,
                a.emergency_stop, a.trading_control_state AS account_trading_control_state,
                s.trading_control_state AS strategy_trading_control_state,
                a.live_confirmed_at, a.signature_type, a.funder_address,
                a.last_exchange_balance_usd, a.status AS account_status,
                r.max_order_usd, r.max_daily_loss_usd, r.max_exposure_usd AS account_max_exposure,
                r.max_slippage_bps AS account_max_slippage, r.max_orders_per_interval, r.interval_seconds,
                r.max_stale_data_seconds, r.paper_cash_usd,
                t.wallet_address
         FROM copy_strategies s
         JOIN source_traders t ON t.id = s.source_trader_id
         JOIN trading_accounts a ON a.id = s.trading_account_id
         JOIN account_risk_limits r ON r.trading_account_id = a.id
         WHERE lower(t.wallet_address) = $1
           AND t.monitoring_enabled
           AND a.status <> 'disconnected'",
    )
    .bind(&event.source_wallet)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    let book = books.book(&event.token_id).await;
    let fee = books.fee(&event.token_id).await;
    let mut decisions = Vec::new();
    for strategy in strategies {
        if strategy.created_at.timestamp() > event.event_ts.timestamp() {
            continue;
        }
        let id = decide_one(
            pool,
            vault,
            cfg,
            data,
            app_cfg,
            &event,
            &strategy,
            &book,
            fee.as_ref(),
        )
        .await?;
        decisions.push(id);
    }
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query(
        "UPDATE source_events SET status = 'processed', processed_at = now() WHERE id = $1",
    )
    .bind(event_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE source_traders
         SET observed_trade_count = observed_trade_count + 1,
             last_trade_at = GREATEST(last_trade_at, $2),
             updated_at = now()
         WHERE lower(wallet_address) = $1",
    )
    .bind(&event.source_wallet)
    .bind(event.event_ts)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(decisions)
}

#[allow(clippy::too_many_arguments)]
async fn decide_one(
    pool: &PgPool,
    vault: &Vault,
    cfg: &PipelineConfig,
    data: &crate::polymarket::http::PublicClient,
    app_cfg: &crate::config::Config,
    event: &EventRow,
    strategy: &StrategyRow,
    book: &Result<OrderBook, String>,
    fee: Option<&FeeSchedule>,
) -> anyhow::Result<Uuid> {
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(strategy.trading_account_id.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("strategy-budget:{}", strategy.id))
        .execute(&mut *tx)
        .await?;
    let already: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM copy_decisions WHERE strategy_id = $1 AND source_event_id = $2",
    )
    .bind(strategy.id)
    .bind(event.id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(id) = already {
        tx.commit().await?;
        return Ok(id);
    }

    let (leader_shares, leader_notional) = crate::polymarket::canonical_source_trade(
        event.price,
        event.size_shares,
        event.notional_usd,
        event.usdc_size,
    );

    let kill: bool = sqlx::query_scalar(
        "SELECT COALESCE((value->>'enabled')::boolean, false) FROM system_settings WHERE key = 'global_kill_switch'",
    )
    .fetch_one(&mut *tx)
    .await?;
    let exposure: Decimal = sqlx::query_scalar(
        "SELECT COALESCE((SELECT SUM(cost_basis_usd) FROM positions
            WHERE trading_account_id = $1 AND operating_mode = $2 AND open_shares > 0), 0)
         + COALESCE((SELECT SUM(position_reserved_usd + fee_reserved_usd) FROM orders
            WHERE trading_account_id = $1 AND operating_mode = $2
              AND status IN ('pending', 'submitting', 'submitted', 'partially_filled', 'unknown')), 0)",
    )
    .bind(strategy.trading_account_id)
    .bind(&strategy.operating_mode)
    .fetch_one(&mut *tx)
    .await?;
    let (strategy_exposure, effective_budget) = strategy_exposure_with_cap(
        &mut tx,
        strategy.id,
        strategy.trading_account_id,
        &strategy.allocation_mode,
        strategy.strategy_budget_usd,
        strategy.allocation_percent,
    )
    .await?;
    let open_positions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM positions
         WHERE strategy_id = $1 AND open_shares > 0",
    )
    .bind(strategy.id)
    .fetch_one(&mut *tx)
    .await?;
    let recent_orders: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders
         WHERE trading_account_id = $1 AND created_at > now() - make_interval(secs => $2::int)",
    )
    .bind(strategy.trading_account_id)
    .bind(strategy.interval_seconds)
    .fetch_one(&mut *tx)
    .await?;
    let daily_pnl: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(realized_pnl_usd), 0) FROM positions
         WHERE trading_account_id = $1 AND operating_mode = $2",
    )
    .bind(strategy.trading_account_id)
    .bind(&strategy.operating_mode)
    .fetch_one(&mut *tx)
    .await?;
    let last_decision: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT decided_at FROM copy_decisions WHERE strategy_id = $1 ORDER BY decided_at DESC LIMIT 1",
    )
    .bind(strategy.id)
    .fetch_optional(&mut *tx)
    .await?;

    let side_buy = event.side == "buy";
    let mut reason: Option<RejectReason> = None;
    if !strategy.enabled {
        reason = Some(RejectReason::StrategyDisabled);
    } else if kill && strategy.account_operating_mode != "monitoring" {
        reason = Some(RejectReason::KillSwitch);
    } else if let Some(code) = trading_control::pipeline_blocked_combined(
        &strategy.account_trading_control_state,
        &strategy.strategy_trading_control_state,
        side_buy,
        kill,
        &strategy.operating_mode,
    ) {
        reason = Some(match code {
            "kill_switch" => RejectReason::KillSwitch,
            "emergency_stop" => RejectReason::EmergencyStop,
            "soft_close_no_buy" => RejectReason::SoftCloseBuyBlocked,
            "hard_close_in_progress" => RejectReason::HardCloseInProgress,
            "account_no_buy" => RejectReason::AccountNoBuy,
            _ => RejectReason::EmergencyStop,
        });
    } else if strategy.emergency_stop {
        reason = Some(RejectReason::EmergencyStop);
    } else if strategy.account_operating_mode == "monitoring" {
        reason = Some(RejectReason::MonitoringMode);
    } else if strategy.operating_mode == "live_trading"
        && (!cfg.confirm_live || strategy.live_confirmed_at.is_none())
    {
        reason = Some(RejectReason::LiveNotConfirmed);
    } else if side_buy && !strategy.copy_buy || !side_buy && !strategy.copy_sell {
        reason = Some(RejectReason::CopySideDisabled);
    }

    let stale_eval = if reason.is_none() {
        Some(crate::source_trade_staleness::evaluate_stale_data(
            event.event_ts,
            event.received_at,
            &event.origin,
            strategy.max_stale_data_seconds,
            Utc::now(),
        ))
    } else {
        None
    };

    if reason.is_none() {
        if stale_eval.as_ref().is_some_and(|e| e.stale) {
            reason = Some(RejectReason::StaleData);
        } else if !market_allowed(&strategy.allowed_markets, event) {
            reason = Some(RejectReason::MarketNotAllowed);
        } else if daily_pnl <= -strategy.max_daily_loss_usd {
            reason = Some(RejectReason::DailyLoss);
        } else if recent_orders >= strategy.max_orders_per_interval as i64 {
            reason = Some(RejectReason::RateLimited);
        } else if last_decision.is_some_and(|at| {
            strategy.cooldown_seconds > 0
                && Utc::now() - at < Duration::seconds(strategy.cooldown_seconds as i64)
        }) {
            reason = Some(RejectReason::Cooldown);
        } else if strategy.operating_mode == "paper_trading"
            && !crate::strategy_accounting::is_capital_bootstrapped(&mut tx, strategy.id).await?
        {
            reason = Some(RejectReason::StrategyCapitalNotInitialized);
        }
    }

    let (_cash, _open_reserve, available) = if strategy.operating_mode == "paper_trading" {
        crate::strategy_accounting::ensure_strategy_capital_row(
            &mut tx,
            strategy.user_id,
            strategy.trading_account_id,
            strategy.id,
        )
        .await?;
        let (avail, res) =
            crate::strategy_accounting::strategy_liquid_balances(&mut tx, strategy.id).await?;
        (avail, res, avail)
    } else {
        let cash: Decimal = sqlx::query_scalar(
            "SELECT COALESCE(last_exchange_balance_usd, 0) FROM trading_accounts WHERE id = $1",
        )
        .bind(strategy.trading_account_id)
        .fetch_one(&mut *tx)
        .await?;
        let open_reserve: Decimal = sqlx::query_scalar(
            "SELECT COALESCE(SUM(position_reserved_usd + fee_reserved_usd), 0) FROM orders
             WHERE trading_account_id = $1 AND operating_mode = $2
               AND status IN ('pending', 'submitting', 'submitted', 'partially_filled', 'unknown')",
        )
        .bind(strategy.trading_account_id)
        .bind(&strategy.operating_mode)
        .fetch_one(&mut *tx)
        .await?;
        (
            cash,
            open_reserve,
            available_after_reserve(cash, open_reserve),
        )
    };
    let slip = strategy.max_slippage_bps.min(strategy.account_max_slippage);
    let tick = book.as_ref().map(|b| b.tick).unwrap_or(Decimal::new(1, 2));
    let limit = limit_price(side_buy, event.price, slip, tick);
    let fee_price = match book {
        Ok(book) if side_buy => conservative_fee_price(
            book.asks.first().map(|level| level.price).unwrap_or(limit),
            limit,
        ),
        Ok(book) => conservative_fee_price(
            limit,
            book.bids.first().map(|level| level.price).unwrap_or(limit),
        ),
        Err(_) => limit,
    };

    let mut target_shares = Decimal::ZERO;
    let mut target_notional_usd = Decimal::ZERO;
    let mut size_trace: Option<crate::engine::SizeTrace> = None;
    let mut sizing_audit: Option<Value> = None;
    let mut position_value_observed_at: Option<DateTime<Utc>> = None;
    if reason.is_none() && side_buy {
        if open_positions >= strategy.max_open_positions as i64 {
            let existing_token: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM positions
                 WHERE strategy_id = $1 AND token_id = $2 AND operating_mode = $3 AND open_shares > 0",
            )
            .bind(strategy.id)
            .bind(&event.token_id)
            .bind(&strategy.operating_mode)
            .fetch_one(&mut *tx)
            .await?;
            if existing_token == 0 {
                reason = Some(RejectReason::MaxPositions);
            }
        }
        let mut portfolio_equity: Option<Decimal> = None;
        let mut portfolio_equity_status = None;
        let mut open_positions_value: Option<Decimal> = None;
        if reason.is_none() && strategy.sizing_mode == "portfolio_ratio" {
            let mut snap =
                crate::source_equity::load_snapshot(pool, strategy.source_trader_id, app_cfg)
                    .await?;
            if snap
                .as_ref()
                .map(|s| !s.status.allows_portfolio_buy())
                .unwrap_or(true)
            {
                let live = crate::source_equity::fetch_live_equity(
                    data,
                    app_cfg,
                    &strategy.wallet_address,
                )
                .await;
                crate::source_equity::upsert_snapshot(pool, strategy.source_trader_id, &live)
                    .await?;
                snap = Some(live);
            }
            match snap {
                Some(s) if s.status.allows_portfolio_buy() => {
                    portfolio_equity = s.total_equity_usd;
                    portfolio_equity_status = Some(s.status);
                }
                _ => reason = Some(RejectReason::SourceEquityUnavailable),
            }
        }
        let pvr_small_buy_skip_snapshot = strategy.sizing_mode == "position_value_ratio"
            && crate::copy_sizing::small_buy_1to1_applies(
                &StrategyModeParams {
                    sizing_mode: strategy.sizing_mode.clone(),
                    fixed_amount_usd: strategy.fixed_amount_usd,
                    proportion: strategy.proportion,
                    multiplier: strategy.multiplier,
                    strategy_allocated_capital_usd: effective_budget,
                    source_trader_equity_usd: None,
                    source_equity_status: None,
                    source_open_positions_value_usd: None,
                    copy_small_buys_1to1: strategy.copy_small_buys_1to1,
                    small_buy_threshold_usd: strategy.small_buy_threshold_usd,
                },
                leader_notional,
            );
        if reason.is_none()
            && strategy.sizing_mode == "position_value_ratio"
            && !pvr_small_buy_skip_snapshot
        {
            let mut snap =
                crate::source_equity::load_snapshot(pool, strategy.source_trader_id, app_cfg)
                    .await?;
            if snap
                .as_ref()
                .map(crate::source_equity::snapshot_needs_position_value_refresh)
                .unwrap_or(true)
            {
                let live = crate::source_equity::fetch_live_equity(
                    data,
                    app_cfg,
                    &strategy.wallet_address,
                )
                .await;
                crate::source_equity::upsert_snapshot(pool, strategy.source_trader_id, &live)
                    .await?;
                snap = Some(live);
            }
            match snap.as_ref() {
                Some(s) => {
                    position_value_observed_at = Some(s.observed_at);
                    match crate::source_equity::position_value_for_sizing(s) {
                        Ok(pv) => open_positions_value = Some(pv),
                        Err(r) => reason = Some(r),
                    }
                }
                None => reason = Some(RejectReason::SourcePositionValueUnavailable),
            }
        }
        if reason.is_none() {
            let mode_params = StrategyModeParams {
                sizing_mode: strategy.sizing_mode.clone(),
                fixed_amount_usd: strategy.fixed_amount_usd,
                proportion: strategy.proportion,
                multiplier: strategy.multiplier,
                strategy_allocated_capital_usd: effective_budget,
                source_trader_equity_usd: portfolio_equity,
                source_equity_status: portfolio_equity_status,
                source_open_positions_value_usd: open_positions_value,
                copy_small_buys_1to1: strategy.copy_small_buys_1to1,
                small_buy_threshold_usd: strategy.small_buy_threshold_usd,
            };
            let sizing_rule = sizing_rule_label(&mode_params, leader_notional);
            let paper_mode = strategy.operating_mode == "paper_trading";
            let allow_subminimum =
                paper_mode && strategy.allow_subminimum_paper_trades;
            let limits = BuyNotionalLimits {
                max_order_usd: strategy.max_order_usd,
                max_position_usd: strategy.max_position_usd,
                min_order_usd: strategy.min_order_usd,
                strategy_remaining_usd: (effective_budget - strategy_exposure).max(Decimal::ZERO),
                available_balance_usd: available,
                allow_subminimum_paper_trades: allow_subminimum,
            };
            let market_min_shares = book
                .as_ref()
                .map(|b| b.min_order_size)
                .unwrap_or(Decimal::ZERO);
            let raw_target = raw_copy_notional(&mode_params, leader_notional).ok();
            match capped_copy_notional(&mode_params, leader_notional, &limits) {
                Ok(notional) => {
                    target_notional_usd = notional;
                    let cap_reason = raw_target
                        .map(|raw| {
                            crate::copy_sizing::compute_cap_reason(raw, notional, &limits)
                        })
                        .flatten();
                    sizing_audit = Some(json!({
                        "sizing_mode": strategy.sizing_mode,
                        "sizing_rule": sizing_rule,
                        "copy_small_buys_1to1": strategy.copy_small_buys_1to1,
                        "small_buy_threshold_usd": strategy.small_buy_threshold_usd,
                        "source_notional_usd": leader_notional,
                        "proportion_coefficient": strategy.proportion,
                        "fixed_amount_usd": strategy.fixed_amount_usd,
                        "strategy_allocated_capital_usd": effective_budget,
                        "open_positions_value_usd": open_positions_value,
                        "open_positions_value_observed_at": position_value_observed_at,
                        "portfolio_equity_usd": portfolio_equity,
                        "raw_target_notional_usd": raw_target,
                        "capped_target_notional_usd": notional,
                        "cap_reason": cap_reason,
                        "estimated_shares": null,
                        "execution_mode": if paper_mode { "paper" } else { "live" },
                        "allow_subminimum_paper_trades": strategy.allow_subminimum_paper_trades,
                        "pvr_trade_multiplier": strategy
                            .multiplier
                            .filter(|m| *m > Decimal::ZERO)
                            .map(|m| m.to_string())
                            .unwrap_or_else(|| "1".into()),
                        "exchange_min_notional_usd": EXCHANGE_MIN_NOTIONAL,
                        "market_min_shares": market_min_shares,
                        "max_order_usd": strategy.max_order_usd,
                        "max_position_usd": strategy.max_position_usd,
                        "min_order_usd": strategy.min_order_usd,
                        "strategy_remaining_usd": limits.strategy_remaining_usd,
                        "available_balance_usd": limits.available_balance_usd,
                    }));
                    let (min_order_size, liquidity) = match &book {
                        Ok(book) => (
                            book.min_order_size,
                            book.asks
                                .iter()
                                .fold(Decimal::ZERO, |sum, level| sum + level.size),
                        ),
                        Err(_) => (Decimal::ZERO, Decimal::MAX),
                    };
                    let spend_cap = (effective_budget - strategy_exposure)
                        .min(strategy.max_order_usd)
                        .min(available)
                        .max(Decimal::ZERO);
                    let trace = trace_buy_size(
                        notional,
                        event.price,
                        limit,
                        tick,
                        min_order_size,
                        liquidity,
                        spend_cap,
                        fee,
                        fee_price,
                    );
                    if allow_subminimum {
                        target_shares = if trace.sdk_normalized_shares > Decimal::ZERO {
                            trace.sdk_normalized_shares
                        } else {
                            paper_shares_from_notional(notional, event.price)
                        };
                    } else {
                        target_shares = trace.sdk_normalized_shares;
                    }
                    if let Some(audit) = sizing_audit.as_mut() {
                        if let Some(obj) = audit.as_object_mut() {
                            obj.insert(
                                "estimated_shares".into(),
                                json!(target_shares.to_string()),
                            );
                        }
                    }
                    if trace.fee_unknown {
                        reason = Some(RejectReason::FeeUnknown);
                    } else if (!paper_mode || !allow_subminimum)
                        && (trace.below_min_order_size
                            || target_shares * limit < EXCHANGE_MIN_NOTIONAL)
                    {
                        let below_shares = trace.below_min_order_size;
                        let below_usd = target_shares * limit < EXCHANGE_MIN_NOTIONAL;
                        if let Some(audit) = sizing_audit.as_mut() {
                            if let Some(obj) = audit.as_object_mut() {
                                obj.extend(
                                    crate::copy_sizing::buy_minimum_rejection_audit(
                                        limit,
                                        target_shares,
                                        min_order_size,
                                        below_shares,
                                        below_usd,
                                    )
                                    .as_object()
                                    .cloned()
                                    .unwrap_or_default(),
                                );
                            }
                        }
                        reason = Some(if spend_cap < notional {
                            RejectReason::StrategyBudget
                        } else {
                            RejectReason::BelowMinimum
                        });
                    } else if paper_mode && target_shares <= Decimal::ZERO {
                        reason = Some(RejectReason::BelowMinimum);
                    } else if paper_mode {
                        let (pos_r, fee_r) =
                            order_reserves(true, limit, target_shares, fee_price, fee);
                        if pos_r + fee_r > spend_cap {
                            reason = Some(RejectReason::StrategyBudget);
                        }
                    }
                    size_trace = Some(trace);
                }
                Err(err) => {
                    if let Some(raw) = raw_target {
                        target_notional_usd = raw.round_dp(2);
                        sizing_audit = sizing_audit.or_else(|| {
                            Some(json!({
                                "sizing_mode": strategy.sizing_mode,
                                "source_notional_usd": leader_notional,
                                "proportion_coefficient": strategy.proportion,
                                "fixed_amount_usd": strategy.fixed_amount_usd,
                                "strategy_allocated_capital_usd": effective_budget,
                                "open_positions_value_usd": open_positions_value,
                                "open_positions_value_observed_at": position_value_observed_at,
                                "portfolio_equity_usd": portfolio_equity,
                                "raw_target_notional_usd": raw,
                                "rejection_reason": err.as_str(),
                                "pvr_trade_multiplier": strategy
                                    .multiplier
                                    .filter(|m| *m > Decimal::ZERO)
                                    .map(|m| m.to_string())
                                    .unwrap_or_else(|| "1".into()),
                            }))
                        });
                    }
                    reason = Some(err);
                }
            }
        }
    }

    let position = sqlx::query_as::<_, PositionRow>(
        "SELECT id, open_shares, avg_price, cost_basis_usd, realized_pnl_usd, source_observed_shares, pending_exit_shares, close_cycle, close_cycle_realized_baseline
         FROM positions
         WHERE strategy_id = $1 AND token_id = $2 AND operating_mode = $3",
    )
    .bind(strategy.id)
    .bind(&event.token_id)
    .bind(&strategy.operating_mode)
    .fetch_optional(&mut *tx)
    .await?;

    if reason.is_none() && fee.is_none() {
        reason = Some(RejectReason::FeeUnknown);
    }

    if reason.is_none() && !side_buy {
        let pos = position.as_ref();
        let allow_subminimum = strategy.operating_mode == "paper_trading"
            && strategy.allow_subminimum_paper_trades;
        let open = pos.map(|p| p.open_shares).unwrap_or(Decimal::ZERO);
        let observed = pos
            .map(|p| p.source_observed_shares)
            .unwrap_or(Decimal::ZERO);
        let pending = pos.map(|p| p.pending_exit_shares).unwrap_or(Decimal::ZERO);
        let plan = plan_exit(
            open,
            observed,
            leader_shares,
            pending,
            limit,
            strategy.min_order_usd,
            allow_subminimum,
        );
        let sell_fraction = if observed > Decimal::ZERO {
            (leader_shares / observed).min(Decimal::ONE).max(Decimal::ZERO)
        } else {
            Decimal::ZERO
        };
        if let Some(reject) = plan.reject {
            if let Some(pos) = pos {
                sqlx::query("UPDATE positions SET pending_exit_shares = $2, updated_at = now() WHERE id = $1")
                    .bind(pos.id)
                    .bind(plan.pending_exit_shares)
                    .execute(&mut *tx)
                    .await?;
            }
            reason = Some(reject);
        } else {
            target_shares = truncate_lot(plan.shares);
            target_notional_usd = (target_shares * limit).round_dp(2);
            sizing_audit = Some(json!({
                "sizing_mode": strategy.sizing_mode,
                "sizing_rule": "share_fraction_exit",
                "source_sold_shares": leader_shares.to_string(),
                "source_observed_shares": observed.to_string(),
                "follower_open_shares": open.to_string(),
                "sell_fraction": sell_fraction.to_string(),
                "planned_sell_shares": target_shares.to_string(),
                "copy_limit_price": limit.to_string(),
            }));
            if let Some(pos) = pos {
                sqlx::query("UPDATE positions SET pending_exit_shares = $2, updated_at = now() WHERE id = $1")
                    .bind(pos.id)
                    .bind(plan.pending_exit_shares)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    let mut book_price = None;
    let mut sim = None;
    let mut fill_simulation: Option<Value> = None;
    if reason.is_none() {
        let paper = strategy.operating_mode == "paper_trading";
        let allow_subminimum = paper && strategy.allow_subminimum_paper_trades;
        let market_min_shares = book
            .as_ref()
            .map(|b| b.min_order_size)
            .unwrap_or(Decimal::ZERO);
        match simulate_copy_fill(
            book,
            paper,
            allow_subminimum,
            cfg.paper_allow_synthetic_fills,
            slip,
            side_buy,
            event.price,
            limit,
            target_shares,
            target_notional_usd,
            market_min_shares,
            fee,
        ) {
            Ok(sim_result) => {
                book_price = Some(sim_result.fill.vwap);
                if slippage_bps(event.price, sim_result.fill.vwap) > slip {
                    reason = Some(RejectReason::Slippage);
                } else {
                    if strategy.operating_mode == "paper_trading" {
                        fill_simulation = Some(json!({
                            "paper_execution_mode": sim_result.paper_execution_mode.as_str(),
                            "context": sim_result.context,
                        }));
                    }
                    sim = Some(sim_result);
                }
            }
            Err(err) => {
                if paper {
                    fill_simulation = Some(json!({
                        "rejected": true,
                        "reason": err.as_str(),
                        "context": paper_clob_diagnostic(book, side_buy, event.price, limit),
                        "allow_synthetic_experimental": cfg.paper_allow_synthetic_fills,
                    }));
                }
                reason = Some(err);
            }
        }
    }

    let approved = reason.is_none();
    let paper_mode = strategy.operating_mode == "paper_trading";
    let allow_subminimum = paper_mode && strategy.allow_subminimum_paper_trades;
    let awaiting_liquidity = !approved
        && side_buy
        && reason.as_ref().is_some_and(|r| {
            RejectReason::liquidity_retry_eligible(r, side_buy, paper_mode, allow_subminimum)
        });
    let status = if approved {
        "approved"
    } else if awaiting_liquidity {
        "awaiting_liquidity"
    } else if matches!(
        reason,
        Some(RejectReason::MonitoringMode) | Some(RejectReason::StrategyDisabled)
    ) {
        "skipped"
    } else {
        "rejected"
    };
    let reason_text = reason
        .as_ref()
        .map(RejectReason::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| "approved".into());
    let decision_id: Uuid = sqlx::query_scalar(
        "INSERT INTO copy_decisions (
            user_id, trading_account_id, source_trader_id, source_event_id, strategy_id, status, reason,
            operating_mode, side, target_notional_usd, target_shares, limit_price, book_price, available_balance_usd,
            strategy_snapshot, checks, detected_at
         ) VALUES (
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17
         ) RETURNING id",
    )
    .bind(strategy.user_id)
    .bind(strategy.trading_account_id)
    .bind(strategy.source_trader_id)
    .bind(event.id)
    .bind(strategy.id)
    .bind(status)
    .bind(&reason_text)
    .bind(&strategy.operating_mode)
    .bind(&event.side)
    .bind(if target_notional_usd > Decimal::ZERO { Some(target_notional_usd) } else { None })
    .bind(if target_shares > Decimal::ZERO { Some(target_shares) } else { None })
    .bind(limit)
    .bind(book_price)
    .bind(available)
    .bind(json!({
        "sizing_mode": strategy.sizing_mode,
        "strategy_budget_usd": strategy.strategy_budget_usd,
        "max_slippage_bps": slip,
        "copy_buy": strategy.copy_buy,
        "copy_sell": strategy.copy_sell
    }))
    .bind({
        let mut checks = json!({
            "exposure_usd": exposure,
            "strategy_exposure_usd": strategy_exposure,
            "open_positions": open_positions,
            "available_balance_usd": available,
            "kill_switch": kill,
            "allow_subminimum_paper_trades": strategy.allow_subminimum_paper_trades,
            "calculated_notional_usd": target_notional_usd,
            "exchange_min_notional_usd": EXCHANGE_MIN_NOTIONAL,
            "market_min_shares": book.as_ref().map(|b| b.min_order_size).unwrap_or(Decimal::ZERO),
            "requested_notional_usd": target_notional_usd,
            "grid_shares": target_shares,
            "limit_price": limit,
            "reserved_notional_usd": (target_shares * limit).round_dp(2),
            "strategy_budget_usd": strategy.strategy_budget_usd,
            "remaining_strategy_budget_usd": (strategy.strategy_budget_usd - strategy_exposure).max(Decimal::ZERO),
            "size_trace": size_trace,
        });
        if let Some(r) = reason.as_ref() {
            checks["rejection_reason"] = json!(r.as_str());
        }
        if let Some(audit) = sizing_audit {
            checks["sizing_audit"] = audit;
        }
        if let Some(fill_sim) = fill_simulation {
            checks["fill_simulation"] = fill_sim;
        }
        if let Some(stale) = stale_eval {
            checks["stale_data_audit"] = json!(stale);
        }
        if awaiting_liquidity {
            if let Some(r) = reason.as_ref() {
                merge_retry_checks(&mut checks, initial_retry_blob(r));
            }
        }
        checks
    })
    .bind(event.received_at)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO timeline_events (user_id, source_event_id, decision_id, kind, message)
         VALUES ($1, $2, $3, 'decision', $4)",
    )
    .bind(strategy.user_id)
    .bind(event.id)
    .bind(decision_id)
    .bind(format!("{status}: {reason_text}"))
    .execute(&mut *tx)
    .await?;

    if !approved {
        if status == "rejected" {
            sqlx::query("UPDATE source_traders SET rejected_trade_count = rejected_trade_count + 1 WHERE id = $1")
                .bind(strategy.source_trader_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        let _ = notify(pool, strategy.user_id, "decision", decision_id).await;
        return Ok(decision_id);
    }

    fulfill_approved_decision_in_tx(
        pool,
        vault,
        cfg,
        strategy,
        event,
        tx,
        decision_id,
        side_buy,
        limit,
        fee,
        fee_price,
        kill,
        sim.expect("approved simulation"),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn fulfill_approved_decision_in_tx(
    pool: &PgPool,
    vault: &Vault,
    cfg: &PipelineConfig,
    strategy: &StrategyRow,
    event: &EventRow,
    mut tx: crate::db::Tx<'_>,
    decision_id: Uuid,
    side_buy: bool,
    limit: Decimal,
    fee: Option<&FeeSchedule>,
    fee_price: Decimal,
    kill: bool,
    sim_result: crate::engine::CopyFillSim,
) -> anyhow::Result<Uuid> {
    let (acct_ctl, strat_ctl): (String, String) = sqlx::query_as(
        "SELECT a.trading_control_state, s.trading_control_state
         FROM copy_strategies s
         JOIN trading_accounts a ON a.id = s.trading_account_id
         WHERE s.id = $1 FOR UPDATE OF s, a",
    )
    .bind(strategy.id)
    .fetch_one(&mut *tx)
    .await?;
    if let Some(code) = trading_control::pipeline_blocked_combined(
        &acct_ctl,
        &strat_ctl,
        side_buy,
        kill,
        &strategy.operating_mode,
    ) {
        let late_reason = match code {
            "kill_switch" => RejectReason::KillSwitch,
            "emergency_stop" => RejectReason::EmergencyStop,
            "soft_close_no_buy" => RejectReason::SoftCloseBuyBlocked,
            "hard_close_in_progress" => RejectReason::HardCloseInProgress,
            "account_no_buy" => RejectReason::AccountNoBuy,
            _ => RejectReason::EmergencyStop,
        };
        let late_text = late_reason.as_str();
        sqlx::query(
            "UPDATE copy_decisions SET status = 'rejected', reason = $2 WHERE id = $1",
        )
        .bind(decision_id)
        .bind(late_text)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        let _ = notify(pool, strategy.user_id, "decision", decision_id).await;
        return Ok(decision_id);
    }

    let fill = &sim_result.fill;
    let order_id: Uuid = sqlx::query_scalar(
        "INSERT INTO orders (
            user_id, trading_account_id, decision_id, source_event_id, client_order_id, operating_mode, order_type, side,
            token_id, condition_id, limit_price, size_shares, status, position_reserved_usd, fee_reserved_usd
         ) VALUES ($1,$2,$3,$4,gen_random_uuid(),$5,'FAK',$6,$7,$8,$9,$10,'pending',$11,$12) RETURNING id",
    )
    .bind(strategy.user_id)
    .bind(strategy.trading_account_id)
    .bind(decision_id)
    .bind(event.id)
    .bind(&strategy.operating_mode)
    .bind(&event.side)
    .bind(&event.token_id)
    .bind(&event.condition_id)
    .bind(limit)
    .bind(fill.shares)
    .bind(order_reserves(side_buy, limit, fill.shares, fee_price, fee).0)
    .bind(order_reserves(side_buy, limit, fill.shares, fee_price, fee).1)
    .fetch_one(&mut *tx)
    .await?;
    if strategy.operating_mode == "paper_trading" && side_buy {
        let (pos_r, fee_r) = order_reserves(side_buy, limit, fill.shares, fee_price, fee);
        crate::strategy_accounting::reserve_order_cash(
            &mut tx,
            strategy.user_id,
            strategy.trading_account_id,
            strategy.id,
            order_id,
            pos_r + fee_r,
            &format!("order-reserve:{}", order_id),
        )
        .await?;
    }
    sqlx::query("UPDATE orders SET status = 'submitting', updated_at = now() WHERE id = $1")
        .bind(order_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    if strategy.operating_mode == "paper_trading" {
        settle_paper(
            pool,
            strategy,
            event,
            decision_id,
            order_id,
            &sim_result,
            side_buy,
        )
        .await?;
    } else if strategy.operating_mode == "live_trading" {
        submit_live(
            pool, vault, cfg, strategy, event, order_id, fill, side_buy, limit,
        )
        .await?;
    }
    let _ = notify(pool, strategy.user_id, "order", order_id).await;
    Ok(decision_id)
}

pub async fn fulfill_approved_decision(
    pool: &PgPool,
    vault: &Vault,
    cfg: &PipelineConfig,
    _data: &crate::polymarket::http::PublicClient,
    _app_cfg: &crate::config::Config,
    decision_id: Uuid,
    sim_result: crate::engine::CopyFillSim,
    fee: Option<FeeSchedule>,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("decision-fulfill:{decision_id}"))
        .execute(&mut *tx)
        .await?;
    let meta: (Uuid, Uuid) = sqlx::query_as(
        "SELECT strategy_id, source_event_id FROM copy_decisions WHERE id = $1 AND status = 'approved'",
    )
    .bind(decision_id)
    .fetch_one(&mut *tx)
    .await?;
    let strategy = sqlx::query_as::<_, StrategyRow>(
        "SELECT s.id, s.user_id, s.trading_account_id, s.source_trader_id, s.enabled, s.sizing_mode,
                s.allocation_mode, s.allocation_percent,
                s.fixed_amount_usd, s.proportion, s.multiplier, s.strategy_budget_usd, s.max_position_usd,
                s.max_exposure_usd, s.max_open_positions, s.max_slippage_bps, s.min_order_usd,
                s.allow_subminimum_paper_trades,
                s.copy_small_buys_1to1, s.small_buy_threshold_usd,
                s.copy_buy, s.copy_sell, s.cooldown_seconds, s.allowed_markets, s.created_at,
                s.operating_mode,
                a.operating_mode AS account_operating_mode,
                a.emergency_stop, a.trading_control_state AS account_trading_control_state,
                s.trading_control_state AS strategy_trading_control_state,
                a.live_confirmed_at, a.signature_type, a.funder_address,
                a.last_exchange_balance_usd, a.status AS account_status,
                r.max_order_usd, r.max_daily_loss_usd, r.max_exposure_usd AS account_max_exposure,
                r.max_slippage_bps AS account_max_slippage, r.max_orders_per_interval, r.interval_seconds,
                r.max_stale_data_seconds, r.paper_cash_usd,
                t.wallet_address
         FROM copy_strategies s
         JOIN source_traders t ON t.id = s.source_trader_id
         JOIN trading_accounts a ON a.id = s.trading_account_id
         JOIN account_risk_limits r ON r.trading_account_id = a.id
         WHERE s.id = $1",
    )
    .bind(meta.0)
    .fetch_one(&mut *tx)
    .await?;
    let event = sqlx::query_as::<_, EventRow>(
        "SELECT id, source_wallet, token_id, condition_id, outcome, market_title, market_slug, side, price, size_shares, notional_usd, usdc_size, event_ts, received_at, origin
         FROM source_events WHERE id = $1",
    )
    .bind(meta.1)
    .fetch_one(&mut *tx)
    .await?;
    let limit: Decimal = sqlx::query_scalar("SELECT limit_price FROM copy_decisions WHERE id = $1")
        .bind(decision_id)
        .fetch_one(&mut *tx)
        .await?;
    let kill: bool = sqlx::query_scalar(
        "SELECT COALESCE((value->>'enabled')::boolean, false) FROM system_settings WHERE key = 'global_kill_switch'",
    )
    .fetch_one(&mut *tx)
    .await?;
    let side_buy = event.side.eq_ignore_ascii_case("buy");
    let fee_price = conservative_fee_price(limit, event.price);
    let _ = fulfill_approved_decision_in_tx(
        pool,
        vault,
        cfg,
        &strategy,
        &event,
        tx,
        decision_id,
        side_buy,
        limit,
        fee.as_ref(),
        fee_price,
        kill,
        sim_result,
    )
    .await?;
    Ok(())
}

async fn settle_paper(
    pool: &PgPool,
    strategy: &StrategyRow,
    event: &EventRow,
    decision_id: Uuid,
    order_id: Uuid,
    sim: &crate::engine::CopyFillSim,
    side_buy: bool,
) -> anyhow::Result<()> {
    let (leader_shares, _) = crate::polymarket::canonical_source_trade(
        event.price,
        event.size_shares,
        event.notional_usd,
        event.usdc_size,
    );
    let mut tx = begin_as(pool, None, "worker").await?;
    let fill = &sim.fill;
    let fee = fill.fee_usd.unwrap_or(Decimal::ZERO);
    let fee_status = fill.fee_status;
    let fill_ctx = serde_json::to_value(&sim.context)?;
  let contributes = true;
    let fill_id: Uuid = sqlx::query_scalar(
        "INSERT INTO fills (user_id, trading_account_id, order_id, price, shares, notional_usd, fee_usd, fee_status,
                slippage_vs_source_bps, slippage_vs_limit_bps, paper_execution_mode, paper_execution_context,
                contributes_to_paper_ledger)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) RETURNING id",
    )
    .bind(strategy.user_id)
    .bind(strategy.trading_account_id)
    .bind(order_id)
    .bind(fill.vwap)
    .bind(fill.shares)
    .bind(fill.notional)
    .bind(fill.fee_usd)
    .bind(fee_status)
    .bind(slippage_bps(event.price, fill.vwap))
    .bind(slippage_bps(sim.context.copy_limit_price, fill.vwap))
    .bind(sim.paper_execution_mode.as_str())
    .bind(fill_ctx)
    .bind(contributes)
    .fetch_one(&mut *tx)
    .await?;
    if !contributes {
        crate::strategy_accounting::release_copy_order_reserves(
            &mut tx,
            order_id,
            &format!("experimental-fill-release:{}", fill_id),
        )
        .await?;
        sqlx::query(
            "UPDATE orders SET status = 'filled', submitted_at = now(), updated_at = now(),
                position_reserved_usd = 0, fee_reserved_usd = 0,
                exchange_response = $2
             WHERE id = $1",
        )
        .bind(order_id)
        .bind(json!({"paper": "synthetic_experimental", "ledger": "isolated"}))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(());
    }
    let sell_cost_released = if !side_buy {
        let pos = sqlx::query_as::<_, PositionRow>(
            "SELECT id, open_shares, avg_price, cost_basis_usd, realized_pnl_usd, source_observed_shares, pending_exit_shares, close_cycle, close_cycle_realized_baseline
             FROM positions WHERE strategy_id = $1 AND token_id = $2 AND operating_mode = 'paper_trading' FOR UPDATE",
        )
        .bind(strategy.id)
        .bind(&event.token_id)
        .fetch_optional(&mut *tx)
        .await?;
        pos.map(|p| {
            let sold = fill.shares.min(p.open_shares);
            if p.open_shares > Decimal::ZERO {
                (p.cost_basis_usd * sold / p.open_shares).round_dp(8)
            } else {
                Decimal::ZERO
            }
        })
        .unwrap_or(Decimal::ZERO)
    } else {
        Decimal::ZERO
    };
    apply_position(
        &mut tx,
        strategy,
        event,
        fill.shares,
        fill.vwap,
        fill.notional,
        fee,
        side_buy,
        leader_shares,
        Some(decision_id),
        Some(order_id),
    )
    .await?;
    if !crate::strategy_accounting::is_capital_bootstrapped(&mut tx, strategy.id).await? {
        anyhow::bail!("strategy capital not bootstrapped");
    }
    if side_buy {
        crate::strategy_accounting::apply_paper_buy_fill(
            &mut tx,
            strategy.user_id,
            strategy.trading_account_id,
            strategy.id,
            order_id,
            fill_id,
            fill.notional,
            fee,
            &format!("fill-buy:{}", fill_id),
        )
        .await?;
    } else {
        crate::strategy_accounting::apply_paper_sell_fill(
            &mut tx,
            strategy.user_id,
            strategy.trading_account_id,
            Some(strategy.id),
            order_id,
            fill_id,
            fill.notional - fee,
            sell_cost_released,
            &format!("fill-sell:{}", fill_id),
            "sell_fill",
        )
        .await?;
        crate::strategy_accounting::sync_invested_from_positions(&mut tx, strategy.id).await?;
    }
    let status = if fill.complete {
        "filled"
    } else {
        "partially_filled"
    };
    let (reserved_position, reserved_fee): (Decimal, Decimal) =
        sqlx::query_as("SELECT position_reserved_usd, fee_reserved_usd FROM orders WHERE id = $1")
            .bind(order_id)
            .fetch_one(&mut *tx)
            .await?;
    let (position_left, fee_left) = reserves_after_execution(
        fill.shares,
        fill.shares,
        reserved_position,
        reserved_fee,
        fill.fee_usd,
    );
    sqlx::query(
        "UPDATE orders SET status = $2, submitted_at = now(), updated_at = now(), exchange_response = $3,
            position_reserved_usd = $4, fee_reserved_usd = $5
         WHERE id = $1",
    )
        .bind(order_id)
        .bind(status)
        .bind(json!({"mode": "paper", "vwap": fill.vwap, "shares": fill.shares, "fee_usd": fill.fee_usd, "fee_status": fee_status}))
        .bind(position_left)
        .bind(fee_left)
        .execute(&mut *tx)
        .await?;
    if side_buy {
        let unused_reserve = position_left + fee_left;
        if unused_reserve > Decimal::ZERO {
            crate::strategy_accounting::release_order_reserve(
                &mut tx,
                strategy.user_id,
                strategy.trading_account_id,
                strategy.id,
                order_id,
                unused_reserve,
                &format!("fill-release-unused:{}", fill_id),
            )
            .await?;
        }
    }
    sqlx::query(
        "UPDATE source_traders SET copied_trade_count = copied_trade_count + 1 WHERE id = $1",
    )
    .bind(strategy.source_trader_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO timeline_events (user_id, source_event_id, decision_id, order_id, kind, message)
         VALUES ($1,$2,$3,$4,'fill',$5)",
    )
    .bind(strategy.user_id)
    .bind(event.id)
    .bind(decision_id)
    .bind(order_id)
    .bind(format!("paper {status} {shares} @ {price}", shares = fill.shares, price = fill.vwap))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn submit_live(
    pool: &PgPool,
    vault: &Vault,
    cfg: &PipelineConfig,
    strategy: &StrategyRow,
    event: &EventRow,
    order_id: Uuid,
    fill: &crate::engine::SimulatedFill,
    side_buy: bool,
    limit: Decimal,
) -> anyhow::Result<()> {
    let (leader_shares, _) = crate::polymarket::canonical_source_trade(
        event.price,
        event.size_shares,
        event.notional_usd,
        event.usdc_size,
    );
    let mut tx = begin_as(pool, None, "worker").await?;
    let cred = sqlx::query_as::<_, CredRow>(
        "SELECT ciphertext, nonce, key_id, aad FROM trading_credentials WHERE trading_account_id = $1",
    )
    .bind(strategy.trading_account_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    let Some(cred) = cred else {
        mark_order(
            pool,
            order_id,
            "rejected",
            Some("credentials_missing"),
            None,
        )
        .await?;
        return Ok(());
    };
    let blob = crate::security::SecretBlob {
        ciphertext: cred.ciphertext,
        nonce: cred.nonce,
        key_id: cred.key_id,
        aad: cred.aad.clone(),
    };
    let expected_aad = credential_aad(strategy.user_id, strategy.trading_account_id);
    if cred.aad != expected_aad {
        mark_order(
            pool,
            order_id,
            "rejected",
            Some("credential_binding_mismatch"),
            None,
        )
        .await?;
        return Ok(());
    }
    let plain = match vault.decrypt(&blob) {
        Ok(plain) => plain,
        Err(_) => {
            mark_order(
                pool,
                order_id,
                "rejected",
                Some("credential_decrypt_failed"),
                None,
            )
            .await?;
            return Ok(());
        }
    };
    let secret: Value = serde_json::from_slice(&plain).unwrap_or(json!({}));
    let Some(private_key) = secret.get("private_key").and_then(|v| v.as_str()) else {
        mark_order(
            pool,
            order_id,
            "rejected",
            Some("credentials_missing"),
            None,
        )
        .await?;
        return Ok(());
    };
    let request = LiveOrderRequest {
        private_key: SecretString::from(private_key.to_string()),
        signature_type: strategy.signature_type.clone(),
        funder: Some(strategy.funder_address.clone()),
        token_id: event.token_id.clone(),
        side_buy,
        limit_price: limit,
        shares: fill.shares,
        notional_usd: fill.notional,
    };
    let result = place_fak(&cfg.clob_host, request).await;
    let status = match result.outcome.as_str() {
        "accepted" => match result.status.as_deref() {
            Some("matched") | Some("filled") => "filled",
            Some("partial") | Some("partially_filled") => "partially_filled",
            _ => "submitted",
        },
        "unknown" => "unknown",
        _ => "rejected",
    };
    mark_order(
        pool,
        order_id,
        status,
        result.error.as_deref(),
        Some(result.exchange_order_id.unwrap_or_default())
            .filter(|s| !s.is_empty())
            .as_deref(),
    )
    .await?;
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query("UPDATE orders SET exchange_response = $2, submitted_at = CASE WHEN $3 THEN now() ELSE submitted_at END, updated_at = now() WHERE id = $1")
        .bind(order_id)
        .bind(json!({"status": result.status, "error": result.error}))
        .bind(status != "rejected")
        .execute(&mut *tx)
        .await?;
    if status == "filled" || status == "partially_filled" {
        let shares = fill.shares;
        let notional = fill.notional;
        let booked_fee = fill.fee_usd.unwrap_or(Decimal::ZERO);
        apply_position(
            &mut tx,
            strategy,
            event,
            shares,
            fill.vwap,
            notional,
            booked_fee,
            side_buy,
            leader_shares,
            None,
            Some(order_id),
        )
        .await?;
        sqlx::query(
            "INSERT INTO fills (user_id, trading_account_id, order_id, price, shares, notional_usd, fee_usd, fee_status, slippage_vs_source_bps, slippage_vs_limit_bps)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(strategy.user_id)
        .bind(strategy.trading_account_id)
        .bind(order_id)
        .bind(fill.vwap)
        .bind(shares)
        .bind(notional)
        .bind(fill.fee_usd)
        .bind(fill.fee_status)
        .bind(slippage_bps(event.price, fill.vwap))
        .bind(slippage_bps(limit, fill.vwap))
        .execute(&mut *tx)
        .await?;
        let (order_shares, reserved_position, reserved_fee): (Decimal, Decimal, Decimal) = sqlx::query_as(
            "SELECT size_shares, position_reserved_usd, fee_reserved_usd FROM orders WHERE id = $1",
        )
        .bind(order_id)
        .fetch_one(&mut *tx)
        .await?;
        let (position_left, fee_left) = reserves_after_execution(
            order_shares,
            shares,
            reserved_position,
            reserved_fee,
            fill.fee_usd,
        );
        sqlx::query("UPDATE orders SET position_reserved_usd = $2, fee_reserved_usd = $3, updated_at = now() WHERE id = $1")
            .bind(order_id)
            .bind(position_left)
            .bind(fee_left)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn cancel_open_buy_orders(
    tx: &mut crate::db::Tx<'_>,
    account_id: Uuid,
    operating_mode: &str,
) -> anyhow::Result<u64> {
    let order_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM orders
         WHERE trading_account_id = $1 AND operating_mode = $2 AND side = 'buy'
           AND status IN ('pending', 'submitting', 'submitted', 'partially_filled', 'unknown')
         FOR UPDATE",
    )
    .bind(account_id)
    .bind(operating_mode)
    .fetch_all(&mut **tx)
    .await?;
    for oid in &order_ids {
        crate::strategy_accounting::release_copy_order_reserves(
            tx,
            *oid,
            &format!("bulk-cancel-buy:{}", oid),
        )
        .await?;
    }
    let result = sqlx::query(
        "UPDATE orders SET status = 'cancelled', position_reserved_usd = 0, fee_reserved_usd = 0,
            updated_at = now(), error = COALESCE(error, 'cancelled_by_trading_control')
         WHERE trading_account_id = $1 AND operating_mode = $2 AND side = 'buy'
           AND status IN ('pending', 'submitting', 'submitted', 'partially_filled', 'unknown')",
    )
    .bind(account_id)
    .bind(operating_mode)
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected())
}

/// Cancel open copy BUY orders attributed to a single strategy (via `copy_decisions`).
pub async fn cancel_open_sell_orders_for_strategy(
    tx: &mut crate::db::Tx<'_>,
    strategy_id: Uuid,
    operating_mode: &str,
) -> anyhow::Result<u64> {
    let result = sqlx::query(
        "UPDATE orders o SET status = 'cancelled', position_reserved_usd = 0, fee_reserved_usd = 0,
            updated_at = now(), error = COALESCE(o.error, 'cancelled_by_strategy_delete')
         FROM copy_decisions d
         WHERE d.id = o.decision_id AND d.strategy_id = $1 AND o.operating_mode = $2 AND o.side = 'sell'
           AND o.status IN ('pending', 'submitting', 'submitted', 'partially_filled', 'unknown')",
    )
    .bind(strategy_id)
    .bind(operating_mode)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE positions SET pending_exit_shares = 0, updated_at = now()
         WHERE strategy_id = $1 AND operating_mode = $2",
    )
    .bind(strategy_id)
    .bind(operating_mode)
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected())
}

pub async fn cancel_open_buy_orders_for_strategy(
    tx: &mut crate::db::Tx<'_>,
    strategy_id: Uuid,
    operating_mode: &str,
) -> anyhow::Result<u64> {
    let order_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT o.id FROM orders o
         JOIN copy_decisions d ON d.id = o.decision_id
         WHERE d.strategy_id = $1 AND o.operating_mode = $2 AND o.side = 'buy'
           AND o.status IN ('pending', 'submitting', 'submitted', 'partially_filled', 'unknown')
         FOR UPDATE OF o",
    )
    .bind(strategy_id)
    .bind(operating_mode)
    .fetch_all(&mut **tx)
    .await?;
    for oid in &order_ids {
        crate::strategy_accounting::release_copy_order_reserves(
            tx,
            *oid,
            &format!("bulk-cancel-buy:{}", oid),
        )
        .await?;
    }
    let result = sqlx::query(
        "UPDATE orders SET status = 'cancelled', position_reserved_usd = 0, fee_reserved_usd = 0,
            updated_at = now(), error = COALESCE(error, 'cancelled_by_trading_control')
         WHERE id = ANY($1)",
    )
    .bind(&order_ids)
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected())
}

pub async fn cancel_open_sell_orders(
    tx: &mut crate::db::Tx<'_>,
    account_id: Uuid,
    operating_mode: &str,
) -> anyhow::Result<u64> {
    let result = sqlx::query(
        "UPDATE orders SET status = 'cancelled', position_reserved_usd = 0, fee_reserved_usd = 0,
            updated_at = now(), error = COALESCE(error, 'cancelled_for_hard_close')
         WHERE trading_account_id = $1 AND operating_mode = $2 AND side = 'sell'
           AND origin = 'copy'
           AND status IN ('pending', 'submitting', 'submitted', 'partially_filled', 'unknown')",
    )
    .bind(account_id)
    .bind(operating_mode)
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected())
}

pub async fn reset_pending_exit_shares(
    tx: &mut crate::db::Tx<'_>,
    account_id: Uuid,
    operating_mode: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE positions SET pending_exit_shares = 0, updated_at = now()
         WHERE trading_account_id = $1 AND operating_mode = $2",
    )
    .bind(account_id)
    .bind(operating_mode)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Shares reserved by active copy sell orders for a token on this account (paper).
pub async fn reserved_sell_shares(
    tx: &mut crate::db::Tx<'_>,
    account_id: Uuid,
    token_id: &str,
    operating_mode: &str,
) -> anyhow::Result<Decimal> {
    let reserved: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(GREATEST(
            o.size_shares - COALESCE((SELECT SUM(f.shares) FROM fills f WHERE f.order_id = o.id), 0),
            0
        )), 0)
         FROM orders o
         WHERE o.trading_account_id = $1 AND o.operating_mode = $2 AND o.side = 'sell'
           AND o.origin = 'copy'
           AND o.token_id = $3
           AND o.status IN ('pending', 'submitting', 'submitted', 'partially_filled', 'unknown')",
    )
    .bind(account_id)
    .bind(operating_mode)
    .bind(token_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(reserved)
}

pub fn sellable_shares(
    open_shares: Decimal,
    pending_exit: Decimal,
    reserved_sells: Decimal,
) -> Decimal {
    (open_shares - pending_exit - reserved_sells).max(Decimal::ZERO)
}

#[cfg(test)]
mod sellable_tests {
    use super::*;
    use rust_decimal::Decimal;

    #[test]
    fn sellable_respects_reservations() {
        assert_eq!(
            sellable_shares(Decimal::from(100), Decimal::from(10), Decimal::from(25)),
            Decimal::from(65)
        );
        assert_eq!(
            sellable_shares(Decimal::from(10), Decimal::from(20), Decimal::ZERO),
            Decimal::ZERO
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn paper_sell_position(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    operation_id: Uuid,
    item_id: Uuid,
    position_id: Uuid,
    token_id: &str,
    want_shares: Decimal,
    book: &Result<OrderBook, String>,
    fee: Option<&FeeSchedule>,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, Some(user_id), "worker").await?;
    let idem_key = format!("hard-close-fill:{}", item_id);
    if crate::strategy_accounting::ledger_idempotency_exists(&mut tx, &idem_key).await? {
        tx.commit().await?;
        return Ok(());
    }
    let pos: PaperSellPositionRow = sqlx::query_as(
        "SELECT id, strategy_id, source_wallet, market_title, operating_mode,
                open_shares, avg_price, cost_basis_usd, realized_pnl_usd, source_observed_shares, pending_exit_shares, close_cycle, close_cycle_realized_baseline
         FROM positions WHERE id = $1 AND trading_account_id = $2 AND user_id = $3 FOR UPDATE",
    )
    .bind(position_id)
    .bind(account_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if pos.open_shares <= Decimal::ZERO {
        sqlx::query(
            "UPDATE hard_close_items SET status = 'closed', closed_shares = requested_shares, remaining_shares = 0, updated_at = now() WHERE id = $1",
        )
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(());
    }
    let reserved = reserved_sell_shares(&mut tx, account_id, token_id, "paper_trading").await?;
    let available = sellable_shares(pos.open_shares, pos.pending_exit_shares, reserved);
    if available <= Decimal::ZERO {
        anyhow::bail!(
            "no sellable shares (open={}, pending_exit={}, reserved_sells={})",
            pos.open_shares,
            pos.pending_exit_shares,
            reserved
        );
    }
    let sell_shares = want_shares.min(available);
    let book = book.as_ref().map_err(|e| anyhow::anyhow!("{}", e))?;
    if book.min_order_size > Decimal::ZERO && sell_shares < book.min_order_size {
        anyhow::bail!(
            "below_minimum_order_size: sellable {} < market min {}",
            sell_shares,
            book.min_order_size
        );
    }
    let slip = 500;
    let tick = book.tick;
    let ref_price = book
        .bids
        .first()
        .map(|l| l.price)
        .unwrap_or(pos.avg_price)
        .max(tick);
    let limit = limit_price(false, ref_price, slip, tick);
    let fill =
        walk_book(book, false, limit, sell_shares, fee, pos.operating_mode != "paper_trading")
            .map_err(|r| anyhow::anyhow!(r.as_str()))?;
    let order_id: Uuid = sqlx::query_scalar(
        "INSERT INTO orders (
            user_id, trading_account_id, client_order_id, operating_mode, order_type, side,
            token_id, limit_price, size_shares, status, origin, position_id, position_reserved_usd, fee_reserved_usd
         ) VALUES ($1,$2,gen_random_uuid(),'paper_trading','FAK','sell',$3,$4,$5,'filled','hard_close',$6,0,0)
         RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(token_id)
    .bind(limit)
    .bind(fill.shares)
    .bind(position_id)
    .fetch_one(&mut *tx)
    .await?;
    let fee_amount = fill.fee_usd.unwrap_or(Decimal::ZERO);
    let fill_id: Uuid = sqlx::query_scalar(
        "INSERT INTO fills (user_id, trading_account_id, order_id, price, shares, notional_usd, fee_usd, fee_status)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(order_id)
    .bind(fill.vwap)
    .bind(fill.shares)
    .bind(fill.notional)
    .bind(fill.fee_usd)
    .bind(fill.fee_status)
    .fetch_one(&mut *tx)
    .await?;
    let shares_before = pos.open_shares;
    let cost_before = pos.cost_basis_usd;
    let avg_entry = pos.avg_price;
    let realized_before = pos.realized_pnl_usd;
    let sold = fill.shares.min(pos.open_shares);
    let released = if pos.open_shares > Decimal::ZERO {
        (pos.cost_basis_usd * sold / pos.open_shares).round_dp(8)
    } else {
        Decimal::ZERO
    };
    let pnl = fill.notional - released - fee_amount;
    let remaining = pos.open_shares - sold;
    sqlx::query(
        "UPDATE positions SET open_shares = $2, cost_basis_usd = $3, realized_pnl_usd = realized_pnl_usd + $4, updated_at = now() WHERE id = $1",
    )
    .bind(position_id)
    .bind(remaining)
    .bind((pos.cost_basis_usd - released).max(Decimal::ZERO))
    .bind(pnl)
    .execute(&mut *tx)
    .await?;
    crate::position_close::set_order_position_open_shares_after(&mut tx, order_id, remaining)
        .await?;
    if remaining == Decimal::ZERO {
        let cycle_realized = crate::position_close::cycle_realized_pnl(
            realized_before + pnl,
            pos.close_cycle_realized_baseline,
        );
        sqlx::query(
            "INSERT INTO closed_positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, market_title, operating_mode, closed_shares, realized_pnl_usd)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(user_id)
        .bind(account_id)
        .bind(pos.strategy_id)
        .bind(&pos.source_wallet)
        .bind(token_id)
        .bind(&pos.market_title)
        .bind(&pos.operating_mode)
        .bind(shares_before)
        .bind(cycle_realized)
        .execute(&mut *tx)
        .await?;
        if let Some(sid) = pos.strategy_id {
            let meta = crate::position_close::CloseSnapshotMeta {
                user_id,
                trading_account_id: account_id,
                strategy_id: sid,
                position_id,
                token_id: token_id.to_string(),
                operating_mode: pos.operating_mode.clone(),
                market_title: pos.market_title.clone(),
                outcome: None,
                reason: crate::position_close::CloseReason::HardClose,
                close_cycle: pos.close_cycle,
                realized_pnl_usd: realized_before + pnl,
                fallback_closed_shares: shares_before,
                fallback_cost_basis: cost_before,
                fallback_avg_entry: avg_entry,
            };
            let _ = crate::position_close::record_close_if_full_exit(&mut tx, meta, None).await?;
        }
    }
    let use_ledger = match pos.strategy_id {
        Some(sid) => crate::strategy_accounting::is_capital_bootstrapped(&mut tx, sid).await?,
        None => false,
    };
    if use_ledger {
        let sid = pos.strategy_id.expect("strategy_id");
        crate::strategy_accounting::apply_paper_sell_fill(
            &mut tx,
            user_id,
            account_id,
            Some(sid),
            order_id,
            fill_id,
            fill.notional - fee_amount,
            released,
            &idem_key,
            "hard_close_sell",
        )
        .await?;
        crate::strategy_accounting::sync_invested_from_positions(&mut tx, sid).await?;
    }
    let item_status =
        if remaining <= Decimal::ZERO || remaining <= Decimal::new(1, 4) || fill.complete {
            "closed"
        } else {
            "partial"
        };
    let item_remaining = remaining.max(Decimal::ZERO);
    sqlx::query(
        "UPDATE hard_close_items SET status = $2, closed_shares = closed_shares + $3, remaining_shares = $4,
            order_id = $5, updated_at = now() WHERE id = $1",
    )
    .bind(item_id)
    .bind(item_status)
    .bind(sold)
    .bind(item_remaining)
    .bind(order_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let _ = operation_id;
    Ok(())
}

async fn mark_order(
    pool: &PgPool,
    order_id: Uuid,
    status: &str,
    error: Option<&str>,
    exchange_id: Option<&str>,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    if matches!(status, "cancelled" | "rejected" | "failed") {
        crate::strategy_accounting::release_copy_order_reserves(
            &mut tx,
            order_id,
            &format!("mark-order-{status}:{order_id}"),
        )
        .await?;
    }
    sqlx::query(
        "UPDATE orders SET status = $2, error = $3, exchange_order_id = COALESCE($4, exchange_order_id), updated_at = now(),
            position_reserved_usd = CASE WHEN $2 IN ('cancelled', 'rejected', 'failed') THEN 0 ELSE position_reserved_usd END,
            fee_reserved_usd = CASE WHEN $2 IN ('cancelled', 'rejected', 'failed') THEN 0 ELSE fee_reserved_usd END
         WHERE id = $1",
    )
        .bind(order_id)
        .bind(status)
        .bind(error.map(redact))
        .bind(exchange_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn apply_position(
    tx: &mut crate::db::Tx<'_>,
    strategy: &StrategyRow,
    event: &EventRow,
    shares: Decimal,
    price: Decimal,
    notional: Decimal,
    fee: Decimal,
    side_buy: bool,
    source_size: Decimal,
    decision_id: Option<Uuid>,
    order_id: Option<Uuid>,
) -> anyhow::Result<()> {
    if side_buy {
        sqlx::query(
            "INSERT INTO positions (
                user_id, trading_account_id, strategy_id, source_wallet, token_id, condition_id, outcome, market_title,
                operating_mode, open_shares, avg_price, cost_basis_usd, source_observed_shares
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
             ON CONFLICT (strategy_id, token_id, operating_mode) WHERE (strategy_id IS NOT NULL)
             DO UPDATE SET
                close_cycle = CASE
                    WHEN positions.open_shares = 0 AND EXCLUDED.open_shares > 0
                    THEN positions.close_cycle + 1
                    ELSE positions.close_cycle
                END,
                close_cycle_realized_baseline = CASE
                    WHEN positions.open_shares = 0 AND EXCLUDED.open_shares > 0
                    THEN positions.realized_pnl_usd
                    ELSE positions.close_cycle_realized_baseline
                END,
                open_shares = positions.open_shares + EXCLUDED.open_shares,
                cost_basis_usd = positions.cost_basis_usd + EXCLUDED.cost_basis_usd,
                avg_price = CASE WHEN positions.open_shares + EXCLUDED.open_shares = 0 THEN 0
                    ELSE (positions.cost_basis_usd + EXCLUDED.cost_basis_usd) / (positions.open_shares + EXCLUDED.open_shares) END,
                source_observed_shares = positions.source_observed_shares + EXCLUDED.source_observed_shares,
                market_title = COALESCE(EXCLUDED.market_title, positions.market_title),
                updated_at = now()",
        )
        .bind(strategy.user_id)
        .bind(strategy.trading_account_id)
        .bind(strategy.id)
        .bind(&strategy.wallet_address)
        .bind(&event.token_id)
        .bind(&event.condition_id)
        .bind(&event.outcome)
        .bind(&event.market_title)
        .bind(&strategy.operating_mode)
        .bind(shares)
        .bind(price)
        .bind(notional + fee)
        .bind(source_size)
        .execute(&mut **tx)
        .await?;
    } else {
        let pos = sqlx::query_as::<_, PositionRow>(
            "SELECT id, open_shares, avg_price, cost_basis_usd, realized_pnl_usd, source_observed_shares, pending_exit_shares, close_cycle, close_cycle_realized_baseline
             FROM positions
             WHERE strategy_id = $1 AND token_id = $2 AND operating_mode = $3
             FOR UPDATE",
        )
        .bind(strategy.id)
        .bind(&event.token_id)
        .bind(&strategy.operating_mode)
        .fetch_one(&mut **tx)
        .await?;
        let shares_before = pos.open_shares;
        let cost_before = pos.cost_basis_usd;
        let avg_entry = pos.avg_price;
        let realized_before = pos.realized_pnl_usd;
        let sold = shares.min(pos.open_shares);
        let released = if pos.open_shares > Decimal::ZERO {
            (pos.cost_basis_usd * sold / pos.open_shares).round_dp(8)
        } else {
            Decimal::ZERO
        };
        let pnl = notional - released - fee;
        let remaining = pos.open_shares - sold;
        let cumulative_realized = realized_before + pnl;
        let cycle_realized = crate::position_close::cycle_realized_pnl(
            cumulative_realized,
            pos.close_cycle_realized_baseline,
        );
        sqlx::query(
            "UPDATE positions SET open_shares = $2, cost_basis_usd = $3, realized_pnl_usd = realized_pnl_usd + $4,
             source_observed_shares = GREATEST(source_observed_shares - $5, 0), updated_at = now() WHERE id = $1",
        )
        .bind(pos.id)
        .bind(remaining)
        .bind((pos.cost_basis_usd - released).max(Decimal::ZERO))
        .bind(pnl)
        .bind(source_size)
        .execute(&mut **tx)
        .await?;
        if let Some(oid) = order_id {
            crate::position_close::set_order_position_open_shares_after(tx, oid, remaining).await?;
        }
        if remaining == Decimal::ZERO {
            sqlx::query(
                "INSERT INTO closed_positions (user_id, trading_account_id, strategy_id, source_wallet, token_id, market_title, operating_mode, closed_shares, realized_pnl_usd)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            )
            .bind(strategy.user_id)
            .bind(strategy.trading_account_id)
            .bind(strategy.id)
            .bind(&strategy.wallet_address)
            .bind(&event.token_id)
            .bind(&event.market_title)
            .bind(&strategy.operating_mode)
            .bind(shares_before)
            .bind(cycle_realized)
            .execute(&mut **tx)
            .await?;
            if decision_id.is_some() {
                let meta = crate::position_close::CloseSnapshotMeta {
                    user_id: strategy.user_id,
                    trading_account_id: strategy.trading_account_id,
                    strategy_id: strategy.id,
                    position_id: pos.id,
                    token_id: event.token_id.clone(),
                    operating_mode: strategy.operating_mode.clone(),
                    market_title: event.market_title.clone(),
                    outcome: event.outcome.clone(),
                    reason: crate::position_close::CloseReason::LeaderSell,
                    close_cycle: pos.close_cycle,
                    realized_pnl_usd: realized_before + pnl,
                    fallback_closed_shares: shares_before,
                    fallback_cost_basis: cost_before,
                    fallback_avg_entry: avg_entry,
                };
                let _ =
                    crate::position_close::record_close_if_full_exit(tx, meta, None).await?;
            }
        }
    }
    Ok(())
}

pub async fn reconcile_live_orders(
    pool: &PgPool,
    vault: &Vault,
    clob_host: &str,
    wait_seconds: i64,
) -> anyhow::Result<()> {
    type LiveReconcileRow = (
        Uuid,
        String,
        DateTime<Utc>,
        Option<String>,
        Uuid,
        Uuid,
        String,
        String,
    );
    let mut tx = begin_as(pool, None, "worker").await?;
    let rows: Vec<LiveReconcileRow> = sqlx::query_as(
        "SELECT o.id, o.status, o.updated_at, o.exchange_order_id, o.trading_account_id, o.user_id, a.signature_type, a.funder_address
         FROM orders o
         JOIN trading_accounts a ON a.id = o.trading_account_id
         WHERE o.operating_mode = 'live_trading' AND o.status IN ('submitting', 'unknown', 'submitted')",
    )
    .fetch_all(&mut *tx)
    .await?;
    drop(tx);
    for (id, status, updated, exchange_id, account_id, user_id, signature_type, funder) in rows {
        let aged = Utc::now() - updated > Duration::seconds(wait_seconds);
        let remote = match exchange_id.as_deref() {
            Some(exchange_id) => {
                observe_saved_order(
                    pool,
                    vault,
                    clob_host,
                    account_id,
                    user_id,
                    &signature_type,
                    &funder,
                    exchange_id,
                )
                .await
            }
            None => RemoteObservation::Unreachable,
        };
        let next = reconcile_live_status(&status, aged, remote);
        if next != status {
            mark_order(
                pool,
                id,
                &next,
                Some("exchange state applied; the order was not resent"),
                None,
            )
            .await?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn observe_saved_order(
    pool: &PgPool,
    vault: &Vault,
    clob_host: &str,
    account_id: Uuid,
    user_id: Uuid,
    signature_type: &str,
    funder: &str,
    exchange_id: &str,
) -> RemoteObservation {
    let Ok(mut tx) = begin_as(pool, None, "worker").await else {
        return RemoteObservation::Unreachable;
    };
    let cred = sqlx::query_as::<_, CredRow>(
        "SELECT ciphertext, nonce, key_id, aad FROM trading_credentials WHERE trading_account_id = $1 AND user_id = $2",
    )
    .bind(account_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .ok()
    .flatten();
    drop(tx);
    let Some(cred) = cred else {
        return RemoteObservation::Unreachable;
    };
    let blob = crate::security::SecretBlob {
        ciphertext: cred.ciphertext,
        nonce: cred.nonce,
        key_id: cred.key_id,
        aad: cred.aad,
    };
    let Ok(plain) = vault.decrypt(&blob) else {
        return RemoteObservation::Unreachable;
    };
    let Ok(secret) = serde_json::from_slice::<serde_json::Value>(&plain) else {
        return RemoteObservation::Unreachable;
    };
    let Some(private_key) = secret.get("private_key").and_then(|v| v.as_str()) else {
        return RemoteObservation::Unreachable;
    };
    match crate::polymarket::live::observe_remote_order(
        clob_host,
        private_key,
        signature_type,
        Some(funder),
        exchange_id,
    )
    .await
    {
        Ok(observation) => observation,
        Err(error) => {
            let lower = error.to_ascii_lowercase();
            if lower.contains("404") || lower.contains("not found") {
                RemoteObservation::NotFound
            } else {
                RemoteObservation::Unreachable
            }
        }
    }
}

pub async fn notify(pool: &PgPool, user_id: Uuid, kind: &str, id: Uuid) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query("SELECT pg_notify('pom_activity', $1)")
        .bind(json!({"user_id": user_id, "kind": kind, "id": id}).to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

fn market_allowed(allowed: &Value, event: &EventRow) -> bool {
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

#[derive(sqlx::FromRow)]
struct EventRow {
    id: Uuid,
    source_wallet: String,
    token_id: String,
    condition_id: Option<String>,
    outcome: Option<String>,
    market_title: Option<String>,
    market_slug: Option<String>,
    side: String,
    price: Decimal,
    size_shares: Decimal,
    notional_usd: Decimal,
    usdc_size: Option<Decimal>,
    event_ts: DateTime<Utc>,
    received_at: DateTime<Utc>,
    origin: String,
}

#[allow(dead_code)] // wide sqlx row; not every selected column is used in every code path
#[derive(sqlx::FromRow)]
struct StrategyRow {
    id: Uuid,
    user_id: Uuid,
    trading_account_id: Uuid,
    source_trader_id: Uuid,
    enabled: bool,
    sizing_mode: String,
    allocation_mode: String,
    allocation_percent: Option<Decimal>,
    fixed_amount_usd: Option<Decimal>,
    proportion: Option<Decimal>,
    multiplier: Option<Decimal>,
    strategy_budget_usd: Decimal,
    max_position_usd: Decimal,
    max_exposure_usd: Decimal,
    max_open_positions: i32,
    max_slippage_bps: i32,
    min_order_usd: Decimal,
    allow_subminimum_paper_trades: bool,
    copy_small_buys_1to1: bool,
    small_buy_threshold_usd: Option<Decimal>,
    copy_buy: bool,
    copy_sell: bool,
    cooldown_seconds: i32,
    allowed_markets: Value,
    created_at: DateTime<Utc>,
    operating_mode: String,
    account_operating_mode: String,
    emergency_stop: bool,
    account_trading_control_state: String,
    strategy_trading_control_state: String,
    live_confirmed_at: Option<DateTime<Utc>>,
    signature_type: String,
    funder_address: String,
    last_exchange_balance_usd: Option<Decimal>,
    account_status: String,
    max_order_usd: Decimal,
    max_daily_loss_usd: Decimal,
    account_max_exposure: Decimal,
    account_max_slippage: i32,
    max_orders_per_interval: i32,
    interval_seconds: i32,
    max_stale_data_seconds: i32,
    paper_cash_usd: Decimal,
    wallet_address: String,
}

#[allow(dead_code)]
#[derive(sqlx::FromRow)]
struct PositionRow {
    id: Uuid,
    open_shares: Decimal,
    avg_price: Decimal,
    cost_basis_usd: Decimal,
    realized_pnl_usd: Decimal,
    source_observed_shares: Decimal,
    pending_exit_shares: Decimal,
    close_cycle: i64,
    close_cycle_realized_baseline: Decimal,
}

#[derive(sqlx::FromRow)]
struct PaperSellPositionRow {
    #[allow(dead_code)]
    id: Uuid,
    strategy_id: Option<Uuid>,
    source_wallet: String,
    market_title: Option<String>,
    operating_mode: String,
    open_shares: Decimal,
    avg_price: Decimal,
    cost_basis_usd: Decimal,
    #[allow(dead_code)]
    realized_pnl_usd: Decimal,
    #[allow(dead_code)]
    source_observed_shares: Decimal,
    pending_exit_shares: Decimal,
    close_cycle: i64,
    close_cycle_realized_baseline: Decimal,
}

#[derive(sqlx::FromRow)]
struct CredRow {
    ciphertext: Vec<u8>,
    nonce: Vec<u8>,
    key_id: String,
    aad: String,
}
