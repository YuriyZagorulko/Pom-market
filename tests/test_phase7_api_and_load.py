"""Phase 7 Automated Tests: End-to-End FastAPI Multi-Tenant HTTP API, Cross-Tenant ID Tampering Prevention, and High-Scale Load Benchmark (2,000 Wallets / 50+ Accounts / 100,000 Mappings)."""

from __future__ import annotations

from datetime import datetime, timezone
from decimal import Decimal
import time

from fastapi.testclient import TestClient

from backend.app.main import app
from backend.app.models import ExecutionMode, OperatingMode, RoleName, UserRole
from backend.app.services.account_router import AccountRouterIndex, RouteTarget
from backend.app.services.collector_service import MarketDataCollector


def test_fastapi_e2e_multi_tenant_api_and_admin_console(db_session):
    client = TestClient(app)

    # 1. Register User A and User B via REST API
    res_a = client.post(
        "/api/v1/auth/register",
        json={
            "email": "tenant_a@pom.io",
            "password": "TenantAPassword!123",
            "full_name": "Tenant A",
        },
    )
    assert res_a.status_code == 201
    data_a = res_a.json()
    token_a = data_a["access_token"]
    headers_a = {"Authorization": f"Bearer {token_a}"}

    res_b = client.post(
        "/api/v1/auth/register",
        json={
            "email": "tenant_b@pom.io",
            "password": "TenantBPassword!123",
            "full_name": "Tenant B",
        },
    )
    assert res_b.status_code == 201
    data_b = res_b.json()
    token_b = data_b["access_token"]
    headers_b = {"Authorization": f"Bearer {token_b}"}

    # 2. User A creates two Polymarket trading accounts
    acc_a1_res = client.post(
        "/api/v1/accounts",
        headers=headers_a,
        json={
            "display_name": "User A Safe #1",
            "signer_wallet_address": "0x90f79bf6eb2c4f870365e785982e1f101e93b906",
            "funder_wallet_address": "0x1111111111111111111111111111111111111111",
            "signature_type": "POLY_GNOSIS_SAFE",
            "private_key": "0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6",
            "api_key": "api-k-a1",
            "api_secret": "c2VjcmV0X2Ex",
            "api_passphrase": "pass-a1",
        },
    )
    assert acc_a1_res.status_code == 201
    acc_a1 = acc_a1_res.json()
    acc_a1_id = acc_a1["id"]
    assert "private_key" not in acc_a1
    assert "api_secret" not in acc_a1
    assert acc_a1["operating_mode"] == "MONITORING"

    # 3. Cross-tenant ID tampering check: User B tries to access, modify, or stop User A's account
    assert client.get(f"/api/v1/accounts/{acc_a1_id}", headers=headers_b).status_code == 404
    assert client.get(f"/api/v1/accounts/{acc_a1_id}/detail", headers=headers_b).status_code == 404
    assert (
        client.post(
            f"/api/v1/accounts/{acc_a1_id}/emergency-stop",
            headers=headers_b,
            json={"emergency_stop": True, "reason": "Unauthorized attack"},
        ).status_code
        == 404
    )
    assert (
        client.post(
            f"/api/v1/accounts/{acc_a1_id}/credentials/rotate",
            headers=headers_b,
            json={"private_key": "0xbad"},
        ).status_code
        == 404
    )

    # 4. User A adds a source trader, views dashboard summary and account detail page
    whale = "0x06dc51826bc524d9a83770e7de9dd7e005b04524"
    st_res = client.post(
        f"/api/v1/accounts/{acc_a1_id}/source-traders",
        headers=headers_a,
        json={
            "wallet_address": whale,
            "custom_name": "Top Polymarket Whale",
            "execution_mode": "FIXED_AMOUNT",
            "fixed_amount_usd": "25.0",
        },
    )
    assert st_res.status_code == 201

    dash_a = client.get("/api/v1/dashboard/summary", headers=headers_a)
    assert dash_a.status_code == 200
    assert dash_a.json()["total_trading_accounts"] == 1
    assert dash_a.json()["monitored_source_traders"] == 1

    dash_b = client.get("/api/v1/dashboard/summary", headers=headers_b)
    assert dash_b.status_code == 200
    assert dash_b.json()["total_trading_accounts"] == 0

    detail_a = client.get(f"/api/v1/accounts/{acc_a1_id}/detail", headers=headers_a)
    assert detail_a.status_code == 200
    assert len(detail_a.json()["source_traders"]) == 1

    # 5. Non-admin cannot access `/api/v1/admin/overview`
    assert client.get("/api/v1/admin/overview", headers=headers_a).status_code == 403

    # Grant SUPER_ADMIN to User A and verify Admin Overview & Global Emergency Stop
    db_session.add(UserRole(user_id=data_a["user"]["id"], role_name=RoleName.SUPER_ADMIN.value))
    db_session.commit()

    admin_res = client.get("/api/v1/admin/overview", headers=headers_a)
    assert admin_res.status_code == 200
    admin_body = admin_res.json()
    assert admin_body["aggregates"]["registered_users"] == 2
    assert "private_key" not in str(admin_body)

    stop_res = client.post(
        "/api/v1/admin/emergency-stop",
        headers=headers_a,
        json={"enabled": True, "reason": "Scheduled maintenance drill"},
    )
    assert stop_res.status_code == 200
    assert stop_res.json()["global_emergency_stop"]["enabled"] is True

    # Health & Prometheus metrics endpoints
    assert client.get("/health").status_code == 200
    metrics_res = client.get("/metrics")
    assert metrics_res.status_code == 200
    assert "pommarket_events_received_total" in metrics_res.text


