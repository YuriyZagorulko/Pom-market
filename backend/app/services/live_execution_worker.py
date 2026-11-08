"""Independent Account Execution Workers with isolated queues, rate limiters, signing credentials, and crash-safe durable intent lifecycle (Fix E-03)."""

from __future__ import annotations

from collections import deque
from dataclasses import dataclass, field
from datetime import datetime, timezone
from decimal import Decimal
import hashlib
import json
import threading
import time
from typing import Any, Callable, Optional, Protocol

from sqlalchemy.orm import Session

from backend.app.models import (
    AccountBalance,
    ExecutionJob,
    ExecutionJobStatus,
    Order,
    OrderStatus,
    RiskLimit,
    SourceTradeEvent,
    TradingAccount,
    VirtualPosition,
)
from backend.app.services.account_service import TradingAccountService
from backend.app.services.copy_engine import (
    CopyTradingEngine,
    OrderBookSnapshot,
    compute_taker_fee_usd,
)
from backend.app.services.metrics_service import GLOBAL_METRICS, PlatformMetricsRegistry
from backend.app.services.polymarket_signer import (
    build_and_sign_v2_order,
    build_l2_auth_headers,
)


class ExchangeTransportError(Exception):
    """Raised when an HTTP call to Polymarket fails."""

    def __init__(self, message: str, *, is_ambiguous: bool = False, retryable: bool = False) -> None:
        super().__init__(message)
        self.message = message
        self.is_ambiguous = is_ambiguous
        self.retryable = retryable


@dataclass
class ExchangePlacementResult:
    exchange_order_id: str
    status: str  # "MATCHED" | "OPEN" | "CANCELLED" | "REJECTED" | "ACKNOWLEDGED"
    filled_shares: Decimal = Decimal("0")
    avg_fill_price: Optional[Decimal] = None
    tx_hash: Optional[str] = None


class PolymarketExchangeGateway(Protocol):
    """Protocol implemented by the live HTTP CLOB client and test mock servers."""

    def submit_signed_order(
        self,
        *,
        signed_payload: dict[str, Any],
        headers: dict[str, str],
    ) -> ExchangePlacementResult: ...

    def submit_batch_orders(
        self,
        *,
        signed_payloads: list[dict[str, Any]],
        headers: dict[str, str],
    ) -> list[ExchangePlacementResult]: ...

    def get_order_status(
        self,
        *,
        exchange_order_id: str,
        headers: dict[str, str],
    ) -> Optional[ExchangePlacementResult]: ...


class TokenBucketLimiter:
    """Thread-safe token bucket rate limiter for per-account and global API throttling."""

    def __init__(self, rate_per_sec: float, capacity: float) -> None:
        self._lock = threading.Lock()
        self.rate_per_sec = max(0.01, float(rate_per_sec))
        self.capacity = max(1.0, float(capacity))
        self.tokens = self.capacity
        self.last_refill = time.monotonic()

    def try_acquire(self, cost: float = 1.0) -> bool:
        with self._lock:
            now = time.monotonic()
            elapsed = max(0.0, now - self.last_refill)
            self.tokens = min(self.capacity, self.tokens + elapsed * self.rate_per_sec)
            self.last_refill = now
            if self.tokens >= cost:
                self.tokens -= cost
                return True
            return False


@dataclass
class AccountWorkerRuntimeState:
    trading_account_id: int
    user_id: int
    max_queue_size: int = 256
    queue: deque[int] = field(default_factory=deque)
    rate_limiter: TokenBucketLimiter = field(
        default_factory=lambda: TokenBucketLimiter(rate_per_sec=2.0, capacity=10.0)
    )


def _deterministic_salt_for_job(job_id: int, account_id: int) -> int:
    digest = hashlib.sha256(f"pommarket-v2-salt:job:{job_id}:acct:{account_id}".encode("ascii")).digest()
    return int.from_bytes(digest[:8], "big") & 0x7FFFFFFFFFFFFFFF


