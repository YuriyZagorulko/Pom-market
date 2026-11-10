use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::begin_as;
use crate::engine::{
    available_after_reserve, conservative_fee_price, limit_price, order_reserves, plan_exit,
    reconcile_live_status, reserves_after_execution, slippage_bps, target_notional, trace_buy_size,
    truncate_lot, walk_book, FeeSchedule, OrderBook, RejectReason, RemoteObservation, SizingInput,
    SizingMode, EXCHANGE_MIN_NOTIONAL,
};
use crate::polymarket::live::{place_fak, LiveOrderRequest};
use crate::polymarket::RawTrade;
use crate::security::{credential_aad, redact, Vault};
use secrecy::SecretString;

#[async_trait]
pub trait Books: Send + Sync {
    async fn book(&self, token_id: &str) -> Result<OrderBook, String>;
    async fn fee(&self, token_id: &str) -> Option<FeeSchedule>;
}

pub struct PipelineConfig {
    pub clob_host: String,
    pub confirm_live: bool,
    pub default_fee_rate: Decimal,
    pub default_fee_exponent: u32,
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
    event_id: Uuid,
) -> anyhow::Result<Vec<Uuid>> {
    let mut tx = begin_as(pool, None, "worker").await?;
    sqlx::query("UPDATE source_events SET status = 'processing' WHERE id = $1 AND status IN ('normalized', 'detected')")
        .bind(event_id)
        .execute(&mut *tx)
        .await?;
    let event = sqlx::query_as::<_, EventRow>(
        "SELECT id, source_wallet, token_id, condition_id, outcome, market_title, market_slug, side, price, size_shares, notional_usd, event_ts, received_at
         FROM source_events WHERE id = $1",
    )
    .bind(event_id)
    .fetch_one(&mut *tx)
    .await?;
    let strategies = sqlx::query_as::<_, StrategyRow>(
        "SELECT s.id, s.user_id, s.trading_account_id, s.source_trader_id, s.enabled, s.sizing_mode,
                s.fixed_amount_usd, s.proportion, s.multiplier, s.strategy_budget_usd, s.max_position_usd,
                s.max_exposure_usd, s.max_open_positions, s.max_slippage_bps, s.min_order_usd,
                s.copy_buy, s.copy_sell, s.cooldown_seconds, s.allowed_markets, s.created_at,
                a.operating_mode, a.emergency_stop, a.live_confirmed_at, a.signature_type, a.funder_address,
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
        let id = decide_one(pool, vault, cfg, &event, &strategy, &book, fee.as_ref()).await?;
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

async fn decide_one(
    pool: &PgPool,
    vault: &Vault,
    cfg: &PipelineConfig,
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
    let strategy_exposure: Decimal = sqlx::query_scalar(
        "SELECT COALESCE((SELECT SUM(cost_basis_usd) FROM positions
            WHERE trading_account_id = $1 AND lower(source_wallet) = $2 AND operating_mode = $3 AND open_shares > 0), 0)
         + COALESCE((SELECT SUM(o.position_reserved_usd + o.fee_reserved_usd) FROM orders o
            JOIN source_events e ON e.id = o.source_event_id
            WHERE o.trading_account_id = $1 AND lower(e.source_wallet) = $2 AND o.operating_mode = $3
              AND o.status IN ('pending', 'submitting', 'submitted', 'partially_filled', 'unknown')), 0)",
    )
    .bind(strategy.trading_account_id)
    .bind(&strategy.wallet_address)
    .bind(&strategy.operating_mode)
    .fetch_one(&mut *tx)
    .await?;
    let open_positions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM positions
         WHERE trading_account_id = $1 AND lower(source_wallet) = $2 AND operating_mode = $3 AND open_shares > 0",
    )
    .bind(strategy.trading_account_id)
    .bind(&strategy.wallet_address)
    .bind(&strategy.operating_mode)
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
    } else if strategy.emergency_stop {
        reason = Some(RejectReason::EmergencyStop);
    } else if strategy.operating_mode == "monitoring" {
        reason = Some(RejectReason::MonitoringMode);
    } else if strategy.operating_mode == "live_trading"
        && (kill || !cfg.confirm_live || strategy.live_confirmed_at.is_none())
    {
        reason = Some(if kill {
            RejectReason::KillSwitch
        } else {
            RejectReason::LiveNotConfirmed
        });
    } else if side_buy && !strategy.copy_buy || !side_buy && !strategy.copy_sell {
        reason = Some(RejectReason::CopySideDisabled);
    } else if Utc::now().signed_duration_since(event.event_ts)
        > Duration::seconds(strategy.max_stale_data_seconds as i64)
    {
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
    }

    let cash: Decimal = if strategy.operating_mode == "paper_trading" {
        sqlx::query_scalar(
            "SELECT paper_cash_usd FROM account_risk_limits WHERE trading_account_id = $1",
        )
        .bind(strategy.trading_account_id)
        .fetch_one(&mut *tx)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT COALESCE(last_exchange_balance_usd, 0) FROM trading_accounts WHERE id = $1",
        )
        .bind(strategy.trading_account_id)
        .fetch_one(&mut *tx)
        .await?
    };
    let open_reserve: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(position_reserved_usd + fee_reserved_usd), 0) FROM orders
         WHERE trading_account_id = $1 AND operating_mode = $2
           AND status IN ('pending', 'submitting', 'submitted', 'partially_filled', 'unknown')",
    )
    .bind(strategy.trading_account_id)
    .bind(&strategy.operating_mode)
    .fetch_one(&mut *tx)
    .await?;
    let available = available_after_reserve(cash, open_reserve);
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
    if reason.is_none() && side_buy {
        if open_positions >= strategy.max_open_positions as i64 {
            let existing_token: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM positions
                 WHERE trading_account_id = $1 AND lower(source_wallet) = $2 AND token_id = $3 AND operating_mode = $4 AND open_shares > 0",
            )
            .bind(strategy.trading_account_id)
            .bind(&strategy.wallet_address)
            .bind(&event.token_id)
            .bind(&strategy.operating_mode)
            .fetch_one(&mut *tx)
            .await?;
            if existing_token == 0 {
                reason = Some(RejectReason::MaxPositions);
            }
        }
        if reason.is_none() {
            let sizing = SizingInput {
                mode: parse_mode(&strategy.sizing_mode),
                fixed_amount_usd: strategy.fixed_amount_usd,
                proportion: strategy.proportion,
                multiplier: strategy.multiplier,
                source_notional: event.notional_usd,
                max_order_usd: strategy.max_order_usd.min(strategy.max_position_usd),
                min_order_usd: strategy.min_order_usd,
                strategy_remaining_usd: (strategy.strategy_budget_usd - strategy_exposure)
                    .max(Decimal::ZERO),
                account_exposure_remaining_usd: (strategy
                    .account_max_exposure
                    .min(strategy.max_exposure_usd)
                    - exposure)
                    .max(Decimal::ZERO),
                available_balance_usd: available,
            };
            match target_notional(&sizing) {
                Ok(notional) => {
                    target_notional_usd = notional;
                    let (min_order_size, liquidity) = match &book {
                        Ok(book) => (
                            book.min_order_size,
                            book.asks
                                .iter()
                                .fold(Decimal::ZERO, |sum, level| sum + level.size),
                        ),
                        Err(_) => (Decimal::ZERO, Decimal::MAX),
                    };
                    let spend_cap = (strategy.strategy_budget_usd - strategy_exposure)
                        .min(
                            strategy.account_max_exposure.min(strategy.max_exposure_usd) - exposure,
                        )
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
                    target_shares = trace.sdk_normalized_shares;
                    if trace.fee_unknown {
                        reason = Some(RejectReason::FeeUnknown);
                    } else if trace.below_min_order_size
                        || target_shares * limit < EXCHANGE_MIN_NOTIONAL
                    {
                        reason = Some(if spend_cap < notional {
                            RejectReason::StrategyBudget
                        } else {
                            RejectReason::BelowMinimum
                        });
                    }
                    size_trace = Some(trace);
                }
                Err(err) => reason = Some(err),
            }
        }
    }

    let position = sqlx::query_as::<_, PositionRow>(
        "SELECT id, open_shares, avg_price, cost_basis_usd, realized_pnl_usd, source_observed_shares, pending_exit_shares
         FROM positions
         WHERE trading_account_id = $1 AND lower(source_wallet) = $2 AND token_id = $3 AND operating_mode = $4",
    )
    .bind(strategy.trading_account_id)
    .bind(&strategy.wallet_address)
    .bind(&event.token_id)
    .bind(&strategy.operating_mode)
    .fetch_optional(&mut *tx)
    .await?;

    if reason.is_none() && fee.is_none() {
        reason = Some(RejectReason::FeeUnknown);
    }

    if reason.is_none() && !side_buy {
        let pos = position.as_ref();
        let plan = plan_exit(
            pos.map(|p| p.open_shares).unwrap_or(Decimal::ZERO),
            pos.map(|p| p.source_observed_shares)
                .unwrap_or(Decimal::ZERO),
            event.size_shares,
            pos.map(|p| p.pending_exit_shares).unwrap_or(Decimal::ZERO),
            limit,
            strategy.min_order_usd,
        );
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
    if reason.is_none() {
        match book {
            Ok(book) => match walk_book(book, side_buy, limit, target_shares, fee) {
                Ok(fill) => {
                    book_price = Some(fill.vwap);
                    if slippage_bps(event.price, fill.vwap) > slip {
                        reason = Some(RejectReason::Slippage);
                    } else {
                        sim = Some(fill);
                    }
                }
                Err(err) => reason = Some(err),
            },
            Err(_) => reason = Some(RejectReason::NoLiquidity),
        }
    }

    let approved = reason.is_none();
    let status = if approved {
        "approved"
    } else if matches!(
        reason,
        Some(RejectReason::MonitoringMode) | Some(RejectReason::StrategyDisabled)
    ) {
        "skipped"
    } else {
        "rejected"
    };
    let reason_text = reason
        .map(|r| r.as_str().to_string())
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
    .bind(json!({
        "exposure_usd": exposure,
        "strategy_exposure_usd": strategy_exposure,
        "open_positions": open_positions,
        "available_balance_usd": available,
        "kill_switch": kill,
        "requested_notional_usd": target_notional_usd,
        "grid_shares": target_shares,
        "limit_price": limit,
        "reserved_notional_usd": (target_shares * limit).round_dp(2),
        "strategy_budget_usd": strategy.strategy_budget_usd,
        "remaining_strategy_budget_usd": (strategy.strategy_budget_usd - strategy_exposure).max(Decimal::ZERO),
        "size_trace": size_trace
    }))
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

    let fill = sim.expect("approved simulation");
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
            &fill,
            side_buy,
        )
        .await?;
    } else if strategy.operating_mode == "live_trading" {
        submit_live(
            pool, vault, cfg, strategy, event, order_id, &fill, side_buy, limit,
        )
        .await?;
    }
    let _ = notify(pool, strategy.user_id, "order", order_id).await;
    Ok(decision_id)
}

