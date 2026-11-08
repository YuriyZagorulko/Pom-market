"""Phase 6 Automated Tests: EIP-712 & L2 HMAC Signing, Live Execution Workers, Ambiguous HTTP Outcomes, Reconciliation, Circuit Breakers, and Restart Recovery."""

from __future__ import annotations

from datetime import datetime, timezone
from decimal import Decimal
from typing import Any, Optional

from sqlalchemy import select

from backend.app.models import (
    ExecutionJobStatus,
    OperatingMode,
    Order,
    OrderStatus,
    SignatureType,
    VirtualPosition,
)
from backend.app.schemas import (
    OperatingModeChangeRequest,
    RiskLimitUpdateRequest,
    SourceTraderAssignRequest,
    TradingAccountCreateRequest,
    UserRegisterRequest,
)
from backend.app.services.account_router import AccountRouterIndex
from backend.app.services.account_service import TradingAccountService
from backend.app.services.auth_service import AuthService
from backend.app.services.collector_service import MarketDataCollector
from backend.app.services.copy_engine import CopyTradingEngine, OrderBookSnapshot
from backend.app.services.event_processor import EventProcessingService
from backend.app.services.live_execution_worker import (
    ExchangePlacementResult,
    ExchangeTransportError,
    LiveExecutionWorkerPool,
)
from backend.app.services.polymarket_signer import (
    ORDER_V2_TYPE_HASH,
    ORDER_V2_TYPE_STRING,
    build_and_sign_v2_order,
    compute_l2_hmac_signature,
    keccak256,
)
from backend.app.services.reconciliation_service import ReconciliationService
from backend.app.services.source_trader_service import SourceTraderService


class MockPolymarketExchange:
    """Deterministic mock of Polymarket CLOB v2 REST API for testing live execution & reconciliation."""

    def __init__(self) -> None:
        self.submitted_payloads: list[dict[str, Any]] = []
        self.submitted_headers: list[dict[str, str]] = []
        self.mode: str = "fill_immediately"
        self.remote_orders: dict[str, ExchangePlacementResult] = {}
        self._seq: int = 1

    def submit_signed_order(
        self,
        *,
        signed_payload: dict[str, Any],
        headers: dict[str, str],
    ) -> ExchangePlacementResult:
        self.submitted_payloads.append(signed_payload)
        self.submitted_headers.append(headers)

        if self.mode == "http_timeout_ambiguous":
            # Simulate an order that actually landed on Polymarket, but the HTTP response timed out!
            ord_id = f"polymarket_live_ord_{self._seq}"
            self._seq += 1
            self.remote_orders[ord_id] = ExchangePlacementResult(
                exchange_order_id=ord_id,
                status="MATCHED",
                filled_shares=Decimal("20.00"),
                avg_fill_price=Decimal("0.48"),
                tx_hash="0xmatched_on_chain_despite_timeout",
            )
            # Also index by idempotency fallback so reconciliation can look it up even when exchange_order_id wasn't received
            raise ExchangeTransportError("HTTP 504 Gateway Timeout on POST /order", is_ambiguous=True)

        if self.mode == "transient_reject_then_fill":
            self.mode = "fill_immediately"
            raise ExchangeTransportError("429 Transient book contention", is_ambiguous=False, retryable=True)

        if self.mode == "hard_reject":
            raise ExchangeTransportError("400 Invalid order parameters", is_ambiguous=False, retryable=False)

        ord_id = f"polymarket_live_ord_{self._seq}"
        self._seq += 1
        res = ExchangePlacementResult(
            exchange_order_id=ord_id,
            status="MATCHED",
            filled_shares=Decimal("20.00"),
            avg_fill_price=Decimal("0.50"),
            tx_hash=f"0xtx_{ord_id}",
        )
        self.remote_orders[ord_id] = res
        return res

    def submit_batch_orders(
        self,
        *,
        signed_payloads: list[dict[str, Any]],
        headers: dict[str, str],
    ) -> list[ExchangePlacementResult]:
        return [self.submit_signed_order(signed_payload=p, headers=headers) for p in signed_payloads]

    def get_order_status(
        self,
        *,
        exchange_order_id: str,
        headers: dict[str, str],
    ) -> Optional[ExchangePlacementResult]:
        if exchange_order_id in self.remote_orders:
            return self.remote_orders[exchange_order_id]
        # Return most recent matched order when queried by client idempotency key after HTTP timeout
        if self.remote_orders:
            return list(self.remote_orders.values())[-1]
        return None


