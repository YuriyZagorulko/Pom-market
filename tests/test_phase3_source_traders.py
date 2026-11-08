"""Phase 3 Automated Tests: Many-to-Many Source Trader Mappings, Tenant Isolation, CSV Import/Export, and 100,000+ Relationship Routing."""

from __future__ import annotations

from datetime import datetime, timezone
from decimal import Decimal
import time

from backend.app.models import ExecutionMode, OperatingMode, SignatureType
from backend.app.schemas import (
    SourceTraderAssignRequest,
    SourceTraderMappingUpdateRequest,
    TradingAccountCreateRequest,
    UserRegisterRequest,
)
from backend.app.services.account_router import AccountRouterIndex, RouteTarget
from backend.app.services.account_service import TradingAccountService
from backend.app.services.auth_service import AuthService
from backend.app.services.source_trader_service import SourceTraderService


def _eth_addr(index: int) -> str:
    return f"0x{index:040x}"


def test_many_to_many_source_traders_and_tenant_isolation(db_session):
    """Verify exact specification relationship model:
    User A:
      - A1 copies T1, T2, T3
      - A2 copies T2, T4
      - A3 copies T1, T5
    User B:
      - B1 copies T1, T6
      - B2 copies T3, T4
    Each account has its own independent configuration, and User A never sees User B's accounts.
    """
    auth = AuthService()
    acct_svc = TradingAccountService()
    st_svc = SourceTraderService(acct_svc)

    user_a = auth.register_user(
        db_session, UserRegisterRequest(email="user_a@pom.io", password="PasswordA!123")
    ).user
    user_b = auth.register_user(
        db_session, UserRegisterRequest(email="user_b@pom.io", password="PasswordB!123")
    ).user

    a1 = acct_svc.create_trading_account(
        db_session,
        user_id=user_a.id,
        req=TradingAccountCreateRequest(
            display_name="A1", signer_wallet_address=_eth_addr(101), signature_type=SignatureType.EOA
        ),
    )
    a2 = acct_svc.create_trading_account(
        db_session,
        user_id=user_a.id,
        req=TradingAccountCreateRequest(
            display_name="A2", signer_wallet_address=_eth_addr(102), signature_type=SignatureType.EOA
        ),
    )
    a3 = acct_svc.create_trading_account(
        db_session,
        user_id=user_a.id,
        req=TradingAccountCreateRequest(
            display_name="A3", signer_wallet_address=_eth_addr(103), signature_type=SignatureType.EOA
        ),
    )
    b1 = acct_svc.create_trading_account(
        db_session,
        user_id=user_b.id,
        req=TradingAccountCreateRequest(
            display_name="B1", signer_wallet_address=_eth_addr(201), signature_type=SignatureType.EOA
        ),
    )
    b2 = acct_svc.create_trading_account(
        db_session,
        user_id=user_b.id,
        req=TradingAccountCreateRequest(
            display_name="B2", signer_wallet_address=_eth_addr(202), signature_type=SignatureType.EOA
        ),
    )

    traders = {i: _eth_addr(1000 + i) for i in range(1, 7)}

    # Assign mappings with distinct per-account configurations
    for t_idx, amount in [(1, "15.0"), (2, "25.0"), (3, "35.0")]:
        st_svc.add_source_trader_to_account(
            db_session,
            user_id=user_a.id,
            account_id=a1.id,
            req=SourceTraderAssignRequest(
                wallet_address=traders[t_idx],
                custom_name=f"A1-T{t_idx}",
                fixed_amount_usd=Decimal(amount),
            ),
        )
    for t_idx, amount in [(2, "50.0"), (4, "60.0")]:
        st_svc.add_source_trader_to_account(
            db_session,
            user_id=user_a.id,
            account_id=a2.id,
            req=SourceTraderAssignRequest(
                wallet_address=traders[t_idx],
                custom_name=f"A2-T{t_idx}",
                fixed_amount_usd=Decimal(amount),
            ),
        )
    for t_idx, amount in [(1, "75.0"), (5, "85.0")]:
        st_svc.add_source_trader_to_account(
            db_session,
            user_id=user_a.id,
            account_id=a3.id,
            req=SourceTraderAssignRequest(
                wallet_address=traders[t_idx],
                custom_name=f"A3-T{t_idx}",
                fixed_amount_usd=Decimal(amount),
            ),
        )
    for t_idx, amount in [(1, "110.0"), (6, "120.0")]:
        st_svc.add_source_trader_to_account(
            db_session,
            user_id=user_b.id,
            account_id=b1.id,
            req=SourceTraderAssignRequest(
                wallet_address=traders[t_idx],
                custom_name=f"B1-T{t_idx}",
                fixed_amount_usd=Decimal(amount),
            ),
        )
    for t_idx, amount in [(3, "130.0"), (4, "140.0")]:
        st_svc.add_source_trader_to_account(
            db_session,
            user_id=user_b.id,
            account_id=b2.id,
            req=SourceTraderAssignRequest(
                wallet_address=traders[t_idx],
                custom_name=f"B2-T{t_idx}",
                fixed_amount_usd=Decimal(amount),
            ),
        )

    router = AccountRouterIndex()
    total_loaded = router.reload_from_db(db_session)
    assert total_loaded == 11
    assert router.monitored_wallet_count == 6

    # T1 must route to A1 ($15), A3 ($75), and B1 ($110)
    t1_targets = router.match_targets(traders[1], side="BUY")
    assert len(t1_targets) == 3
    by_acct = {t.trading_account_id: t.fixed_amount_usd for t in t1_targets}
    assert by_acct == {
        a1.id: Decimal("15.000000"),
        a3.id: Decimal("75.000000"),
        b1.id: Decimal("110.000000"),
    }

    # Source trader detail view for User A on T1 shows ONLY User A's accounts (A1, A3), never B1
    details_a = st_svc.get_source_trader_details_for_user(
        db_session, user_id=user_a.id, wallet_address=traders[1]
    )
    copying_ids_a = [c["trading_account_id"] for c in details_a["copying_destination_accounts"]]
    assert sorted(copying_ids_a) == sorted([a1.id, a3.id])
    assert b1.id not in copying_ids_a

    # Disable T1 on A3 and verify router only returns A1 and B1
    t1_id = details_a["source_trader_id"]
    st_svc.update_source_trader_mapping(
        db_session,
        user_id=user_a.id,
        account_id=a3.id,
        source_trader_id=t1_id,
        req=SourceTraderMappingUpdateRequest(is_enabled=False),
    )
    router.reload_from_db(db_session)
    t1_after_disable = router.match_targets(traders[1], side="BUY")
    assert sorted(t.trading_account_id for t in t1_after_disable) == sorted([a1.id, b1.id])

    # Export CSV from A1 and import into A2
    csv_data = st_svc.export_source_wallets_csv(db_session, user_id=user_a.id, account_id=a1.id)
    assert traders[1] in csv_data
    imported = st_svc.import_source_wallets_csv(
        db_session, user_id=user_a.id, account_id=a2.id, csv_content=csv_data
    )
    assert len(imported) == 3


