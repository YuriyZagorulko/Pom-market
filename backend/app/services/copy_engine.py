"""Copy-Trading Engine: sizing modes, risk controls, virtual position attribution, and paper/live order book execution."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from decimal import Decimal, ROUND_CEILING, ROUND_DOWN, ROUND_FLOOR
import math
from typing import Any, Optional

from sqlalchemy import func, select
from sqlalchemy.orm import Session

from backend.app.models import (
    AccountBalance,
    AccountSourceTrader,
    CopyStrategy,
    ExecutionJob,
    ExecutionJobStatus,
    ExecutionMode,
    Fill,
    OperatingMode,
    Order,
    OrderStatus,
    RiskLimit,
    SourceTradeEvent,
    SourceTrader,
    SystemSetting,
    TradeSide,
    TradingAccount,
    VirtualPosition,
)
from backend.app.services.metrics_service import GLOBAL_METRICS, PlatformMetricsRegistry

EXCHANGE_MIN_NOTIONAL_USD = Decimal("1.00")
SIZE_DECIMALS_MAX = 5
AMOUNT_DECIMALS_MAX = 2


@dataclass(frozen=True)
class OrderBookLevel:
    price: Decimal
    size: Decimal


@dataclass
class OrderBookSnapshot:
    """Normalized order book with asks sorted ascending (best first) and bids descending (best first)."""

    token_id: str
    bids: list[OrderBookLevel]
    asks: list[OrderBookLevel]
    tick_size: Decimal = Decimal("0.01")
    min_order_size: Decimal = Decimal("1.0")
    fee_rate: Decimal = Decimal("0.02")
    fee_exponent: Decimal = Decimal("1.0")
    neg_risk: bool = False

    @classmethod
    def from_raw(cls, token_id: str, raw: dict[str, Any]) -> "OrderBookSnapshot":
        asks = [
            OrderBookLevel(price=Decimal(str(lvl["price"])), size=Decimal(str(lvl["size"])))
            for lvl in raw.get("asks", [])
            if Decimal(str(lvl["price"])) > Decimal("0") and Decimal(str(lvl["size"])) > Decimal("0")
        ]
        bids = [
            OrderBookLevel(price=Decimal(str(lvl["price"])), size=Decimal(str(lvl["size"])))
            for lvl in raw.get("bids", [])
            if Decimal(str(lvl["price"])) > Decimal("0") and Decimal(str(lvl["size"])) > Decimal("0")
        ]
        # Critical invariant: Polymarket /book returns asks descending; always sort ascending!
        asks.sort(key=lambda x: x.price)
        bids.sort(key=lambda x: x.price, reverse=True)
        return cls(
            token_id=token_id,
            bids=bids,
            asks=asks,
            tick_size=Decimal(str(raw.get("tick_size", "0.01"))),
            min_order_size=Decimal(str(raw.get("min_order_size", "1.0"))),
            fee_rate=Decimal(str(raw.get("fee_rate", "0.02"))),
            fee_exponent=Decimal(str(raw.get("fee_exponent", "1.0"))),
            neg_risk=bool(raw.get("neg_risk", False)),
        )

    @property
    def best_ask(self) -> Optional[Decimal]:
        return self.asks[0].price if self.asks else None

    @property
    def best_bid(self) -> Optional[Decimal]:
        return self.bids[0].price if self.bids else None


def compute_taker_fee_usd(
    price: Decimal,
    shares: Decimal,
    fee_rate: Decimal = Decimal("0.02"),
    fee_exponent: Decimal = Decimal("1.0"),
) -> Decimal:
    """Polymarket taker fee formula: `rate * (p * (1 - p))^exponent * shares`."""
    if fee_rate <= Decimal("0") or shares <= Decimal("0") or price <= Decimal("0") or price >= Decimal("1"):
        return Decimal("0")
    base = price * (Decimal("1") - price)
    if fee_exponent == Decimal("1") or fee_exponent == Decimal("1.0"):
        shaped = base
    else:
        shaped = Decimal(str(float(base) ** float(fee_exponent)))
    return (fee_rate * shaped * shares).quantize(Decimal("0.000001"), rounding=ROUND_DOWN)


def snap_price_to_tick(price: Decimal, tick_size: Decimal, *, side: str) -> Decimal:
    if tick_size <= Decimal("0"):
        return price.quantize(Decimal("0.000001"))
    if side.upper() == "BUY":
        steps = (price / tick_size).to_integral_value(rounding=ROUND_FLOOR)
        snapped = steps * tick_size
        return max(tick_size, snapped).quantize(Decimal("0.000001"))
    steps = (price / tick_size).to_integral_value(rounding=ROUND_CEILING)
    snapped = steps * tick_size
    return min(Decimal("0.999"), max(tick_size, snapped)).quantize(Decimal("0.000001"))


def cent_grid_share_step(limit_price: Decimal) -> Decimal:
    """Compute the exact share step so `limit_price * shares` has <= 2 decimal places ($0.01 cents)
    and `shares` has <= 5 decimal places (Fix P-03).
    """
    if limit_price <= Decimal("0"):
        return Decimal("1")
    norm_p = limit_price.normalize()
    sign, digits, exponent = norm_p.as_tuple()
    if sign != 0 or not digits:
        return Decimal("1")
    scale = max(0, -int(exponent))
    mantissa = int("".join(str(d) for d in digits))
    if exponent > 0:
        mantissa *= 10 ** int(exponent)
    if mantissa == 0:
        return Decimal("1")

    modulus = 10 ** (scale + SIZE_DECIMALS_MAX - AMOUNT_DECIMALS_MAX)
    step_units = modulus // math.gcd(mantissa, modulus)
    step = Decimal(step_units) / (Decimal(10) ** SIZE_DECIMALS_MAX)
    return step.normalize()


def snap_shares_to_cent_grid(shares: Decimal, limit_price: Decimal) -> Decimal:
    """Snap order size down to the price-dependent cent grid step so that `shares * limit_price`
    has at most 2 decimal places (cents) and `shares` has at most 5 decimal places (Fix P-03).
    """
    if shares <= Decimal("0") or limit_price <= Decimal("0"):
        return Decimal("0")
    step = cent_grid_share_step(limit_price)
    steps = (shares / step).to_integral_value(rounding=ROUND_FLOOR)
    snapped = (steps * step).quantize(Decimal("0.00001"), rounding=ROUND_DOWN)
    return snapped.normalize() if snapped > Decimal("0") else Decimal("0")


def simulate_book_walk_buy(
    book: OrderBookSnapshot,
    requested_shares: Decimal,
    limit_price: Decimal,
) -> tuple[Decimal, Decimal, Decimal, Decimal]:
    """Walk asks up to `limit_price` to buy `requested_shares`.

    Returns `(filled_shares, avg_price, notional_usd, fee_usd)`.
    """
    filled_shares = Decimal("0")
    spent_usd = Decimal("0")
    for lvl in book.asks:
        if lvl.price > limit_price:
            break
        remaining = requested_shares - filled_shares
        if remaining <= Decimal("0"):
            break
        take = min(remaining, lvl.size)
        if take <= Decimal("0"):
            break
        filled_shares += take
        spent_usd += take * lvl.price

    if filled_shares <= Decimal("0"):
        return Decimal("0"), Decimal("0"), Decimal("0"), Decimal("0")
    avg_price = (spent_usd / filled_shares).quantize(Decimal("0.000001"))
    notional = spent_usd.quantize(Decimal("0.000001"))
    fee = compute_taker_fee_usd(avg_price, filled_shares, book.fee_rate, book.fee_exponent)
    return filled_shares.quantize(Decimal("0.000001")), avg_price, notional, fee


def simulate_book_walk_sell(
    book: OrderBookSnapshot,
    requested_shares: Decimal,
    floor_price: Decimal,
) -> tuple[Decimal, Decimal, Decimal, Decimal]:
    """Walk bids down to `floor_price` to sell `requested_shares`.

    Returns `(filled_shares, avg_price, notional_usd, fee_usd)`.
    """
    filled_shares = Decimal("0")
    proceeds_usd = Decimal("0")
    for lvl in book.bids:
        if lvl.price < floor_price:
            break
        remaining = requested_shares - filled_shares
        if remaining <= Decimal("0"):
            break
        take = min(remaining, lvl.size)
        if take <= Decimal("0"):
            break
        filled_shares += take
        proceeds_usd += take * lvl.price

    if filled_shares <= Decimal("0"):
        return Decimal("0"), Decimal("0"), Decimal("0"), Decimal("0")
    avg_price = (proceeds_usd / filled_shares).quantize(Decimal("0.000001"))
    notional = proceeds_usd.quantize(Decimal("0.000001"))
    fee = compute_taker_fee_usd(avg_price, filled_shares, book.fee_rate, book.fee_exponent)
    return filled_shares.quantize(Decimal("0.000001")), avg_price, notional, fee


class CopyTradingEngine:
    """Executes queued copy-trading jobs in PAPER_TRADING mode and delegates LIVE_TRADING orders."""

    def __init__(self, metrics: Optional[PlatformMetricsRegistry] = None) -> None:
        self._metrics = metrics or GLOBAL_METRICS

    def compute_buy_target_notional(
        self,
        *,
        mapping: AccountSourceTrader,
        strategy: Optional[CopyStrategy],
        risk: RiskLimit,
        source_event: SourceTradeEvent,
        source_trader: SourceTrader,
        account_balance_usd: Decimal,
    ) -> Decimal:
        """Compute target USD notional using the configured execution mode."""
        mode = mapping.execution_mode
        multiplier = Decimal(str(mapping.copy_multiplier))
        max_order = min(
            Decimal(str(mapping.max_order_size_usd)),
            Decimal(str(risk.max_order_size_usd)),
        )
        if strategy and strategy.is_active:
            max_order = min(max_order, Decimal(str(strategy.max_order_size_usd)))

        source_notional = Decimal(str(source_event.notional_usd))

        if mode == ExecutionMode.FIXED_AMOUNT.value:
            raw_usd = Decimal(str(mapping.fixed_amount_usd)) * multiplier
        elif mode == ExecutionMode.FIXED_PERCENTAGE.value:
            pct = Decimal(str(mapping.copy_percentage))
            raw_usd = source_notional * pct * multiplier
        elif mode == ExecutionMode.PROPORTIONAL_BALANCE.value:
            source_bal = max(Decimal(str(source_trader.estimated_balance_usd)), Decimal("1.0"))
            dest_bal = max(account_balance_usd, Decimal("0.0"))
            ratio = dest_bal / source_bal
            raw_usd = source_notional * ratio * multiplier
        elif mode == ExecutionMode.MAX_ORDER_SIZE.value:
            raw_usd = max_order
        else:
            raw_usd = Decimal(str(mapping.fixed_amount_usd)) * multiplier

        return min(raw_usd, max_order).quantize(Decimal("0.000001"), rounding=ROUND_DOWN)

    def execute_job(
        self,
        session: Session,
        job: ExecutionJob,
        book: OrderBookSnapshot,
        *,
        now: Optional[datetime] = None,
        live_executor: Optional[Any] = None,
    ) -> ExecutionJob:
        """Evaluate all risk gates, compute sizing/attribution, and execute a copy-trading job."""
        current_time = now or datetime.now(timezone.utc)
        job.started_at = current_time
        job.attempts += 1

        account = session.get(TradingAccount, job.trading_account_id)
        source_event = session.get(SourceTradeEvent, job.source_trade_event_id)
        source_trader = session.get(SourceTrader, job.source_trader_id)
        if account is None or source_event is None or source_trader is None:
            return self._skip_job(job, "missing_account_or_event", current_time)

        if account.operating_mode == OperatingMode.MONITORING.value:
            job.status = ExecutionJobStatus.COMPLETED.value
            job.skip_or_error_reason = "monitoring_mode_no_order"
            job.completed_at = current_time
            return job

        # 1. Global & Per-Account Emergency Stop / Circuit Breaker checks
        global_stop = session.scalar(
            select(SystemSetting).where(SystemSetting.key == "global_emergency_stop")
        )
        if global_stop and isinstance(global_stop.value_json, dict) and global_stop.value_json.get("enabled"):
            return self._skip_job(job, "global_emergency_stop", current_time)

        if account.emergency_stop or account.trading_status != "ACTIVE":
            return self._skip_job(job, "account_emergency_stop", current_time)

        if account.circuit_breaker_tripped:
            return self._skip_job(job, "circuit_breaker_tripped", current_time)

        mapping = session.scalar(
            select(AccountSourceTrader).where(
                AccountSourceTrader.trading_account_id == account.id,
                AccountSourceTrader.source_trader_id == source_trader.id,
            )
        )
        if mapping is None or not mapping.is_enabled:
            return self._skip_job(job, "source_trader_disabled", current_time)

        risk = account.risk_limits
        if risk is None:
            return self._skip_job(job, "missing_risk_limits", current_time)

        strategy = account.copy_strategies[0] if account.copy_strategies else None

        # 2. Stale-data protection
        ev_ts = source_event.source_event_ts
        if ev_ts.tzinfo is None:
            ev_ts = ev_ts.replace(tzinfo=timezone.utc)
        age_secs = (current_time - ev_ts).total_seconds()
        if age_secs > risk.max_stale_data_seconds:
            return self._skip_job(job, "stale_source_event", current_time)

        # 3. Allowed markets filter
        if strategy and strategy.allowed_markets:
            if (
                source_event.market_condition_id not in strategy.allowed_markets
                and source_event.token_id not in strategy.allowed_markets
            ):
                return self._skip_job(job, "market_not_allowed", current_time)

        # 4. Daily loss protection check
        balance = self._get_or_create_balance(session, account)
        today_str = current_time.strftime("%Y-%m-%d")
        if risk.daily_loss_stop_latched and risk.daily_loss_latched_date == today_str:
            return self._skip_job(job, "daily_loss_limit_latched", current_time)

        if risk.max_daily_loss_usd > Decimal("0") and balance.daily_realized_pnl_usd <= -risk.max_daily_loss_usd:
            risk.daily_loss_stop_latched = True
            risk.daily_loss_latched_date = today_str
            return self._skip_job(job, "daily_loss_limit_exceeded", current_time)

        # 5. Execution rate limit per time interval
        if risk.max_executions_per_interval > 0:
            window_start = current_time - timedelta(seconds=risk.execution_interval_seconds)
            recent_orders_count = session.scalar(
                select(func.count(Order.id)).where(
                    Order.trading_account_id == account.id,
                    Order.order_submitted_ts >= window_start,
                )
            ) or 0
            if recent_orders_count >= risk.max_executions_per_interval:
                return self._skip_job(job, "rate_limited", current_time)

        side = source_event.side.upper()
        if side == TradeSide.BUY.value:
            return self._handle_buy(
                session,
                job=job,
                account=account,
                mapping=mapping,
                strategy=strategy,
                risk=risk,
                balance=balance,
                source_event=source_event,
                source_trader=source_trader,
                book=book,
                current_time=current_time,
                live_executor=live_executor,
            )
        if side == TradeSide.SELL.value:
            return self._handle_sell(
                session,
                job=job,
                account=account,
                mapping=mapping,
                strategy=strategy,
                risk=risk,
                balance=balance,
                source_event=source_event,
                source_trader=source_trader,
                book=book,
                current_time=current_time,
                live_executor=live_executor,
            )
        return self._skip_job(job, "unsupported_side", current_time)

    def _handle_buy(
        self,
        session: Session,
        *,
        job: ExecutionJob,
        account: TradingAccount,
        mapping: AccountSourceTrader,
        strategy: Optional[CopyStrategy],
        risk: RiskLimit,
        balance: AccountBalance,
        source_event: SourceTradeEvent,
        source_trader: SourceTrader,
        book: OrderBookSnapshot,
        current_time: datetime,
        live_executor: Optional[Any],
    ) -> ExecutionJob:
        if not mapping.copy_buy_enabled or (strategy and not strategy.buy_enabled):
            return self._skip_job(job, "buy_copying_disabled", current_time)

        # Fix E-01: Do NOT increment `vpos.source_observed_shares` before risk, slippage,
        # balance, and execution checks succeed! Only increment `source_observed_shares`
        # inside `apply_confirmed_fill` when a BUY order actually fills (proportionally for partial fills).

        # Slippage & book liquidity check
        max_slippage = min(Decimal(str(mapping.max_slippage_pct)), Decimal(str(risk.max_slippage_pct)))
        if strategy and strategy.is_active:
            max_slippage = min(max_slippage, Decimal(str(strategy.max_slippage_pct)))

        raw_limit = min(Decimal(str(source_event.price)) * (Decimal("1") + max_slippage), Decimal("0.999"))
        limit_price = snap_price_to_tick(raw_limit, book.tick_size, side="BUY")

        best_ask = book.best_ask
        if best_ask is None:
            return self._skip_job(job, "unavailable_liquidity_empty_book", current_time)
        if best_ask > limit_price:
            return self._skip_job(job, "slippage_exceeded", current_time)

        avail_liquidity_usd = sum(
            (lvl.price * lvl.size for lvl in book.asks if lvl.price <= limit_price),
            Decimal("0"),
        )
        if avail_liquidity_usd < Decimal(str(risk.min_market_liquidity_usd)):
            return self._skip_job(job, "insufficient_market_liquidity", current_time)

        target_usd = self.compute_buy_target_notional(
            mapping=mapping,
            strategy=strategy,
            risk=risk,
            source_event=source_event,
            source_trader=source_trader,
            account_balance_usd=Decimal(str(balance.available_cash_usd)),
        )
        min_order_usd = max(
            EXCHANGE_MIN_NOTIONAL_USD,
            Decimal(str(mapping.min_order_size_usd)),
            Decimal(str(risk.min_order_size_usd)),
        )
        if target_usd < min_order_usd:
            return self._skip_job(job, "below_minimum_order_size", current_time)

        # Exposure checks: per-market, per-source-trader, and total account exposure
        market_exp = self._get_market_exposure_usd(session, account.id, account.operating_mode, source_event.market_condition_id)
        if risk.max_market_exposure_usd > Decimal("0") and (market_exp + target_usd) > risk.max_market_exposure_usd:
            return self._skip_job(job, "max_market_exposure_exceeded", current_time)

        trader_exp = self._get_source_trader_exposure_usd(
            session, account.id, account.operating_mode, source_trader.wallet_address
        )
        effective_trader_cap = min(
            Decimal(str(mapping.max_exposure_usd)),
            Decimal(str(risk.max_source_trader_exposure_usd)),
        )
        if effective_trader_cap > Decimal("0") and (trader_exp + target_usd) > effective_trader_cap:
            return self._skip_job(job, "max_source_trader_exposure_exceeded", current_time)

        total_exp = self._get_total_exposure_usd(session, account.id, account.operating_mode)
        if risk.max_total_exposure_usd > Decimal("0") and (total_exp + target_usd) > risk.max_total_exposure_usd:
            return self._skip_job(job, "max_total_exposure_exceeded", current_time)

        # Balance check
        if Decimal(str(balance.available_cash_usd)) < target_usd:
            return self._skip_job(job, "insufficient_balance", current_time)

        # Fix P-03: Snap shares to the price-dependent cent grid step for `limit_price`
        # so that `requested_shares * limit_price` has <= 2 decimal places (exact cents).
        requested_shares = snap_shares_to_cent_grid(target_usd / best_ask, limit_price)
        if requested_shares <= Decimal("0") or (requested_shares * limit_price) < EXCHANGE_MIN_NOTIONAL_USD:
            return self._skip_job(job, "below_minimum_order_size", current_time)

        vpos = self._get_or_create_virtual_position(
            session,
            account=account,
            source_trader=source_trader,
            source_event=source_event,
        )

        job.target_notional_usd = (requested_shares * limit_price).quantize(Decimal("0.000001"))
        job.target_shares = requested_shares
        job.limit_price = limit_price

        return self._dispatch_and_book_order(
            session,
            job=job,
            account=account,
            source_event=source_event,
            source_trader=source_trader,
            vpos=vpos,
            balance=balance,
            risk=risk,
            book=book,
            side="BUY",
            requested_shares=requested_shares,
            limit_price=limit_price,
            current_time=current_time,
            live_executor=live_executor,
        )

    def _handle_sell(
        self,
        session: Session,
        *,
        job: ExecutionJob,
        account: TradingAccount,
        mapping: AccountSourceTrader,
        strategy: Optional[CopyStrategy],
        risk: RiskLimit,
        balance: AccountBalance,
        source_event: SourceTradeEvent,
        source_trader: SourceTrader,
        book: OrderBookSnapshot,
        current_time: datetime,
        live_executor: Optional[Any],
    ) -> ExecutionJob:
        if not mapping.copy_sell_enabled or (strategy and not strategy.sell_enabled):
            return self._skip_job(job, "sell_copying_disabled", current_time)

        # Critical Virtual Position Attribution Lookup:
        # ONLY look up the lot belonging to `(account.id, source_trader.wallet_address, source_event.token_id, account.operating_mode)`!
        vpos = session.scalar(
            select(VirtualPosition).where(
                VirtualPosition.trading_account_id == account.id,
                VirtualPosition.source_wallet == source_trader.wallet_address.lower(),
                VirtualPosition.token_id == source_event.token_id,
                VirtualPosition.operating_mode == account.operating_mode,
            )
        )
        if vpos is None or Decimal(str(vpos.open_shares)) <= Decimal("0"):
            return self._skip_job(job, "no_attributable_position_to_sell", current_time)

        open_shares = Decimal(str(vpos.open_shares))
        observed = Decimal(str(vpos.source_observed_shares))
        source_sold = Decimal(str(source_event.size))

        if observed <= Decimal("0"):
            exit_fraction = Decimal("1.0")
        else:
            exit_fraction = min(Decimal("1.0"), max(Decimal("0.0"), source_sold / observed))

        max_slippage = min(Decimal(str(mapping.max_slippage_pct)), Decimal(str(risk.max_slippage_pct)))
        if strategy and strategy.is_active:
            max_slippage = min(max_slippage, Decimal(str(strategy.max_slippage_pct)))

        raw_floor = max(Decimal(str(source_event.price)) * (Decimal("1") - max_slippage), Decimal("0.001"))
        floor_price = snap_price_to_tick(raw_floor, book.tick_size, side="SELL")

        # Combine with any previously deferred exit shares (never exceeding open_shares!)
        pending = Decimal(str(vpos.pending_exit_shares))
        desired_shares = min(open_shares, (open_shares * exit_fraction) + pending)
        requested_shares = snap_shares_to_cent_grid(desired_shares, floor_price)
        # Strictly never sell more than attributable open_shares
        requested_shares = min(open_shares, requested_shares)

        if requested_shares <= Decimal("0"):
            vpos.pending_exit_shares = desired_shares
            return self._skip_job(job, "exit_fraction_accumulated_below_lot", current_time)

        # Check exchange $1 minimum notional on exit: never top up a sell above open_shares!
        if (requested_shares * floor_price) < EXCHANGE_MIN_NOTIONAL_USD:
            vpos.pending_exit_shares = desired_shares
            return self._skip_job(job, "exit_below_minimum_notional_accumulated", current_time)

        best_bid = book.best_bid
        if best_bid is None:
            vpos.pending_exit_shares = desired_shares
            return self._skip_job(job, "unavailable_liquidity_empty_book", current_time)
        if best_bid < floor_price:
            vpos.pending_exit_shares = desired_shares
            return self._skip_job(job, "slippage_exceeded", current_time)

        job.target_shares = requested_shares
        job.target_notional_usd = (requested_shares * floor_price).quantize(Decimal("0.000001"))
        job.limit_price = floor_price

        return self._dispatch_and_book_order(
            session,
            job=job,
            account=account,
            source_event=source_event,
            source_trader=source_trader,
            vpos=vpos,
            balance=balance,
            risk=risk,
            book=book,
            side="SELL",
            requested_shares=requested_shares,
            limit_price=floor_price,
            current_time=current_time,
            live_executor=live_executor,
        )

    def _dispatch_and_book_order(
        self,
        session: Session,
        *,
        job: ExecutionJob,
        account: TradingAccount,
        source_event: SourceTradeEvent,
        source_trader: SourceTrader,
        vpos: VirtualPosition,
        balance: AccountBalance,
        risk: RiskLimit,
        book: OrderBookSnapshot,
        side: str,
        requested_shares: Decimal,
        limit_price: Decimal,
        current_time: datetime,
        live_executor: Optional[Any],
    ) -> ExecutionJob:
        submitted_ts = datetime.now(timezone.utc)
        order_idempotency = f"order:job:{job.id}"
        client_order_id = f"pom_ord_job_{job.id}_acct_{account.id}"

        # Check if a durable order intent already exists for this job
        order = session.scalar(select(Order).where(Order.execution_job_id == job.id))
        if order is None:
            order = Order(
                user_id=account.user_id,
                trading_account_id=account.id,
                source_trader_id=source_trader.id,
                execution_job_id=job.id,
                client_order_id=client_order_id,
                idempotency_key=order_idempotency,
                operating_mode=account.operating_mode,
                market_condition_id=source_event.market_condition_id,
                token_id=source_event.token_id,
                side=side,
                order_type="FAK",
                limit_price=limit_price,
                requested_size_shares=requested_shares,
                requested_notional_usd=(requested_shares * limit_price).quantize(Decimal("0.000001")),
                filled_size_shares=Decimal("0"),
                status=OrderStatus.PREPARED.value,
                order_submitted_ts=submitted_ts,
            )
            session.add(order)
            session.flush()

        self._metrics.inc("orders_submitted_total")

        if account.operating_mode == OperatingMode.PAPER_TRADING.value:
            order.status = OrderStatus.SUBMITTING.value
            if side == "BUY":
                filled_shares, avg_price, notional, fee = simulate_book_walk_buy(
                    book, requested_shares, limit_price
                )
            else:
                filled_shares, avg_price, notional, fee = simulate_book_walk_sell(
                    book, requested_shares, limit_price
                )
            ack_ts = datetime.now(timezone.utc)
            order.order_ack_ts = ack_ts
            order.exchange_order_id = f"paper_ord_{order.id}"

            if filled_shares <= Decimal("0"):
                order.status = OrderStatus.REJECTED.value
                order.error_message = "No liquidity inside limit price"
                job.status = ExecutionJobStatus.FAILED.value
                job.skip_or_error_reason = order.error_message
                job.completed_at = ack_ts
                self._metrics.inc("orders_rejected_total")
                return job

            self.apply_confirmed_fill(
                session,
                order=order,
                job=job,
                account=account,
                vpos=vpos,
                balance=balance,
                risk=risk,
                filled_shares=filled_shares,
                avg_price=avg_price,
                notional_usd=notional,
                fee_usd=fee,
                fill_ts=ack_ts,
                source_event=source_event,
            )
            return job

        # LIVE_TRADING mode delegates to the isolated LiveAccountExecutionWorker
        if live_executor is None:
            order.status = OrderStatus.REJECTED.value
            order.error_message = "Live executor not attached"
            job.status = ExecutionJobStatus.FAILED.value
            job.skip_or_error_reason = order.error_message
            job.completed_at = current_time
            return job

        return live_executor.execute_live_order(
            session,
            order=order,
            job=job,
            account=account,
            vpos=vpos,
            balance=balance,
            risk=risk,
            book=book,
            source_event=source_event,
        )

    def apply_confirmed_fill(
        self,
        session: Session,
        *,
        order: Order,
        job: Optional[ExecutionJob],
        account: TradingAccount,
        vpos: VirtualPosition,
        balance: AccountBalance,
        risk: RiskLimit,
        filled_shares: Decimal,
        avg_price: Decimal,
        notional_usd: Decimal,
        fee_usd: Decimal,
        fill_ts: datetime,
        source_event: Optional[SourceTradeEvent] = None,
        exchange_trade_id: Optional[str] = None,
    ) -> Fill:
        """Atomically book a confirmed BUY or SELL fill into `fills`, `virtual_positions`, and `account_balances`."""
        # Idempotency guard: if a Fill already exists for this order_id, return it without double-booking
        existing_fill = session.scalar(select(Fill).where(Fill.order_id == order.id))
        if existing_fill is not None:
            return existing_fill

        order.filled_size_shares = filled_shares
        requested_sh = Decimal(str(order.requested_size_shares))
        if filled_shares < requested_sh:
            order.status = OrderStatus.PARTIAL.value
            self._metrics.inc("orders_partial_total")
        else:
            order.status = OrderStatus.FILLED.value
            self._metrics.inc("orders_filled_total")

        realized_pnl = Decimal("0")
        open_shares_before = Decimal(str(vpos.open_shares))
        cost_basis_before = Decimal(str(vpos.cost_basis_usd))
        observed_before = Decimal(str(vpos.source_observed_shares))

        if order.side == "BUY":
            # Fix E-01: Only increment `source_observed_shares` when our BUY actually fills!
            # Scale proportionally if our BUY was only partially filled.
            if source_event is not None and requested_sh > Decimal("0"):
                fill_ratio = min(Decimal("1.0"), filled_shares / requested_sh)
                attributed_source_shares = (Decimal(str(source_event.size)) * fill_ratio).quantize(
                    Decimal("0.000001")
                )
                vpos.source_observed_shares = observed_before + attributed_source_shares

            new_open = open_shares_before + filled_shares
            new_cost = cost_basis_before + notional_usd
            vpos.open_shares = new_open
            vpos.total_bought_shares = Decimal(str(vpos.total_bought_shares)) + filled_shares
            vpos.cost_basis_usd = new_cost
            vpos.avg_entry_price = (new_cost / new_open).quantize(Decimal("0.000001")) if new_open > Decimal("0") else Decimal("0")
            vpos.total_fees_usd = Decimal(str(vpos.total_fees_usd)) + fee_usd
            vpos.closed_at = None

            balance.available_cash_usd = Decimal(str(balance.available_cash_usd)) - (notional_usd + fee_usd)
            balance.positions_market_value_usd = Decimal(str(balance.positions_market_value_usd)) + notional_usd
            balance.total_equity_usd = Decimal(str(balance.available_cash_usd)) + Decimal(str(balance.positions_market_value_usd))
        else:
            sell_shares = min(open_shares_before, filled_shares)
            proportion = (sell_shares / open_shares_before) if open_shares_before > Decimal("0") else Decimal("1")
            released_cost = (cost_basis_before * proportion).quantize(Decimal("0.000001"))
            realized_pnl = (notional_usd - released_cost - fee_usd).quantize(Decimal("0.000001"))

            remaining_shares = max(Decimal("0"), open_shares_before - sell_shares)
            vpos.open_shares = remaining_shares
            vpos.total_sold_shares = Decimal(str(vpos.total_sold_shares)) + sell_shares
            vpos.cost_basis_usd = max(Decimal("0"), cost_basis_before - released_cost)
            # Reduce source_observed_shares proportionally to the fraction of open_shares actually sold
            vpos.source_observed_shares = max(
                Decimal("0"), (observed_before * (Decimal("1") - proportion)).quantize(Decimal("0.000001"))
            )
            if remaining_shares == Decimal("0"):
                vpos.avg_entry_price = Decimal("0")
                vpos.closed_at = fill_ts
                vpos.pending_exit_shares = Decimal("0")
                vpos.source_observed_shares = Decimal("0")
            else:
                vpos.pending_exit_shares = max(
                    Decimal("0"), Decimal(str(vpos.pending_exit_shares)) - sell_shares
                )
            vpos.realized_pnl_usd = Decimal(str(vpos.realized_pnl_usd)) + realized_pnl
            vpos.total_fees_usd = Decimal(str(vpos.total_fees_usd)) + fee_usd

            balance.available_cash_usd = Decimal(str(balance.available_cash_usd)) + (notional_usd - fee_usd)
            balance.positions_market_value_usd = max(
                Decimal("0"), Decimal(str(balance.positions_market_value_usd)) - released_cost
            )
            balance.total_equity_usd = Decimal(str(balance.available_cash_usd)) + Decimal(str(balance.positions_market_value_usd))
            balance.daily_realized_pnl_usd = Decimal(str(balance.daily_realized_pnl_usd)) + realized_pnl
            balance.weekly_realized_pnl_usd = Decimal(str(balance.weekly_realized_pnl_usd)) + realized_pnl
            balance.monthly_realized_pnl_usd = Decimal(str(balance.monthly_realized_pnl_usd)) + realized_pnl
            balance.all_time_realized_pnl_usd = Decimal(str(balance.all_time_realized_pnl_usd)) + realized_pnl

            if risk.max_daily_loss_usd > Decimal("0") and balance.daily_realized_pnl_usd <= -risk.max_daily_loss_usd:
                risk.daily_loss_stop_latched = True
                risk.daily_loss_latched_date = fill_ts.strftime("%Y-%m-%d")

        fill = Fill(
            user_id=account.user_id,
            trading_account_id=account.id,
            order_id=order.id,
            source_trader_id=order.source_trader_id,
            exchange_trade_id=exchange_trade_id or f"fill_{order.id}",
            operating_mode=account.operating_mode,
            token_id=order.token_id,
            side=order.side,
            filled_shares=filled_shares,
            avg_fill_price=avg_price,
            notional_usd=notional_usd,
            fee_usd=fee_usd,
            realized_pnl_usd=realized_pnl,
            fill_ts=fill_ts,
        )
        session.add(fill)

        if job is not None:
            job.status = ExecutionJobStatus.COMPLETED.value
            job.completed_at = fill_ts

        account.consecutive_errors = 0

        if source_event is not None and source_event.event_routed_ts is not None:
            self._metrics.record_latency(
                source_event_ts=source_event.source_event_ts,
                event_received_ts=source_event.event_received_ts,
                event_normalized_ts=source_event.event_normalized_ts,
                event_routed_ts=source_event.event_routed_ts,
                order_submitted_ts=order.order_submitted_ts,
                order_ack_ts=order.order_ack_ts or fill_ts,
                fill_ts=fill_ts,
            )

        session.flush()
        return fill

    def _skip_job(self, job: ExecutionJob, reason: str, current_time: datetime) -> ExecutionJob:
        job.status = ExecutionJobStatus.SKIPPED.value
        job.skip_or_error_reason = reason
        job.completed_at = current_time
        self._metrics.inc("jobs_skipped_total")
        return job

    def _get_or_create_balance(self, session: Session, account: TradingAccount) -> AccountBalance:
        bal = session.scalar(
            select(AccountBalance).where(
                AccountBalance.trading_account_id == account.id,
                AccountBalance.operating_mode == account.operating_mode,
            )
        )
        if bal is None:
            bal = AccountBalance(
                user_id=account.user_id,
                trading_account_id=account.id,
                operating_mode=account.operating_mode,
                available_cash_usd=Decimal("1000.0"),
                total_equity_usd=Decimal("1000.0"),
            )
            session.add(bal)
            session.flush()
        return bal

    def _get_or_create_virtual_position(
        self,
        session: Session,
        *,
        account: TradingAccount,
        source_trader: SourceTrader,
        source_event: SourceTradeEvent,
    ) -> VirtualPosition:
        vpos = session.scalar(
            select(VirtualPosition).where(
                VirtualPosition.trading_account_id == account.id,
                VirtualPosition.source_wallet == source_trader.wallet_address.lower(),
                VirtualPosition.token_id == source_event.token_id,
                VirtualPosition.operating_mode == account.operating_mode,
            )
        )
        if vpos is None:
            vpos = VirtualPosition(
                user_id=account.user_id,
                trading_account_id=account.id,
                source_trader_id=source_trader.id,
                source_wallet=source_trader.wallet_address.lower(),
                market_condition_id=source_event.market_condition_id,
                token_id=source_event.token_id,
                operating_mode=account.operating_mode,
                open_shares=Decimal("0"),
                cost_basis_usd=Decimal("0"),
            )
            session.add(vpos)
            session.flush()
        return vpos

    def _get_market_exposure_usd(
        self,
        session: Session,
        account_id: int,
        operating_mode: str,
        market_condition_id: str,
    ) -> Decimal:
        total = session.scalar(
            select(func.coalesce(func.sum(VirtualPosition.cost_basis_usd), 0)).where(
                VirtualPosition.trading_account_id == account_id,
                VirtualPosition.operating_mode == operating_mode,
                VirtualPosition.market_condition_id == market_condition_id,
                VirtualPosition.open_shares > 0,
            )
        )
        return Decimal(str(total or 0))

    def _get_source_trader_exposure_usd(
        self,
        session: Session,
        account_id: int,
        operating_mode: str,
        source_wallet: str,
    ) -> Decimal:
        total = session.scalar(
            select(func.coalesce(func.sum(VirtualPosition.cost_basis_usd), 0)).where(
                VirtualPosition.trading_account_id == account_id,
                VirtualPosition.operating_mode == operating_mode,
                VirtualPosition.source_wallet == source_wallet.lower(),
                VirtualPosition.open_shares > 0,
            )
        )
        return Decimal(str(total or 0))

    def _get_total_exposure_usd(
        self,
        session: Session,
        account_id: int,
        operating_mode: str,
    ) -> Decimal:
        total = session.scalar(
            select(func.coalesce(func.sum(VirtualPosition.cost_basis_usd), 0)).where(
                VirtualPosition.trading_account_id == account_id,
                VirtualPosition.operating_mode == operating_mode,
                VirtualPosition.open_shares > 0,
            )
        )
        return Decimal(str(total or 0))