def test_eip712_v2_order_signing_and_l2_hmac_vectors():
    """Verify our clean-room Keccak-256, EIP-712 Order V2 struct signing, and L2 URL-safe Base64 HMAC."""
    import base64

    # 1. Verify L2 HMAC known test vector
    secret_b64 = base64.urlsafe_b64encode(b"0123456789abcdef0123456789abcdef").decode("ascii")
    sig = compute_l2_hmac_signature(
        api_secret_b64=secret_b64,
        timestamp_str="1700000000",
        method="GET",
        path="/orders",
        body_str=None,
    )
    assert sig == "d_8rYNiskB9DSmPVGBEqPIK9veia9oWolAp_uuLjkXA="

    # 2. Verify EIP-712 Order V2 signing structure and determinism
    assert ORDER_V2_TYPE_HASH == keccak256(ORDER_V2_TYPE_STRING.encode("ascii"))
    test_pk = "7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6"
    signed_a = build_and_sign_v2_order(
        private_key_hex=test_pk,
        maker_address="0x1111111111111111111111111111111111111111",
        signer_address="0x90F79bf6EB2c4f870365E785982E1f101E93b906",
        signature_type=SignatureType.POLY_GNOSIS_SAFE.value,
        token_id="1234",
        side="BUY",
        price=Decimal("0.62"),
        size_shares=Decimal("100"),
        api_key_owner="api-key-uuid",
        order_type="FAK",
        salt=42,
        timestamp_ms=1700000000000,
    )
    signed_b = build_and_sign_v2_order(
        private_key_hex=test_pk,
        maker_address="0x1111111111111111111111111111111111111111",
        signer_address="0x90F79bf6EB2c4f870365E785982E1f101E93b906",
        signature_type=SignatureType.POLY_GNOSIS_SAFE.value,
        token_id="1234",
        side="BUY",
        price=Decimal("0.62"),
        size_shares=Decimal("100"),
        api_key_owner="api-key-uuid",
        order_type="FAK",
        salt=42,
        timestamp_ms=1700000000000,
    )
    assert signed_a.signature_hex == signed_b.signature_hex
    assert len(signed_a.signature_hex) == 2 + 130
    assert signed_a.order_payload["orderType"] == "FAK"
    assert signed_a.order_payload["order"]["makerAmount"] == "62000000"
    assert signed_a.order_payload["order"]["takerAmount"] == "100000000"
    assert signed_a.order_payload["order"]["signatureType"] == 2