async fn settle_paper(
    pool: &PgPool,
    strategy: &StrategyRow,
    event: &EventRow,
    decision_id: Uuid,
    order_id: Uuid,
    fill: &crate::engine::SimulatedFill,
    side_buy: bool,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
    let fee = fill.fee_usd.unwrap_or(Decimal::ZERO);
    let fee_status = fill.fee_status;
    sqlx::query(
        "INSERT INTO fills (user_id, trading_account_id, order_id, price, shares, notional_usd, fee_usd, fee_status, slippage_vs_source_bps, slippage_vs_limit_bps)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
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
    .bind(slippage_bps(event.price, fill.vwap))
    .execute(&mut *tx)
    .await?;
    apply_position(
        &mut tx,
        strategy,
        event,
        fill.shares,
        fill.vwap,
        fill.notional,
        fee,
        side_buy,
        event.size_shares,
    )
    .await?;
    let cash_delta = if side_buy {
        -(fill.notional + fee)
    } else {
        fill.notional - fee
    };
    sqlx::query("UPDATE account_risk_limits SET paper_cash_usd = paper_cash_usd + $2, updated_at = now() WHERE trading_account_id = $1")
        .bind(strategy.trading_account_id)
        .bind(cash_delta)
        .execute(&mut *tx)
        .await?;
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
            event.size_shares,
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

async fn mark_order(
    pool: &PgPool,
    order_id: Uuid,
    status: &str,
    error: Option<&str>,
    exchange_id: Option<&str>,
) -> anyhow::Result<()> {
    let mut tx = begin_as(pool, None, "worker").await?;
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
) -> anyhow::Result<()> {
    if side_buy {
        sqlx::query(
            "INSERT INTO positions (
                user_id, trading_account_id, source_wallet, token_id, condition_id, outcome, market_title,
                operating_mode, open_shares, avg_price, cost_basis_usd, source_observed_shares
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
             ON CONFLICT (trading_account_id, source_wallet, token_id, operating_mode)
             DO UPDATE SET
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
            "SELECT id, open_shares, avg_price, cost_basis_usd, realized_pnl_usd, source_observed_shares, pending_exit_shares
             FROM positions
             WHERE trading_account_id = $1 AND lower(source_wallet) = $2 AND token_id = $3 AND operating_mode = $4
             FOR UPDATE",
        )
        .bind(strategy.trading_account_id)
        .bind(&strategy.wallet_address)
        .bind(&event.token_id)
        .bind(&strategy.operating_mode)
        .fetch_one(&mut **tx)
        .await?;
        let sold = shares.min(pos.open_shares);
        let released = if pos.open_shares > Decimal::ZERO {
            (pos.cost_basis_usd * sold / pos.open_shares).round_dp(8)
        } else {
            Decimal::ZERO
        };
        let pnl = notional - released - fee;
        let remaining = pos.open_shares - sold;
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
        if remaining == Decimal::ZERO {
            sqlx::query(
                "INSERT INTO closed_positions (user_id, trading_account_id, source_wallet, token_id, market_title, operating_mode, closed_shares, realized_pnl_usd)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
            )
            .bind(strategy.user_id)
            .bind(strategy.trading_account_id)
            .bind(&strategy.wallet_address)
            .bind(&event.token_id)
            .bind(&event.market_title)
            .bind(&strategy.operating_mode)
            .bind(sold)
            .bind(pnl)
            .execute(&mut **tx)
            .await?;
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

fn parse_mode(value: &str) -> SizingMode {
    match value {
        "proportional" => SizingMode::Proportional,
        "multiplier" => SizingMode::Multiplier,
        _ => SizingMode::FixedAmount,
    }
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
    event_ts: DateTime<Utc>,
    received_at: DateTime<Utc>,
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
    fixed_amount_usd: Option<Decimal>,
    proportion: Option<Decimal>,
    multiplier: Option<Decimal>,
    strategy_budget_usd: Decimal,
    max_position_usd: Decimal,
    max_exposure_usd: Decimal,
    max_open_positions: i32,
    max_slippage_bps: i32,
    min_order_usd: Decimal,
    copy_buy: bool,
    copy_sell: bool,
    cooldown_seconds: i32,
    allowed_markets: Value,
    created_at: DateTime<Utc>,
    operating_mode: String,
    emergency_stop: bool,
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
}

#[derive(sqlx::FromRow)]
struct CredRow {
    ciphertext: Vec<u8>,
    nonce: Vec<u8>,
    key_id: String,
    aad: String,
}