def test_router_100k_relationships_scale_and_sub_millisecond_lookup():
    """Verify AccountRouterIndex efficiently supports 100,000+ account-to-source-trader relationships."""
    router = AccountRouterIndex()
    num_wallets = 2_000
    num_accounts = 50
    now = datetime.now(timezone.utc)

    wallets = [_eth_addr(50_000 + w) for w in range(num_wallets)]
    targets: list[RouteTarget] = []
    mapping_id = 1
    for w_idx, w_addr in enumerate(wallets):
        for a_idx in range(1, num_accounts + 1):
            targets.append(
                RouteTarget(
                    mapping_id=mapping_id,
                    user_id=(a_idx % 10) + 1,
                    trading_account_id=a_idx,
                    source_trader_id=w_idx + 1,
                    source_wallet=w_addr,
                    operating_mode=OperatingMode.PAPER_TRADING.value,
                    execution_mode=ExecutionMode.FIXED_AMOUNT.value,
                    fixed_amount_usd=Decimal("10.0"),
                    copy_percentage=Decimal("0.10"),
                    copy_multiplier=Decimal("1.0"),
                    max_order_size_usd=Decimal("100.0"),
                    min_order_size_usd=Decimal("1.0"),
                    max_exposure_usd=Decimal("500.0"),
                    max_slippage_pct=Decimal("0.05"),
                    copy_buy_enabled=True,
                    copy_sell_enabled=True,
                    mapping_created_at=now,
                )
            )
            mapping_id += 1

    count = router.load_bulk_targets(targets)
    assert count == 100_000
    assert router.monitored_wallet_count == 2_000
    assert router.total_mappings_count == 100_000

    t0 = time.perf_counter()
    for i in range(1_000):
        matched = router.match_targets(wallets[i % num_wallets], side="BUY")
        assert len(matched) == 50
    elapsed_ms = (time.perf_counter() - t0) * 1000.0
    # 1,000 lookups across 100,000 relationships should take well under 50ms (<0.05ms per lookup)
    assert elapsed_ms < 50.0, f"1,000 lookups took {elapsed_ms:.2f}ms"
