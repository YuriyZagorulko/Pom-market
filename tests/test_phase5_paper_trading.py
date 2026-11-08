"""Phase 5 Automated Tests: BUY/SELL Copy-Trading Engine, Virtual Position Attribution, Sizing Modes, Partial Fills, and Risk Controls."""

from __future__ import annotations

from datetime import datetime, timezone
from decimal import Decimal

from sqlalchemy import select

from backend.app.models import (
    ExecutionJobStatus,
    ExecutionMode,
    OperatingMode,
    Order,
    OrderStatus,
    SignatureType,
    VirtualPosition,
)
from backend.app.schemas import (
    EmergencyStopRequest,
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
from backend.app.services.source_trader_service import SourceTraderService


def _make_book(token_id: str, *, ask_price: str = "0.50", ask_size: str = "500", bid_price: str = "0.49", bid_size: str = "500") -> OrderBookSnapshot:
    # Include an expensive ask first to verify descending-ask sorting trap protection!
    return OrderBookSnapshot.from_raw(
        token_id,
        {
            "asks": [
                {"price": "0.99", "size": "1000"},
                {"price": ask_price, "size": ask_size},
            ],
            "bids": [
                {"price": "0.01", "size": "1000"},
                {"price": bid_price, "size": bid_size},
            ],
            "tick_size": "0.01",
            "fee_rate": "0.02",
            "fee_exponent": "1.0",
        },
    )


def test_virtual_position_source_attribution_and_partial_exits(db_session):
    """Critical specification test:
    If source trader A and source trader B both buy the same YES token,
    and A subsequently sells, ONLY the position attributable to A is considered
    for copying that sale, and never more than A's attributable lot!
    """
    auth = AuthService()
    acct_svc = TradingAccountService()
    st_svc = SourceTraderService(acct_svc)
    collector = MarketDataCollector()
    router = AccountRouterIndex()
    processor = EventProcessingService(router)
    engine = CopyTradingEngine()

    user = auth.register_user(
        db_session, UserRegisterRequest(email="paper@pom.io", password="Password123!")
    ).user
    acc = acct_svc.create_trading_account(
        db_session,
        user_id=user.id,
        req=TradingAccountCreateRequest(
            display_name="Paper Account 1",
            signer_wallet_address="0x90f79bf6eb2c4f870365e785982e1f101e93b906",
            signature_type=SignatureType.EOA,
            initial_paper_balance_usd=Decimal("1000.0"),
        ),
    )
    acct_svc.change_operating_mode(
        db_session,
        user_id=user.id,
        account_id=acc.id,
        req=OperatingModeChangeRequest(operating_mode=OperatingMode.PAPER_TRADING),
    )

    trader_a = "0xaaaa000000000000000000000000000000000001"
    trader_b = "0xbbbb000000000000000000000000000000000002"
    token_yes = "10000000000000000000000000000000000001"

    # Trader A copies with $20 fixed amount; Trader B copies with $40 fixed amount
    st_svc.add_source_trader_to_account(
        db_session,
        user_id=user.id,
        account_id=acc.id,
        req=SourceTraderAssignRequest(
            wallet_address=trader_a,
            custom_name="Trader A",
            execution_mode=ExecutionMode.FIXED_AMOUNT,
            fixed_amount_usd=Decimal("20.0"),
        ),
    )
    st_svc.add_source_trader_to_account(
        db_session,
        user_id=user.id,
        account_id=acc.id,
        req=SourceTraderAssignRequest(
            wallet_address=trader_b,
            custom_name="Trader B",
            execution_mode=ExecutionMode.FIXED_AMOUNT,
            fixed_amount_usd=Decimal("40.0"),
        ),
    )
    router.reload_from_db(db_session)

    now_ts = int(datetime.now(timezone.utc).timestamp()) + 2
    book = _make_book(token_yes, ask_price="0.50", bid_price="0.50")

    # 1. Trader A buys 100 shares @ $0.50 -> Account 1 buys $20 / $0.50 = 40 shares attributed to Trader A
    ev_a_buy = collector.parse_activity_poll_rows(
        [
            {
                "proxyWallet": trader_a,
                "conditionId": "0xcond_election",
                "type": "TRADE",
                "side": "BUY",
                "price": "0.50",
                "size": "100",
                "timestamp": now_ts,
                "transactionHash": "0xtx_a_buy_1",
                "asset": token_yes,
            }
        ]
    )[0]
    job_a_buy = processor.process_event(db_session, ev_a_buy).created_jobs[0]
    engine.execute_job(db_session, job_a_buy, book)
    assert job_a_buy.status == ExecutionJobStatus.COMPLETED.value

    # 2. Trader B buys 200 shares @ $0.50 -> Account 1 buys $40 / $0.50 = 80 shares attributed to Trader B
    ev_b_buy = collector.parse_activity_poll_rows(
        [
            {
                "proxyWallet": trader_b,
                "conditionId": "0xcond_election",
                "type": "TRADE",
                "side": "BUY",
                "price": "0.50",
                "size": "200",
                "timestamp": now_ts + 1,
                "transactionHash": "0xtx_b_buy_1",
                "asset": token_yes,
            }
        ]
    )[0]
    job_b_buy = processor.process_event(db_session, ev_b_buy).created_jobs[0]
    engine.execute_job(db_session, job_b_buy, book)
    assert job_b_buy.status == ExecutionJobStatus.COMPLETED.value

    # Verify two distinct virtual position lots exist
    lot_a = db_session.scalar(
        select(VirtualPosition).where(
            VirtualPosition.trading_account_id == acc.id,
            VirtualPosition.source_wallet == trader_a,
            VirtualPosition.token_id == token_yes,
        )
    )
    lot_b = db_session.scalar(
        select(VirtualPosition).where(
            VirtualPosition.trading_account_id == acc.id,
            VirtualPosition.source_wallet == trader_b,
            VirtualPosition.token_id == token_yes,
        )
    )
    assert lot_a.open_shares == Decimal("40.000000")
    assert lot_b.open_shares == Decimal("80.000000")

    # 3. Trader A sells 50% of their observed 100 shares (50 shares) @ $0.60
    #    Account 1 must sell 50% of Trader A's 40-share lot = 20 shares, leaving Trader B's 80 shares untouched!
    sell_book = _make_book(token_yes, ask_price="0.61", bid_price="0.60")
    ev_a_sell_half = collector.parse_activity_poll_rows(
        [
            {
                "proxyWallet": trader_a,
                "conditionId": "0xcond_election",
                "type": "TRADE",
                "side": "SELL",
                "price": "0.60",
                "size": "50",
                "timestamp": now_ts + 2,
                "transactionHash": "0xtx_a_sell_half",
                "asset": token_yes,
            }
        ]
    )[0]
    job_a_sell_1 = processor.process_event(db_session, ev_a_sell_half).created_jobs[0]
    engine.execute_job(db_session, job_a_sell_1, sell_book)
    assert job_a_sell_1.status == ExecutionJobStatus.COMPLETED.value

    db_session.refresh(lot_a)
    db_session.refresh(lot_b)
    assert lot_a.open_shares == Decimal("20.000000")
    assert lot_b.open_shares == Decimal("80.000000")

    # 4. Trader A now sells 500 shares (more than their remaining 50 observed shares!).
    #    Account 1 must cap the sale at Trader A's remaining 20 shares, NEVER touching Trader B's 80 shares!
    ev_a_sell_excess = collector.parse_activity_poll_rows(
        [
            {
                "proxyWallet": trader_a,
                "conditionId": "0xcond_election",
                "type": "TRADE",
                "side": "SELL",
                "price": "0.60",
                "size": "500",
                "timestamp": now_ts + 3,
                "transactionHash": "0xtx_a_sell_excess",
                "asset": token_yes,
            }
        ]
    )[0]
    job_a_sell_2 = processor.process_event(db_session, ev_a_sell_excess).created_jobs[0]
    engine.execute_job(db_session, job_a_sell_2, sell_book)
    assert job_a_sell_2.status == ExecutionJobStatus.COMPLETED.value

    db_session.refresh(lot_a)
    db_session.refresh(lot_b)
    assert lot_a.open_shares == Decimal("0")
    assert lot_a.closed_at is not None
    assert lot_a.realized_pnl_usd > Decimal("0")  # Bought at 0.50, sold at 0.60 minus taker fee
    assert lot_b.open_shares == Decimal("80.000000")


def test_execution_modes_partial_fills_slippage_balance_and_emergency_stop(db_session):
    auth = AuthService()
    acct_svc = TradingAccountService()
    st_svc = SourceTraderService(acct_svc)
    collector = MarketDataCollector()
    router = AccountRouterIndex()
    processor = EventProcessingService(router)
    engine = CopyTradingEngine()

    user = auth.register_user(
        db_session, UserRegisterRequest(email="risk@pom.io", password="Password123!")
    ).user
    acc = acct_svc.create_trading_account(
        db_session,
        user_id=user.id,
        req=TradingAccountCreateRequest(
            display_name="Risk Tested Account",
            signer_wallet_address="0x90f79bf6eb2c4f870365e785982e1f101e93b906",
            signature_type=SignatureType.EOA,
            initial_paper_balance_usd=Decimal("100.0"),
        ),
    )
    acct_svc.change_operating_mode(
        db_session,
        user_id=user.id,
        account_id=acc.id,
        req=OperatingModeChangeRequest(operating_mode=OperatingMode.PAPER_TRADING),
    )

    whale = "0xcccc000000000000000000000000000000000003"
    token_id = "20000000000000000000000000000000000002"

    # Configure FIXED_PERCENTAGE (20% of source trade notional) with max_slippage_pct = 5%
    st_svc.add_source_trader_to_account(
        db_session,
        user_id=user.id,
        account_id=acc.id,
        req=SourceTraderAssignRequest(
            wallet_address=whale,
            execution_mode=ExecutionMode.FIXED_PERCENTAGE,
            copy_percentage=Decimal("0.20"),
            copy_multiplier=Decimal("1.0"),
            max_slippage_pct=Decimal("0.05"),
        ),
    )
    router.reload_from_db(db_session)
    now_ts = int(datetime.now(timezone.utc).timestamp()) + 2

    # 1. Partial fill test: Whale buys $200 notional (400 shares @ $0.50) -> 20% = $40 target (80 shares @ $0.50),
    #    but the order book only has 30 shares at $0.50 inside the limit!
    thin_book = _make_book(token_id, ask_price="0.50", ask_size="30", bid_price="0.49", bid_size="30")
    ev1 = collector.parse_activity_poll_rows(
        [
            {
                "proxyWallet": whale,
                "conditionId": "0xcond_thin",
                "type": "TRADE",
                "side": "BUY",
                "price": "0.50",
                "size": "400",
                "timestamp": now_ts,
                "transactionHash": "0xtx_partial_1",
                "asset": token_id,
            }
        ]
    )[0]
    job1 = processor.process_event(db_session, ev1).created_jobs[0]
    engine.execute_job(db_session, job1, thin_book)
    order1 = db_session.scalar(select(Order).where(Order.execution_job_id == job1.id))
    assert order1.status == OrderStatus.PARTIAL.value
    assert order1.filled_size_shares == Decimal("30.000000")

    # 2. Slippage exceeded test: Whale buys at $0.50, max_slippage = 5% (limit $0.52), best ask is $0.60
    expensive_book = _make_book(token_id, ask_price="0.60", ask_size="500")
    ev2 = collector.parse_activity_poll_rows(
        [
            {
                "proxyWallet": whale,
                "conditionId": "0xcond_thin",
                "type": "TRADE",
                "side": "BUY",
                "price": "0.50",
                "size": "400",
                "timestamp": now_ts + 1,
                "transactionHash": "0xtx_slip_2",
                "asset": token_id,
            }
        ]
    )[0]
    job2 = processor.process_event(db_session, ev2).created_jobs[0]
    engine.execute_job(db_session, job2, expensive_book)
    assert job2.status == ExecutionJobStatus.SKIPPED.value
    assert job2.skip_or_error_reason == "slippage_exceeded"

    # 3. Insufficient balance test: Whale buys $1000 notional -> 20% = $200 target, but remaining cash is ~$85
    deep_book = _make_book(token_id, ask_price="0.50", ask_size="1000")
    acct_svc.update_risk_limits(
        db_session,
        user_id=user.id,
        account_id=acc.id,
        req=RiskLimitUpdateRequest(max_order_size_usd=Decimal("500.0")),
    )
    st_svc.add_source_trader_to_account(
        db_session,
        user_id=user.id,
        account_id=acc.id,
        req=SourceTraderAssignRequest(
            wallet_address=whale,
            execution_mode=ExecutionMode.FIXED_PERCENTAGE,
            copy_percentage=Decimal("0.20"),
            max_order_size_usd=Decimal("500.0"),
        ),
    )
    ev3 = collector.parse_activity_poll_rows(
        [
            {
                "proxyWallet": whale,
                "conditionId": "0xcond_thin",
                "type": "TRADE",
                "side": "BUY",
                "price": "0.50",
                "size": "2000",
                "timestamp": now_ts + 2,
                "transactionHash": "0xtx_bal_3",
                "asset": token_id,
            }
        ]
    )[0]
    job3 = processor.process_event(db_session, ev3).created_jobs[0]
    engine.execute_job(db_session, job3, deep_book)
    assert job3.status == ExecutionJobStatus.SKIPPED.value
    assert job3.skip_or_error_reason == "insufficient_balance"

    # 4. Per-account emergency stop test
    acct_svc.set_emergency_stop(
        db_session,
        user_id=user.id,
        account_id=acc.id,
        req=EmergencyStopRequest(emergency_stop=True, reason="Operator kill switch"),
    )
    ev4 = collector.parse_activity_poll_rows(
        [
            {
                "proxyWallet": whale,
                "conditionId": "0xcond_thin",
                "type": "TRADE",
                "side": "BUY",
                "price": "0.50",
                "size": "100",
                "timestamp": now_ts + 3,
                "transactionHash": "0xtx_kill_4",
                "asset": token_id,
            }
        ]
    )[0]
    # Force-create a queued job before stop or execute while stopped
    job4 = processor.process_event(db_session, ev4).created_jobs[0]
    engine.execute_job(db_session, job4, deep_book)
    assert job4.status == ExecutionJobStatus.SKIPPED.value
    assert job4.skip_or_error_reason == "account_emergency_stop"
