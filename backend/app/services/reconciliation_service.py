"""Reconciliation Service: resolves uncertain (`SUBMITTING`, `ACKNOWLEDGED`, `AMBIGUOUS`, `SUBMITTED`) order outcomes, recovers unfinished jobs after worker restarts, and checks position divergences."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
from decimal import Decimal
from typing import Any, Optional

from sqlalchemy import select
from sqlalchemy.orm import Session

from backend.app.models import (
    AccountBalance,
    ExecutionJob,
    ExecutionJobStatus,
    Order,
    OrderStatus,
    RiskLimit,
    SourceTradeEvent,
    SourceTrader,
    TradingAccount,
    VirtualPosition,
)
from backend.app.services.account_service import TradingAccountService
from backend.app.services.copy_engine import (
    CopyTradingEngine,
    OrderBookSnapshot,
    compute_taker_fee_usd,
)
from backend.app.services.live_execution_worker import PolymarketExchangeGateway
from backend.app.services.metrics_service import GLOBAL_METRICS, PlatformMetricsRegistry
from backend.app.services.polymarket_signer import build_l2_auth_headers

UNCERTAIN_ORDER_STATUSES = (
    OrderStatus.SUBMITTING.value,
    OrderStatus.ACKNOWLEDGED.value,
    OrderStatus.AMBIGUOUS.value,
    OrderStatus.SUBMITTED.value,
)


@dataclass
class PositionDivergence:
    trading_account_id: int
    token_id: str
    local_open_shares: Decimal
    exchange_shares: Decimal
    delta_shares: Decimal
    verdict: str  # "AGREED" | "EXPLAINED_IN_FLIGHT" | "UNATTRIBUTED_DIVERGENCE"


@dataclass
class ReconciliationSummary:
    ambiguous_orders_checked: int = 0
    orders_resolved_filled: int = 0
    orders_resolved_rejected: int = 0
    unfinished_jobs_recovered: int = 0
    positions_checked: int = 0
    divergences: list[PositionDivergence] = field(default_factory=list)


class ReconciliationService:
    """Periodically reconciles uncertain orders, unfinished jobs after restart, and on-chain/exchange positions."""

    def __init__(
        self,
        exchange_gateway: PolymarketExchangeGateway,
        *,
        account_service: Optional[TradingAccountService] = None,
        copy_engine: Optional[CopyTradingEngine] = None,
        metrics: Optional[PlatformMetricsRegistry] = None,
    ) -> None:
        self._gateway = exchange_gateway
        self._account_service = account_service or TradingAccountService()
        self._copy_engine = copy_engine or CopyTradingEngine()
        self._metrics = metrics or GLOBAL_METRICS

    def reconcile_ambiguous_orders(self, session: Session) -> ReconciliationSummary:
        """Query Polymarket for all orders in uncertain states (`SUBMITTING`, `ACKNOWLEDGED`, `AMBIGUOUS`, `SUBMITTED`)
        and resolve them idempotently without ever re-submitting an order whose outcome was uncertain.
        """
        summary = ReconciliationSummary()
        self._metrics.inc("reconciliations_run_total")

        uncertain_orders = session.scalars(
            select(Order).where(Order.status.in_(UNCERTAIN_ORDER_STATUSES))
        ).all()

        now = datetime.now(timezone.utc)
        for order in uncertain_orders:
            summary.ambiguous_orders_checked += 1
            account = session.get(TradingAccount, order.trading_account_id)
            if account is None:
                continue

            lookup_id = (
                order.exchange_order_id
                or order.eip712_order_hash
                or order.client_order_id
                or order.idempotency_key
            )
            headers: dict[str, str] = {}
            try:
                bundle = self._account_service.get_decrypted_signing_bundle(
                    session, user_id=account.user_id, account_id=account.id
                )
                maker = account.funder_wallet_address or account.signer_wallet_address
                if bundle.get("api_key") and bundle.get("api_secret") and bundle.get("api_passphrase"):
                    headers = build_l2_auth_headers(
                        poly_address=maker,
                        api_key=bundle["api_key"],
                        api_secret=bundle["api_secret"],
                        api_passphrase=bundle["api_passphrase"],
                        method="GET",
                        path=f"/data/order/{lookup_id}",
                    )
            except Exception:
                pass

            remote = self._gateway.get_order_status(exchange_order_id=lookup_id, headers=headers)
            if remote is None:
                # Keep in AMBIGUOUS state if the exchange cannot yet confirm
                order.status = OrderStatus.AMBIGUOUS.value
                job = session.get(ExecutionJob, order.execution_job_id) if order.execution_job_id else None
                if job is not None:
                    job.status = ExecutionJobStatus.AMBIGUOUS.value
                continue

            order.exchange_order_id = remote.exchange_order_id
            order.reconciled_at = now
            job = session.get(ExecutionJob, order.execution_job_id) if order.execution_job_id else None

            if remote.filled_shares > Decimal("0"):
                source_trader = session.get(SourceTrader, order.source_trader_id) if order.source_trader_id else None
                source_event = session.get(SourceTradeEvent, job.source_trade_event_id) if job else None
                if source_trader is None:
                    continue

                vpos = session.scalar(
                    select(VirtualPosition).where(
                        VirtualPosition.trading_account_id == account.id,
                        VirtualPosition.source_wallet == source_trader.wallet_address.lower(),
                        VirtualPosition.token_id == order.token_id,
                        VirtualPosition.operating_mode == account.operating_mode,
                    )
                )
                if vpos is None:
                    vpos = VirtualPosition(
                        user_id=account.user_id,
                        trading_account_id=account.id,
                        source_trader_id=source_trader.id,
                        source_wallet=source_trader.wallet_address.lower(),
                        market_condition_id=order.market_condition_id,
                        token_id=order.token_id,
                        operating_mode=account.operating_mode,
                    )
                    session.add(vpos)
                    session.flush()

                balance = session.scalar(
                    select(AccountBalance).where(
                        AccountBalance.trading_account_id == account.id,
                        AccountBalance.operating_mode == account.operating_mode,
                    )
                )
                risk = account.risk_limits or session.scalar(
                    select(RiskLimit).where(RiskLimit.trading_account_id == account.id)
                )
                fill_price = remote.avg_fill_price or Decimal(str(order.limit_price))
                notional = (remote.filled_shares * fill_price).quantize(Decimal("0.000001"))
                fee = compute_taker_fee_usd(fill_price, remote.filled_shares)

                self._copy_engine.apply_confirmed_fill(
                    session,
                    order=order,
                    job=job,
                    account=account,
                    vpos=vpos,
                    balance=balance,
                    risk=risk,
                    filled_shares=remote.filled_shares,
                    avg_price=fill_price,
                    notional_usd=notional,
                    fee_usd=fee,
                    fill_ts=now,
                    source_event=source_event,
                    exchange_trade_id=f"reconciled_{remote.exchange_order_id}",
                )
                summary.orders_resolved_filled += 1
                self._metrics.inc("reconciliations_resolved_total")

            elif remote.status in ("CANCELLED", "EXPIRED"):
                order.status = OrderStatus.CANCELLED.value
                order.error_message = f"Reconciled as {remote.status} with zero fill"
                if job is not None:
                    job.status = ExecutionJobStatus.FAILED.value
                    job.skip_or_error_reason = order.error_message
                    job.completed_at = now
                summary.orders_resolved_rejected += 1
                self._metrics.inc("reconciliations_resolved_total")

            elif remote.status in ("REJECTED", "NOT_FOUND"):
                order.status = OrderStatus.REJECTED.value
                order.error_message = f"Reconciled as {remote.status} with zero fill"
                if job is not None:
                    job.status = ExecutionJobStatus.FAILED.value
                    job.skip_or_error_reason = order.error_message
                    job.completed_at = now
                summary.orders_resolved_rejected += 1
                self._metrics.inc("reconciliations_resolved_total")

        session.commit()
        return summary

    def recover_unfinished_jobs_after_restart(
        self,
        session: Session,
        *,
        books_by_token: dict[str, OrderBookSnapshot],
        live_executor: Optional[Any] = None,
    ) -> int:
        """Recover unfinished `QUEUED`, `PROCESSING`, or `AMBIGUOUS` jobs after a worker restart (Fix E-03).

        - First reconciles any orders in `SUBMITTING`, `ACKNOWLEDGED`, `AMBIGUOUS`, or `SUBMITTED` state
          against Polymarket so an order that already left the box is NEVER submitted a second time.
        - If an order died in `PREPARED` state (before transitioning to `SUBMITTING`), it is safe to resume
          execution using the persisted order intent.
        - If a job is still `QUEUED` with no order record, it is executed normally.
        """
        # Step 1: Reconcile all uncertain in-flight orders first!
        self.reconcile_ambiguous_orders(session)

        unfinished = session.scalars(
            select(ExecutionJob)
            .where(ExecutionJob.status.in_([ExecutionJobStatus.QUEUED.value, ExecutionJobStatus.PROCESSING.value]))
            .order_by(ExecutionJob.id.asc())
        ).all()

        recovered = 0
        for job in unfinished:
            existing_order = session.scalar(
                select(Order).where(Order.execution_job_id == job.id)
            )
            if existing_order is not None:
                if existing_order.status in (OrderStatus.FILLED.value, OrderStatus.PARTIAL.value):
                    job.status = ExecutionJobStatus.COMPLETED.value
                    job.completed_at = datetime.now(timezone.utc)
                    recovered += 1
                    continue
                if existing_order.status in (OrderStatus.REJECTED.value, OrderStatus.CANCELLED.value):
                    job.status = ExecutionJobStatus.FAILED.value
                    job.completed_at = datetime.now(timezone.utc)
                    recovered += 1
                    continue
                if existing_order.status in UNCERTAIN_ORDER_STATUSES:
                    # Never re-submit an order whose status is still uncertain after reconciliation!
                    existing_order.status = OrderStatus.AMBIGUOUS.value
                    job.status = ExecutionJobStatus.AMBIGUOUS.value
                    recovered += 1
                    continue
                # Otherwise `existing_order.status == PREPARED` (crashed before `SUBMITTING`):
                # safe to resume execution using the existing `Order` row.

            source_event = session.get(SourceTradeEvent, job.source_trade_event_id)
            if source_event is None:
                continue
            book = books_by_token.get(source_event.token_id)
            if book is None:
                continue

            self._copy_engine.execute_job(
                session,
                job,
                book,
                live_executor=live_executor,
            )
            recovered += 1

        session.commit()
        return recovered

    def reconcile_positions(
        self,
        session: Session,
        *,
        trading_account_id: int,
        exchange_balances_by_token: dict[str, Decimal],
    ) -> list[PositionDivergence]:
        """Compare aggregate local virtual positions for an account against reported exchange token balances."""
        positions = session.scalars(
            select(VirtualPosition).where(
                VirtualPosition.trading_account_id == trading_account_id,
                VirtualPosition.open_shares > 0,
            )
        ).all()

        local_by_token: dict[str, Decimal] = {}
        for p in positions:
            local_by_token[p.token_id] = local_by_token.get(p.token_id, Decimal("0")) + Decimal(str(p.open_shares))

        in_flight_orders = session.scalars(
            select(Order).where(
                Order.trading_account_id == trading_account_id,
                Order.status.in_(UNCERTAIN_ORDER_STATUSES),
            )
        ).all()
        in_flight_by_token: dict[str, Decimal] = {}
        for o in in_flight_orders:
            in_flight_by_token[o.token_id] = in_flight_by_token.get(o.token_id, Decimal("0")) + Decimal(
                str(o.requested_size_shares)
            )

        all_tokens = sorted(set(local_by_token.keys()) | set(exchange_balances_by_token.keys()))
        results: list[PositionDivergence] = []

        for tok in all_tokens:
            local_sh = local_by_token.get(tok, Decimal("0"))
            exch_sh = exchange_balances_by_token.get(tok, Decimal("0"))
            delta = exch_sh - local_sh
            tolerance = max(Decimal("0.01"), local_sh * Decimal("0.001"))

            if abs(delta) <= tolerance:
                verdict = "AGREED"
            elif abs(delta) <= in_flight_by_token.get(tok, Decimal("0")) + tolerance:
                verdict = "EXPLAINED_IN_FLIGHT"
            else:
                verdict = "UNATTRIBUTED_DIVERGENCE"

            results.append(
                PositionDivergence(
                    trading_account_id=trading_account_id,
                    token_id=tok,
                    local_open_shares=local_sh,
                    exchange_shares=exch_sh,
                    delta_shares=delta,
                    verdict=verdict,
                )
            )
        return results