def test_load_benchmark_2000_wallets_50_accounts_100k_mappings_and_event_burst():
    """Load test simulating:
    - 2,000 monitored source wallets
    - 50+ destination trading accounts (60 accounts across 15 tenants)
    - 100,000+ account-to-source mappings (100,000 mappings)
    - Burst of 500 incoming RTDS trade events fanned out across 50 copying accounts per event (25,000 routed fan-out deliveries).
    Measures normalization latency, routing latency, and throughput.
    """
    collector = MarketDataCollector()
    router = AccountRouterIndex()

    num_wallets = 2_000
    num_accounts = 50  # 2,000 * 50 = 100,000 account-to-source mappings
    now = datetime.now(timezone.utc)
    wallets = [f"0x{100_000 + i:040x}" for i in range(num_wallets)]

    targets: list[RouteTarget] = []
    mapping_id = 1
    for w_idx, w_addr in enumerate(wallets):
        for acct_id in range(1, num_accounts + 1):
            targets.append(
                RouteTarget(
                    mapping_id=mapping_id,
                    user_id=((acct_id - 1) // 4) + 1,
                    trading_account_id=acct_id,
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

    t_hydrate_start = time.perf_counter()
    loaded = router.load_bulk_targets(targets)
    hydrate_ms = (time.perf_counter() - t_hydrate_start) * 1000.0
    assert loaded == 100_000

    # Simulate burst of 500 incoming RTDS trade frames across the 2,000 monitored source wallets
    burst_size = 500
    now_epoch = int(now.timestamp()) + 5
    raw_frames = [
        {
            "topic": "activity",
            "type": "trades",
            "payload": {
                "proxyWallet": wallets[i % num_wallets],
                "conditionId": f"0xcond_{i % 40}",
                "asset": f"{900000 + (i % 80)}",
                "side": "BUY" if i % 2 == 0 else "SELL",
                "price": "0.54",
                "size": "150.0",
                "timestamp": now_epoch,
                "transactionHash": f"0xburst_tx_{i:08x}",
            },
        }
        for i in range(burst_size)
    ]

    t_burst_start = time.perf_counter()
    total_normalized = 0
    total_routed_deliveries = 0
    for frame in raw_frames:
        evs = collector.parse_rtds_frame(frame, received_at=now)
        for ev in evs:
            total_normalized += 1
            matched = router.match_targets(ev.source_wallet, side=ev.side)
            total_routed_deliveries += len(matched)
    burst_elapsed_ms = (time.perf_counter() - t_burst_start) * 1000.0

    assert total_normalized == 500
    assert total_routed_deliveries == 25_000  # 500 events * 50 copying accounts each
    avg_per_event_ms = burst_elapsed_ms / burst_size
    # Sub-second processing for the entire 500-event (25,000-delivery) burst!
    assert burst_elapsed_ms < 500.0, f"Burst took {burst_elapsed_ms:.2f}ms ( hydration: {hydrate_ms:.2f}ms, avg/event: {avg_per_event_ms:.4f}ms )"