def test_live_execution_ambiguous_http_timeout_reconciliation_and_circuit_breaker(db_session):
    """Critical specification test:
    1. Never retry an order when an HTTP request times out (marks AMBIGUOUS, 1 request sent).
    2. Reconciliation Service queries Polymarket, discovers the order actually filled at $0.48,
       and books the fill + virtual position without duplicate execution!
    3. Consecutive hard rejections trip the account's error circuit breaker.
    4. Worker restart recovers unfinished QUEUED jobs idempotently.
    """
    import base64

    auth = AuthService()
    acct_svc = TradingAccountService()
    st_svc = SourceTraderService(acct_svc)
    collector = MarketDataCollector()
    router = AccountRouterIndex()
    processor = EventProcessingService(router)
    engine = CopyTradingEngine()
    mock_exchange = MockPolymarketExchange()
    worker_pool = LiveExecutionWorkerPool(mock_exchange, account_service=acct_svc, copy_engine=engine)
    reconciler = ReconciliationService(mock_exchange, account_service=acct_svc, copy_engine=engine)

    user = auth.register_user(
        db_session, UserRegisterRequest(email="live_trader@pom.io", password="Password123!")
    ).user
    secret_b64 = base64.urlsafe_b64encode(b"live_account_hmac_secret_32bytes").decode("ascii")
    acc = acct_svc.create_trading_account(
        db_session,
        user_id=user.id,
        req=TradingAccountCreateRequest(
            display_name="Live Safe Account",
            signer_wallet_address="0x90f79bf6eb2c4f870365e785982e1f101e93b906",
            funder_wallet_address="0x1111111111111111111111111111111111111111",
            signature_type=SignatureType.POLY_GNOSIS_SAFE,
            private_key="0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6",
            api_key="live-api-key",
            api_secret=secret_b64,
            api_passphrase="live-passphrase",
        ),
    )
    # Fund live balance for testing
    from backend.app.models import AccountBalance

    live_bal = db_session.scalar(
        select(AccountBalance).where(
            AccountBalance.trading_account_id == acc.id,
            AccountBalance.operating_mode == OperatingMode.LIVE_TRADING.value,
        )
    )
    live_bal.available_cash_usd = Decimal("500.0")
    live_bal.total_equity_usd = Decimal("500.0")

    acct_svc.change_operating_mode(
        db_session,
        user_id=user.id,
        account_id=acc.id,
        req=OperatingModeChangeRequest(
            operating_mode=OperatingMode.LIVE_TRADING,
            explicit_confirmation="CONFIRM_LIVE_TRADING",
        ),
    )
    acct_svc.update_risk_limits(
        db_session,
        user_id=user.id,
        account_id=acc.id,
        req=RiskLimitUpdateRequest(circuit_breaker_error_threshold=2),
    )

    whale = "0xdddd000000000000000000000000000000000004"
    token_id = "30000000000000000000000000000000000003"
    st_svc.add_source_trader_to_account(
        db_session,
        user_id=user.id,
        account_id=acc.id,
        req=SourceTraderAssignRequest(
            wallet_address=whale,
            fixed_amount_usd=Decimal("10.0"),
        ),
    )
    router.reload_from_db(db_session)
    now_ts = int(datetime.now(timezone.utc).timestamp()) + 2
    book = OrderBookSnapshot.from_raw(
        token_id,
        {
            "asks": [{"price": "0.50", "size": "500"}],
            "bids": [{"price": "0.49", "size": "500"}],
            "tick_size": "0.01",
            "fee_rate": "0.02",
        },
    )

    # 1. Ambiguous HTTP timeout test:
    #    Exchange times out on POST /order. Worker MUST NOT retry (only 1 POST sent) and marks order AMBIGUOUS.
    mock_exchange.mode = "http_timeout_ambiguous"
    ev1 = collector.parse_activity_poll_rows(
        [
            {
                "proxyWallet": whale,
                "conditionId": "0xcond_live",
                "type": "TRADE",
                "side": "BUY",
                "price": "0.50",
                "size": "100",
                "timestamp": now_ts,
                "transactionHash": "0xtx_live_timeout_1",
                "asset": token_id,
            }
        ]
    )[0]
    job1 = processor.process_event(db_session, ev1).created_jobs[0]
    engine.execute_job(db_session, job1, book, live_executor=worker_pool)

    assert len(mock_exchange.submitted_payloads) == 1  # Strictly NO retry on timeout!
    assert job1.status == ExecutionJobStatus.AMBIGUOUS.value
    ord1 = db_session.scalar(select(Order).where(Order.execution_job_id == job1.id))
    assert ord1.status == OrderStatus.AMBIGUOUS.value

    # 2. Run Reconciliation Service: discovers the timed-out order actually filled 20 shares @ $0.48!
    summary = reconciler.reconcile_ambiguous_orders(db_session)
    assert summary.ambiguous_orders_checked == 1
    assert summary.orders_resolved_filled == 1
    db_session.refresh(ord1)
    db_session.refresh(job1)
    assert ord1.status == OrderStatus.FILLED.value
    assert job1.status == ExecutionJobStatus.COMPLETED.value

    vpos = db_session.scalar(
        select(VirtualPosition).where(
            VirtualPosition.trading_account_id == acc.id,
            VirtualPosition.token_id == token_id,
        )
    )
    assert vpos.open_shares == Decimal("20.000000")
    assert vpos.avg_entry_price == Decimal("0.480000")

    # 3. Position reconciliation check against exchange balances
    div_ok = reconciler.reconcile_positions(
        db_session,
        trading_account_id=acc.id,
        exchange_balances_by_token={token_id: Decimal("20.00")},
    )
    assert div_ok[0].verdict == "AGREED"

    div_mismatch = reconciler.reconcile_positions(
        db_session,
        trading_account_id=acc.id,
        exchange_balances_by_token={token_id: Decimal("5.00")},
    )
    assert div_mismatch[0].verdict == "UNATTRIBUTED_DIVERGENCE"

    # 4. Restart recovery test: create a QUEUED job and recover it via `recover_unfinished_jobs_after_restart`
    mock_exchange.mode = "fill_immediately"
    ev2 = collector.parse_activity_poll_rows(
        [
            {
                "proxyWallet": whale,
                "conditionId": "0xcond_live",
                "type": "TRADE",
                "side": "BUY",
                "price": "0.50",
                "size": "100",
                "timestamp": now_ts + 1,
                "transactionHash": "0xtx_live_restart_2",
                "asset": token_id,
            }
        ]
    )[0]
    job2 = processor.process_event(db_session, ev2).created_jobs[0]
    assert job2.status == ExecutionJobStatus.QUEUED.value
    recovered_count = reconciler.recover_unfinished_jobs_after_restart(
        db_session,
        books_by_token={token_id: book},
        live_executor=worker_pool,
    )
    assert recovered_count == 1
    db_session.refresh(job2)
    assert job2.status == ExecutionJobStatus.COMPLETED.value

    # 5. Circuit breaker test: two consecutive hard rejections trip `circuit_breaker_tripped`
    mock_exchange.mode = "hard_reject"
    for idx in (3, 4):
        ev_err = collector.parse_activity_poll_rows(
            [
                {
                    "proxyWallet": whale,
                    "conditionId": "0xcond_live",
                    "type": "TRADE",
                    "side": "BUY",
                    "price": "0.50",
                    "size": "100",
                    "timestamp": now_ts + idx,
                    "transactionHash": f"0xtx_live_err_{idx}",
                    "asset": token_id,
                }
            ]
        )[0]
        job_err = processor.process_event(db_session, ev_err).created_jobs[0]
        engine.execute_job(db_session, job_err, book, live_executor=worker_pool)
        assert job_err.status == ExecutionJobStatus.FAILED.value

    updated_acc = acct_svc.get_account_response(db_session, user_id=user.id, account_id=acc.id)
    assert updated_acc.circuit_breaker_tripped is True