class LiveExecutionWorkerPool:
    """Manages independent execution workers per destination trading account.

    Guarantees (including Fix E-03 Crash-Safe Order Submission):
    - Isolated decrypted signing credentials per account
    - Separate bounded queues with backpressure per account
    - Per-account AND global rate limiting (never assuming 50 accounts = 50x API limit)
    - Explicit durable order lifecycle (`PREPARED` -> `SUBMITTING` -> `ACKNOWLEDGED` / `FILLED` / `PARTIAL` / `AMBIGUOUS` / `CANCELLED` / `REJECTED`)
    - Durable execution intent (`status=SUBMITTING`, `client_order_id`, `eip712_order_hash`, `signed_payload_json`)
      is **committed to PostgreSQL BEFORE sending `POST /order`** so a crash during or after HTTP submission
      never rolls back the order record and never triggers a duplicate submission on restart.
    """

    def __init__(
        self,
        exchange_gateway: PolymarketExchangeGateway,
        *,
        account_service: Optional[TradingAccountService] = None,
        copy_engine: Optional[CopyTradingEngine] = None,
        metrics: Optional[PlatformMetricsRegistry] = None,
        global_rps: float = 20.0,
        per_account_rps: float = 2.0,
        max_queue_per_account: int = 256,
        crash_hook: Optional[Callable[[str, Order], None]] = None,
    ) -> None:
        self._gateway = exchange_gateway
        self._account_service = account_service or TradingAccountService()
        self._copy_engine = copy_engine or CopyTradingEngine()
        self._metrics = metrics or GLOBAL_METRICS
        self._global_limiter = TokenBucketLimiter(rate_per_sec=global_rps, capacity=max(5.0, global_rps))
        self._per_account_rps = per_account_rps
        self._max_queue_per_account = max_queue_per_account
        self._crash_hook = crash_hook
        self._lock = threading.Lock()
        self._workers: dict[int, AccountWorkerRuntimeState] = {}

    def get_or_create_worker_state(self, trading_account_id: int, user_id: int) -> AccountWorkerRuntimeState:
        with self._lock:
            state = self._workers.get(trading_account_id)
            if state is None:
                state = AccountWorkerRuntimeState(
                    trading_account_id=trading_account_id,
                    user_id=user_id,
                    max_queue_size=self._max_queue_per_account,
                    rate_limiter=TokenBucketLimiter(
                        rate_per_sec=self._per_account_rps,
                        capacity=max(2.0, self._per_account_rps * 5),
                    ),
                )
                self._workers[trading_account_id] = state
            return state

    def enqueue_job(self, trading_account_id: int, user_id: int, job_id: int) -> bool:
        """Enqueue a job to the account's isolated bounded queue. Returns False on backpressure."""
        state = self.get_or_create_worker_state(trading_account_id, user_id)
        with self._lock:
            if len(state.queue) >= state.max_queue_size:
                return False
            state.queue.append(job_id)
            return True

    def execute_live_order(
        self,
        session: Session,
        *,
        order: Order,
        job: ExecutionJob,
        account: TradingAccount,
        vpos: VirtualPosition,
        balance: AccountBalance,
        risk: RiskLimit,
        book: OrderBookSnapshot,
        source_event: SourceTradeEvent,
    ) -> ExecutionJob:
        state = self.get_or_create_worker_state(account.id, account.user_id)

        # Enforce both per-account and global rate limiters
        if not state.rate_limiter.try_acquire(1.0) or not self._global_limiter.try_acquire(1.0):
            order.status = OrderStatus.REJECTED.value
            order.error_message = "rate_limited_by_worker_token_bucket"
            job.status = ExecutionJobStatus.SKIPPED.value
            job.skip_or_error_reason = "rate_limited"
            job.completed_at = datetime.now(timezone.utc)
            self._metrics.inc("jobs_skipped_total")
            session.commit()
            return job

        # Decrypt isolated signing credentials strictly for this account's owner
        bundle = self._account_service.get_decrypted_signing_bundle(
            session,
            user_id=account.user_id,
            account_id=account.id,
        )
        private_key = bundle.get("private_key")
        api_key = bundle.get("api_key")
        api_secret = bundle.get("api_secret")
        api_passphrase = bundle.get("api_passphrase")

        if not (private_key and api_key and api_secret and api_passphrase):
            order.status = OrderStatus.REJECTED.value
            order.error_message = "Missing live signing credentials"
            job.status = ExecutionJobStatus.FAILED.value
            job.skip_or_error_reason = order.error_message
            job.completed_at = datetime.now(timezone.utc)
            session.commit()
            return job

        maker_address = account.funder_wallet_address or account.signer_wallet_address
        det_salt = _deterministic_salt_for_job(job.id, account.id)
        det_ts_ms = int(source_event.source_event_ts.timestamp() * 1000)

        signed = build_and_sign_v2_order(
            private_key_hex=private_key,
            maker_address=maker_address,
            signer_address=account.signer_wallet_address,
            signature_type=account.signature_type,
            token_id=order.token_id,
            side=order.side,
            price=Decimal(str(order.limit_price)),
            size_shares=Decimal(str(order.requested_size_shares)),
            api_key_owner=api_key,
            order_type="FAK",
            neg_risk=book.neg_risk,
            salt=det_salt,
            timestamp_ms=det_ts_ms,
        )

        # Stage 1 (PREPARED): Persist deterministic EIP-712 order hash, client_order_id, and signed payload
        order.client_order_id = order.client_order_id or f"pom_ord_job_{job.id}_acct_{account.id}"
        order.eip712_order_hash = signed.eip712_hash_hex
        order.signed_payload_json = signed.order_payload
        order.status = OrderStatus.PREPARED.value
        job.status = ExecutionJobStatus.PROCESSING.value
        session.commit()

        if self._crash_hook is not None:
            self._crash_hook("after_prepared_commit", order)

        # Stage 2 (SUBMITTING): Transition to SUBMITTING and commit durably BEFORE network I/O!
        # If the worker crashes during `submit_signed_order` or before receiving the HTTP response,
        # the database already holds `Order(status='SUBMITTING', eip712_order_hash=...)`, so restart
        # recovery will reconcile with Polymarket rather than sending a duplicate order.
        order.status = OrderStatus.SUBMITTING.value
        order.order_submitted_ts = datetime.now(timezone.utc)
        session.commit()

        if self._crash_hook is not None:
            self._crash_hook("before_http_post", order)

        body_str = json.dumps(signed.order_payload, separators=(",", ":"))
        headers = build_l2_auth_headers(
            poly_address=maker_address,
            api_key=api_key,
            api_secret=api_secret,
            api_passphrase=api_passphrase,
            method="POST",
            path="/order",
            body_str=body_str,
        )

        placement: Optional[ExchangePlacementResult] = None
        for attempt_idx in range(2):
            try:
                placement = self._gateway.submit_signed_order(
                    signed_payload=signed.order_payload,
                    headers=headers,
                )
                break
            except ExchangeTransportError as exc:
                # CRITICAL RULE: Never assume an order was rejected if an HTTP request timed out
                # or returned an ambiguous transport error! Mark AMBIGUOUS and wait for reconciliation.
                if exc.is_ambiguous:
                    order.status = OrderStatus.AMBIGUOUS.value
                    order.error_message = f"Ambiguous outcome (awaiting reconciliation): {exc.message}"
                    job.status = ExecutionJobStatus.AMBIGUOUS.value
                    job.skip_or_error_reason = order.error_message
                    self._metrics.inc("orders_ambiguous_total")
                    self._record_account_error(account, risk, reason=order.error_message)
                    session.commit()
                    return job

                # Only retry once if the exchange explicitly rejected before acceptance and it's marked retryable
                if exc.retryable and attempt_idx == 0:
                    job.attempts += 1
                    continue

                order.status = OrderStatus.REJECTED.value
                order.error_message = exc.message
                job.status = ExecutionJobStatus.FAILED.value
                job.skip_or_error_reason = exc.message
                job.completed_at = datetime.now(timezone.utc)
                self._metrics.inc("orders_rejected_total")
                self._record_account_error(account, risk, reason=exc.message)
                session.commit()
                return job

        assert placement is not None
        ack_ts = datetime.now(timezone.utc)
        order.exchange_order_id = placement.exchange_order_id
        order.order_ack_ts = ack_ts
        order.tx_hash = placement.tx_hash
        order.status = OrderStatus.ACKNOWLEDGED.value
        session.commit()

        if self._crash_hook is not None:
            self._crash_hook("after_acknowledged_commit", order)

        if placement.status == "CANCELLED" and placement.filled_shares <= Decimal("0"):
            order.status = OrderStatus.CANCELLED.value
            order.error_message = "Exchange returned CANCELLED with zero fill"
            job.status = ExecutionJobStatus.FAILED.value
            job.skip_or_error_reason = order.error_message
            job.completed_at = ack_ts
            self._metrics.inc("orders_rejected_total")
            self._record_account_error(account, risk, reason=order.error_message)
            session.commit()
            return job

        if placement.status == "REJECTED" and placement.filled_shares <= Decimal("0"):
            order.status = OrderStatus.REJECTED.value
            order.error_message = "Exchange returned REJECTED with zero fill"
            job.status = ExecutionJobStatus.FAILED.value
            job.skip_or_error_reason = order.error_message
            job.completed_at = ack_ts
            self._metrics.inc("orders_rejected_total")
            self._record_account_error(account, risk, reason=order.error_message)
            session.commit()
            return job

        if placement.filled_shares <= Decimal("0"):
            # Order accepted by exchange, but fill size not yet confirmed -> mark AMBIGUOUS for reconciliation
            order.status = OrderStatus.AMBIGUOUS.value
            order.error_message = "Order accepted by exchange; awaiting fill confirmation via reconciliation"
            job.status = ExecutionJobStatus.AMBIGUOUS.value
            job.skip_or_error_reason = order.error_message
            self._metrics.inc("orders_ambiguous_total")
            session.commit()
            return job

        fill_price = placement.avg_fill_price or Decimal(str(order.limit_price))
        notional = (placement.filled_shares * fill_price).quantize(Decimal("0.000001"))
        fee = compute_taker_fee_usd(fill_price, placement.filled_shares, book.fee_rate, book.fee_exponent)

        self._copy_engine.apply_confirmed_fill(
            session,
            order=order,
            job=job,
            account=account,
            vpos=vpos,
            balance=balance,
            risk=risk,
            filled_shares=placement.filled_shares,
            avg_price=fill_price,
            notional_usd=notional,
            fee_usd=fee,
            fill_ts=ack_ts,
            source_event=source_event,
        )
        session.commit()
        return job

    def _record_account_error(self, account: TradingAccount, risk: RiskLimit, *, reason: str) -> None:
        account.consecutive_errors = int(account.consecutive_errors or 0) + 1
        if account.consecutive_errors >= int(risk.circuit_breaker_error_threshold):
            account.circuit_breaker_tripped = True
            account.emergency_stop_reason = f"Circuit breaker tripped after {account.consecutive_errors} errors: {reason}"
