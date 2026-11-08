"""Phase 4 Automated Tests: Market Data Collector, Normalization, Deduplication, Historical Guard, and Monitoring Mode Routing."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
from decimal import Decimal

from sqlalchemy import select

from backend.app.models import (
    ExecutionJob,
    ExecutionJobStatus,
    OperatingMode,
    Order,
    SignatureType,
    SourceTradeEvent,
)
from backend.app.schemas import (
    SourceTraderAssignRequest,
    TradingAccountCreateRequest,
    UserRegisterRequest,
)
from backend.app.services.account_router import AccountRouterIndex
from backend.app.services.account_service import TradingAccountService
from backend.app.services.auth_service import AuthService
from backend.app.services.collector_service import MarketDataCollector
from backend.app.services.event_processor import EventProcessingService
from backend.app.services.source_trader_service import SourceTraderService


def test_collector_normalizes_rtds_activity_and_polygon_logs_and_deduplicates(db_session):
    auth = AuthService()
    acct_svc = TradingAccountService()
    st_svc = SourceTraderService(acct_svc)
    collector = MarketDataCollector()
    router = AccountRouterIndex()
    processor = EventProcessingService(router)

    user = auth.register_user(
        db_session, UserRegisterRequest(email="monitor@pom.io", password="Password123!")
    ).user
    acc = acct_svc.create_trading_account(
        db_session,
        user_id=user.id,
        req=TradingAccountCreateRequest(
            display_name="Monitoring Account",
            signer_wallet_address="0x90f79bf6eb2c4f870365e785982e1f101e93b906",
            signature_type=SignatureType.EOA,
        ),
    )
    assert acc.operating_mode == OperatingMode.MONITORING.value

    whale_wallet = "0x06dc51826bc524d9a83770e7de9dd7e005b04524"
    st_svc.add_source_trader_to_account(
        db_session,
        user_id=user.id,
        account_id=acc.id,
        req=SourceTraderAssignRequest(
            wallet_address=whale_wallet,
            custom_name="Whale Alpha",
        ),
    )
    router.reload_from_db(db_session)

    now_epoch = int(datetime.now(timezone.utc).timestamp()) + 2

    # 1. Non-trade price update frame must be ignored by trade parser
    price_frame = {
        "topic": "crypto_prices",
        "type": "update",
        "payload": {"symbol": "BTC", "price": "95000"},
    }
    assert collector.parse_rtds_frame(price_frame) == []

    # 2. Valid RTDS trade frame is normalized and routed
    rtds_frame = {
        "topic": "activity",
        "type": "trades",
        "payload": {
            "proxyWallet": whale_wallet.upper(),
            "conditionId": "0xcond_lakers_celtics",
            "title": "Lakers vs Celtics",
            "outcome": "Lakers",
            "asset": "2446159436675684781373961997321275913450205936915730680811157268",
            "side": "BUY",
            "price": 0.42,
            "size": 120.5,
            "timestamp": now_epoch,
            "transactionHash": "0xa6e7a7b4e49982cd3e215d93203024dab2f2bf874a134df58620f6e28a4aee63",
        },
    }
    events = collector.parse_rtds_frame(rtds_frame)
    assert len(events) == 1
    res1 = processor.process_event(db_session, events[0])
    assert res1.was_duplicate is False
    assert res1.matched_targets_count == 1
    assert len(res1.created_jobs) == 1
    assert res1.created_jobs[0].operating_mode == OperatingMode.MONITORING.value
    assert res1.created_jobs[0].status == ExecutionJobStatus.COMPLETED.value

    # Verify MONITORING mode never creates an Order row
    orders_in_db = db_session.scalars(select(Order)).all()
    assert orders_in_db == []

    # 3. Same trade arriving from `/activity` fallback poll with slightly different decimal rounding is deduplicated!
    activity_rows = [
        {
            "proxyWallet": whale_wallet,
            "conditionId": "0xcond_lakers_celtics",
            "type": "TRADE",
            "side": "BUY",
            "price": "0.4200001",
            "size": "120.5000",
            "timestamp": now_epoch,
            "transactionHash": "0xa6e7a7b4e49982cd3e215d93203024dab2f2bf874a134df58620f6e28a4aee63",
            "asset": "2446159436675684781373961997321275913450205936915730680811157268",
        }
    ]
    poll_events = collector.parse_activity_poll_rows(activity_rows)
    assert len(poll_events) == 1
    res2 = processor.process_event(db_session, poll_events[0])
    assert res2.was_duplicate is True
    assert res2.created_jobs == []

    # 4. Same trade arriving from Polygon V2 OrderFilled log is also deduplicated on `(tx_hash, wallet, token_id, side)`
    polygon_log = {
        "address": "0xe111180000d2663c0091e4f400237545b87b996b",
        "topics": [
            "0xd543adfd945773f1a62f74f0ee55a5e3b9b1a28262980ba90b1a89f2ea84d8ee",
            "0xe87236d8296a11388b4edcf201d7b37d2fa4f9592fe28cb99736cb7959065c9c",
            "0x00000000000000000000000006dc51826bc524d9a83770e7de9dd7e005b04524",
            "0x00000000000000000000000083e8f2ea25df8e71fcaf2271c9edaae02860b9bc",
        ],
        "data": (
            "0x"
            + f"{0:064x}"  # side = 0 (BUY)
            + f"{int('2446159436675684781373961997321275913450205936915730680811157268'):064x}"
            + f"{50610000:064x}"  # maker_amount
            + f"{120500000:064x}"  # taker_amount
            + f"{0:064x}"
            + f"{0:064x}"
            + f"{0:064x}"
        ),
        "blockNumber": "0x59b6d01",
        "transactionHash": "0xa6e7a7b4e49982cd3e215d93203024dab2f2bf874a134df58620f6e28a4aee63",
    }
    chain_ev = collector.parse_polygon_v2_log(
        polygon_log,
        block_timestamp=datetime.fromtimestamp(now_epoch, tz=timezone.utc),
    )
    assert chain_ev is not None
    assert chain_ev.source_wallet == whale_wallet
    res3 = processor.process_event(db_session, chain_ev)
    assert res3.was_duplicate is True

    # 5. Historical trade before wallet assignment timestamp is recorded in `source_trade_events` (so dedup remembers it)
    #    but creates ZERO execution jobs!
    old_ts = int((datetime.now(timezone.utc) - timedelta(days=2)).timestamp())
    old_rows = [
        {
            "proxyWallet": whale_wallet,
            "conditionId": "0xcond_old",
            "type": "TRADE",
            "side": "BUY",
            "price": "0.55",
            "size": "50",
            "timestamp": old_ts,
            "transactionHash": "0xold_trade_hash_999",
            "asset": "999999999999999999999",
        }
    ]
    old_ev = collector.parse_activity_poll_rows(old_rows)[0]
    res_old = processor.process_event(db_session, old_ev)
    assert res_old.was_duplicate is False
    assert res_old.created_jobs == []

    # Verify total source_trade_events = 2, total execution_jobs = 1
    assert len(db_session.scalars(select(SourceTradeEvent)).all()) == 2
    assert len(db_session.scalars(select(ExecutionJob)).all()) == 1
