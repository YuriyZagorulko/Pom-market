# Phase 8 — Independent Security, Integration, and Reliability Audit Report

**Audit Date**: November 8, 2026  
**Target Repository**: `/home/yuri/Desktop/my/projects/Pom-market`  
**Audit Scope**: Complete verification of Phases 1–7 implementation, security controls, Polymarket protocol integration (verified against live Polymarket endpoints), execution correctness, performance/load behavior, and architecture.  
**Audit Verification Suite**: `tests/test_phase8_audit.py` (11 new adversarial, protocol, security, and performance tests; 23/23 total repository tests passing).

---

## 1. Executive Summary

An independent, code-level and live-endpoint audit of the **Pom-Market Multi-Tenant Polymarket Copy-Trading Platform** was conducted without relying on prior implementation reports.

### Overall Verdict: **NOT READY FOR LIVE PRODUCTION TRADING (4 CRITICAL, 6 HIGH, 4 MEDIUM, 2 LOW Findings)**

While the foundational PostgreSQL data model (17 tables + Alembic migration), Argon2id password hashing, TOTP 2FA, AES-256-GCM credential encryption with tenant/account AAD binding, source-attributed virtual positions (`(trading_account_id, source_wallet, token_id, operating_mode)`), and FastAPI tenant ownership guards work in happy-path and unit/integration tests, **critical gaps exist between the claimed architecture and the actual runtime implementation**:

1. **CRITICAL (`A-01`) — Rust Engine Is a Non-Integrated Stub**: `rust-engine/src/main.rs` parses a single hardcoded JSON string and exits immediately (`Ok(())`). It has no WebSocket loop, no database connection, no NATS/Redis connection, and zero integration with the Python backend. Furthermore, no background collector or execution daemon task runs in the Python FastAPI process (`backend/app/main.py`) either.
2. **CRITICAL (`P-01`) — Polymarket Data API v1 Retired (`HTTP 410 Gone`) & v2 Schema Incompatibility**: Live probes against `https://data-api.polymarket.com/activity` confirmed that Polymarket retired Data API v1 (`HTTP 410 Gone`) in favor of `/v2/activity`. `/v2/activity` wraps payloads in `{"data": [...], "pagination": {...}}` and uses `snake_case` field names (`proxy_wallet`, `transaction_hash`, `condition_id`, `token_id`). `MarketDataCollector.parse_activity_poll_rows` (`backend/app/services/collector_service.py`) only checks `camelCase` keys (`proxyWallet`, `transactionHash`) and **silently drops 100% of live Data API v2 trade rows**.
3. **CRITICAL (`P-02`) — EIP-712 Signature Hardcodes `v = 27` (~50% Live Order Signature Rejection)**: `build_and_sign_v2_order` (`backend/app/services/polymarket_signer.py:206`) hardcodes the ECDSA recovery ID byte to `27` instead of computing the actual `secp256k1` curve point $R.y$ parity (`27` or `28`). Empirical testing across 20 order salts proved that ~50% of generated signatures recover the wrong Ethereum address under `ecrecover` and will be rejected by Polymarket's CTF Exchange V2 contract.
4. **CRITICAL (`E-03`) — Worker Crash After HTTP POST Causes Duplicate Live Order Submission**: In `CopyTradingEngine._dispatch_and_book_order` (`backend/app/services/copy_engine.py:592-672`), the `Order` record and `ExecutionJob` status transition are not committed to PostgreSQL prior to calling `submit_signed_order`. If a worker crashes immediately after the HTTP `POST /order` reaches Polymarket, the database transaction rolls back (`ExecutionJob.status` remains `QUEUED` with no `Order` row), and `ReconciliationService.recover_unfinished_jobs_after_restart` submits a **second live order** on restart.

---

## 2. Feature-by-Feature Implementation & Execution Path Verification

| Feature / Subsystem | Actual Implementation Files | Real Execution Path | Automated Test Coverage | Missing Functionality & Limitations |
|---|---|---|---|---|
| **Multi-Tenant Database & Alembic** | `alembic/versions/0001_initial_multi_tenant_schema.py`, `backend/app/models.py`, `backend/app/db.py` | 17 SQLAlchemy 2.0 tables; single linear Alembic head (`0001_initial_schema`); PostgreSQL RLS policies via `pommarket_rls` role. | `tests/test_phase2_foundation.py`, `tests/test_phase8_audit.py` | DB connection user `pommarket` is a PostgreSQL superuser; RLS relies on `SET LOCAL ROLE pommarket_rls`, which resets to superuser after `session.commit()` (`S-01`). |
| **Authentication, RBAC & TOTP** | `backend/app/services/auth_service.py`, `backend/app/security.py`, `backend/app/main.py` | Argon2id (`PyNaCl`), JWT access tokens, SHA-256 refresh tokens, RFC-6238 TOTP, `USER`/`SUPER_ADMIN` roles. | `tests/test_phase2_foundation.py`, `tests/test_phase7_api_and_load.py`, `tests/test_phase8_audit.py` | `get_current_user` does not verify if `payload["sid"]` was revoked on logout (`S-02`); `/password-reset/initiate` leaks raw reset token when `ENVIRONMENT != "production"` (`S-03`). |
| **Trading Account CRUD & AES-256-GCM Vault** | `backend/app/services/account_service.py`, `backend/app/security.py` | HKDF-SHA256 + AES-256-GCM with `AAD = tenant:{user_id}:account:{account_id}`; supports `EOA`, `POLY_PROXY`, `POLY_GNOSIS_SAFE`, `POLY_1271`. | `tests/test_phase2_foundation.py` | No on-chain RPC verification of Gnosis Safe `masterCopy()` or USDC.e allowance; version prefix `v1:` is not bound in AAD (`S-05`). |
| **Source Trader CRUD & Account Router** | `backend/app/services/source_trader_service.py`, `backend/app/services/account_router.py` | Python `AccountRouterIndex` inverted map `wallet -> tuple[RouteTarget, ...]`. | `tests/test_phase3_source_traders.py` | In-memory router is process-local; no NATS/Redis pub-sub invalidation when mappings are updated via API (`A-02`). |
| **Rust Hot-Path Engine** | `rust-engine/src/lib.rs`, `rust-engine/src/main.rs`, `rust-engine/Cargo.toml` | **Disconnected stub**. `main.rs` parses 1 static JSON string and exits. | None (`rustc`/`cargo` not installed on host; not wired to Python). | **Not integrated into event collection, routing, or execution (`A-01`)**. |
| **Market Data Collector** | `backend/app/services/collector_service.py` | Synchronous parser methods (`parse_rtds_frame`, `parse_activity_poll_rows`, `parse_polygon_v2_log`). | `tests/test_phase4_monitoring.py`, `tests/test_phase8_audit.py` | No background asyncio task connecting to `wss://ws-live-data.polymarket.com` or polling `/v2/activity`; drops `/v2/activity` `snake_case` fields (`P-01`); ignores `MERGE` leader exits (`P-04`). |
| **Copy-Trading Engine (Paper & Live)** | `backend/app/services/copy_engine.py` | Evaluates risk gates, sizes orders across 4 modes, walks order book, updates `virtual_positions` and `account_balances`. | `tests/test_phase5_paper_trading.py`, `tests/test_phase8_audit.py` | Skipped BUYs still increment `source_observed_shares` (`E-01`); multi-level ask walk can exceed `available_cash_usd` (`E-04`); no `SELECT ... FOR UPDATE` row locking (`E-05`). |
| **Live Execution Workers & Signer** | `backend/app/services/live_execution_worker.py`, `backend/app/services/polymarket_signer.py` | Signs EIP-712 `Order` V2 + L2 HMAC headers; calls `PolymarketExchangeGateway`. | `tests/test_phase6_live_execution.py`, `tests/test_phase8_audit.py` | `PolymarketExchangeGateway` has no concrete HTTP client implementation (`P-05`); EIP-712 `v=27` hardcoded (`P-02`); no order cancellation endpoint (`P-06`). |
| **Reconciliation Service** | `backend/app/services/reconciliation_service.py` | Resolves `AMBIGUOUS`/`SUBMITTED` orders, recovers `QUEUED` jobs, compares token balances. | `tests/test_phase6_live_execution.py`, `tests/test_phase8_audit.py` | Crash before initial commit causes duplicate live order on recovery (`E-03`). |
| **Web Dashboard (Next.js)** | `frontend/src/app/page.tsx`, `accounts/[id]/page.tsx`, `traders/[wallet]/page.tsx`, `admin/page.tsx` | Next.js 14 + TypeScript pages & `PlatformApiClient` (`frontend/src/lib/api.ts`). | Manual/type structure inspection | Sub-pages (`accounts/[id]`, `traders/[wallet]`, `admin`) render static hardcoded demo data instead of calling `PlatformApiClient` (`A-04`). |

---

## 3. Actual Architecture vs. Claimed Architecture

### Actual Data Flow in Codebase
```
[Tests / Caller]
       │  (Direct Python method calls — no NATS JetStream or Redis in path)
       ▼
MarketDataCollector (Python: collector_service.py)
       │  NormalizedTradeEvent
       ▼
EventProcessingService (Python: event_processor.py)
  ├─ In-memory AccountRouterIndex (Python: account_router.py)
  └─ Synchronous PostgreSQL INSERTs (1 SourceTradeEvent + N ExecutionJobs)
       │
       ▼
CopyTradingEngine (Python: copy_engine.py)
  ├─ PAPER_TRADING: simulate_book_walk_buy / sell -> PostgreSQL (Order, Fill, VirtualPosition, AccountBalance)
  └─ LIVE_TRADING: LiveExecutionWorkerPool (Python: live_execution_worker.py)
        └─ PolymarketExchangeGateway Protocol (No concrete HTTP client in repo; mocked in tests)
```

---

## 4. Security Audit Findings

### Verified Security Controls (Working)
- **Cross-Tenant API Isolation**: Verified in `test_fastapi_e2e_multi_tenant_api_and_admin_console` (`tests/test_phase7_api_and_load.py`). User B attempting `GET /api/v1/accounts/{a_id}`, `GET /api/v1/accounts/{a_id}/detail`, `POST /api/v1/accounts/{a_id}/emergency-stop`, or `POST /api/v1/accounts/{a_id}/credentials/rotate` against User A's account receives `HTTP 404 Not Found`.
- **Argon2id Password Hashing & TOTP**: Verified in `test_user_registration_login_refresh_and_totp`. Uses `nacl.pwhash.argon2id` (`OPSLIMIT_INTERACTIVE`, `MEMLIMIT_INTERACTIVE`) and constant-time `hmac.compare_digest` for 6-digit RFC-6238 TOTP codes.
- **AES-256-GCM Credential Encryption & AAD Binding**: Verified in `test_cross_tenant_isolation_and_postgresql_rls`. Private keys and API secrets are encrypted with a 16-byte random salt, 12-byte random nonce, HKDF-SHA256 derived 256-bit key, and `AAD = tenant:{user_id}:account:{trading_account_id}`. Tampering with `user_id` or `trading_account_id` raises `PermissionError`.
- **Secret Redaction in API Responses & Logs**: Verified that `TradingAccountResponse`, `get_trading_account_page_detail`, and `get_admin_platform_overview` never serialize `encrypted_private_key`, `encrypted_api_secret`, or `encrypted_api_passphrase`.

### Discovered Security Vulnerabilities

1. **`[CRITICAL] S-01` — PostgreSQL RLS Bypass After `session.commit()` or on Unscoped Worker Sessions** (`backend/app/db.py:45-58`)
   - **Mechanism**: The database connection string connects as PostgreSQL role `pommarket`, which is a **PostgreSQL superuser** (`rolsuper = true`, `rolbypassrls = true`) and table owner. `set_tenant_context()` executes `SET LOCAL ROLE pommarket_rls` and `set_config('app.current_user_id', ..., true)`. In PostgreSQL, `SET LOCAL` and `set_config(..., is_local=true)` automatically expire at the end of the current transaction (`COMMIT` or `ROLLBACK`).
   - **Proof (`test_security_finding_rls_reverts_to_superuser_after_commit`)**: Immediately after `db_session.commit()`, `SELECT current_user` reverts from `pommarket_rls` back to `pommarket` (superuser), and `db_session.scalars(select(TradingAccount)).all()` returns **all tenants' rows**. Furthermore, background services (`EventProcessingService`, `CopyTradingEngine`, `ReconciliationService`) do not call `set_tenant_context` per tenant.

2. **`[HIGH] S-02` — Logged-Out and Password-Reset JWT Access Tokens Remain Valid Until Expiration** (`backend/app/main.py:77-98`)
   - **Mechanism**: `create_access_token` embeds `"sid": user_session.id` in the JWT payload (`backend/app/security.py:188`), and `logout()` / `confirm_password_reset()` sets `user_session.is_revoked = True`. However, `get_current_user()` in `backend/app/main.py` never queries `UserSession` to verify that `payload["sid"]` is still active.
   - **Proof (`test_security_finding_access_token_remains_active_after_logout`)**: After `POST /api/v1/auth/logout` returns `200 OK`, calling `GET /api/v1/auth/me` with the logged-out `access_token` still succeeds with `200 OK` for up to 30 minutes.

3. **`[HIGH] S-03` — Unauthenticated Password Reset & Verification Token Leakage Under Default Configuration** (`backend/app/main.py:220-229`, `backend/app/services/auth_service.py:106`)
   - **Mechanism**: `Settings.environment` defaults to `"development"` (`backend/app/config.py:12`). Whenever `ENVIRONMENT != "production"`, `POST /api/v1/auth/password-reset/initiate` returns the raw `reset_token_preview` in the JSON response to any unauthenticated caller, and `POST /api/v1/auth/register` unconditionally returns `verification_token_preview` in all environments.
   - **Proof (`test_security_finding_unauthenticated_password_reset_token_leak`)**: An unauthenticated attacker knowing only a victim's email address receives a valid password reset token in the response body if `ENVIRONMENT=production` was omitted at deployment.

4. **`[MEDIUM] S-04` — Hardcoded Fallback Cryptographic Keys in `Settings` & Permissive CORS** (`backend/app/config.py:19-35`, `backend/app/main.py:60-66`)
   - **Mechanism**: `JWT_SECRET_KEY` and `MASTER_ENCRYPTION_KEY` fall back to static hardcoded strings if environment variables are unset, without a startup guard rejecting default keys in non-test environments. Additionally, `CORSMiddleware` is configured with `allow_origins=["*"]` and `allow_credentials=True`.

---

## 5. Polymarket Protocol Integration Audit

We probed live Polymarket infrastructure (`https://clob.polymarket.com`, `https://gamma-api.polymarket.com`, `https://data-api.polymarket.com`, and `wss://ws-live-data.polymarket.com`) and compared actual wire behavior against the repository's implementation.

### Live Polymarket Endpoint Probe Summary
- `GET https://clob.polymarket.com/ok` -> `HTTP 200 "OK"`
- `GET https://clob.polymarket.com/book?token_id=323382...` -> `HTTP 200`: Confirmed `asks` arrive in **descending price order** (`0.999`, `0.998`, ..., `0.018`), `tick_size: 0.001`, `min_order_size: 5`, `neg_risk: False`. `OrderBookSnapshot.from_raw` (`backend/app/services/copy_engine.py:70`) **correctly** sorts `asks` ascending and `bids` descending.
- `WSS wss://ws-live-data.polymarket.com` -> Confirmed live `activity/trades` frames arrive with `camelCase` keys (`proxyWallet`, `conditionId`, `asset`, `transactionHash`, `side`, `price`, `size`, `timestamp`), which `MarketDataCollector.parse_rtds_frame` **correctly** parses.
- `GET https://data-api.polymarket.com/activity?user=...` -> **`HTTP 410 Gone: {"error":"Data API v1 has been retired","migration":"https://data-api.polymarket.com/v2/"}`**.

### Discovered Polymarket Protocol Discrepancies

1. **`[CRITICAL] P-01` — Data API v1 Retired (`HTTP 410`) & Data API v2 (`snake_case`) Rows Dropped** (`backend/app/config.py:45`, `backend/app/services/collector_service.py:143-163, 213-253`)
   - **Discrepancy**: Live `GET https://data-api.polymarket.com/v2/activity?user=<wallet>` returns `{"data": [...], "pagination": {...}}` where each trade uses `proxy_wallet`, `transaction_hash`, `condition_id`, and `token_id`. `MarketDataCollector._normalize_row` only checks `proxyWallet`/`wallet` and `transactionHash`/`tx_hash`.
   - **Proof (`test_polymarket_finding_data_api_v2_snake_case_payload_dropped`)**: Passing a real live `/v2/activity` trade object to `parse_activity_poll_rows` returns `[]` (100% data loss on fallback/reconciliation feed).

2. **`[CRITICAL] P-02` — EIP-712 Order Signer Hardcodes `v = 27` Recovery Byte** (`backend/app/services/polymarket_signer.py:200-207`)
   - **Discrepancy**: `build_and_sign_v2_order` signs the 32-byte EIP-712 digest using `ecdsa.SigningKey.sign_digest_deterministic(..., sigencode=sigencode_string_canonize)` (which returns 64 bytes `r || s` without the recovery ID) and appends `bytes([27])`. In Ethereum/Polygon `ecrecover`, `v` must be `27 + (R.y & 1)`.
   - **Proof (`test_polymarket_finding_eip712_hardcoded_v27_recovery_id_mismatch`)**: Tested across 20 deterministic salts; whenever $R.y$ is odd, `v = 27` recovers a completely different public key/address than `signer_address`, causing on-chain/CLOB signature verification failure on ~50% of live orders.

3. **`[HIGH] P-03` — Order Size Snapping Violates Polymarket 2-Decimal Maker Amount Grid** (`backend/app/services/copy_engine.py:121-125`)
   - **Discrepancy**: Polymarket CLOB requires the maker USD notional (`price * size` on a BUY) to have at most 2 decimal places (`$0.01` cent grid), which requires `size` to be a multiple of the price-dependent cent step (e.g. `0.0625` shares at `$0.48`, or `10` shares at `$0.021`). `snap_shares_to_cent_grid(shares, limit_price)` ignores `limit_price` and simply rounds `shares` to `0.01`.
   - **Proof (`test_polymarket_finding_cent_grid_notional_precision_violation`)**: At `price = 0.48` and `shares = 10.11`, `price * size = 4.8528` (4 decimal places), which Polymarket CLOB rejects.

4. **`[HIGH] P-05` — No Concrete HTTP/WebSocket Client Implementation for `PolymarketExchangeGateway`** (`backend/app/services/live_execution_worker.py:59-82`)
   - **Discrepancy**: `PolymarketExchangeGateway` is defined only as a `typing.Protocol`. No concrete HTTP client class (`httpx`/`aiohttp`) implementing `POST /order`, `GET /data/order/{id}`, `GET /data/trades`, or `DELETE /order` exists in `backend/app/`.

5. **`[MEDIUM] P-04` — Leader `MERGE` Exit Events Are Ignored by `MarketDataCollector`** (`backend/app/services/collector_service.py:219-220`)
   - **Discrepancy**: When a source trader exits a position by merging YES+NO tokens (`type: "MERGE"` or `side: "MERGE"`), `_normalize_row` filters `if row_type not in ("TRADE", "BUY", "SELL"): return None`, leaving destination accounts holding open positions after the source trader has exited via merge.

---

## 6. Execution Correctness Audit

All 10 required execution scenarios were tested across `tests/test_phase5_paper_trading.py`, `tests/test_phase6_live_execution.py`, and `tests/test_phase8_audit.py`:

| Scenario | Test Result | Details |
|---|---|---|
| 1. Two source traders buy the same token | **PASS** | Separate `VirtualPosition` rows created for `(account_id, trader_a, token_id, mode)` and `(account_id, trader_b, token_id, mode)`. |
| 2. One source trader sells while the other holds | **PASS** | Selling Trader A's lot leaves Trader B's 80-share lot 100% untouched. |
| 3. One source trader partially exits | **PASS** | 50% exit by Trader A sells 50% (20 of 40 shares) of Trader A's attributed lot. |
| 4. Multiple events arrive out of order | **FAIL (`E-02`)** | Verified in `test_execution_finding_out_of_order_sell_before_buy_opens_stale_position`: if SELL (`t=110`) arrives before BUY (`t=102`), the SELL is skipped and the stale BUY at `t=102` still opens a position. |
| 5. Duplicate events arrive from different data sources | **PASS** | Verified in `test_collector_normalizes_rtds_activity_and_polygon_logs_and_deduplicates`: identical `(tx_hash, source_wallet, token_id, side)` across RTDS, `/activity`, and Polygon logs produces 1 `SourceTradeEvent` and 1 `ExecutionJob`. |
| 6. Order times out (`504`) but was actually accepted | **PASS** | Verified in `test_live_execution_ambiguous_http_timeout_reconciliation_and_circuit_breaker`: marked `AMBIGUOUS` (0 retries), then resolved by `ReconciliationService`. |
| 7. Worker crashes after order submission | **FAIL (`E-03`)** | Verified in `test_execution_finding_uncommitted_order_before_crash_causes_duplicate_live_submission`: crash before DB commit rolls back `Order` row, causing `recover_unfinished_jobs_after_restart` to submit a **duplicate live order**. |
| 8. Worker restarts with unfinished `QUEUED` jobs | **PASS (if no prior POST)** | `recover_unfinished_jobs_after_restart` processes `QUEUED` jobs and skips jobs that already have an `Order` row committed. |
| 9. Destination account has insufficient balance | **PARTIAL (`E-01`, `E-04`)** | Rejects when `available_cash_usd < target_usd`, **but**: (a) still increments `vpos.source_observed_shares` before skipping (`E-01`), and (b) multi-level ask walk up to `limit_price` can spend more than `available_cash_usd`, driving cash negative (`E-04`). |
| 10. Destination account has insufficient attributable inventory | **PASS** | Caps exit shares at `min(open_shares, requested_shares)` and never sells another source trader's lot. |

---

## 7. Performance, Concurrency, and Reliability Benchmark Results

Benchmarks were executed against PostgreSQL 16 (`pommarket_test` on port `5439`) with **2,000 monitored source wallets**, **50–55 trading accounts**, and **100,000–110,000 account-to-source-trader mappings**:

| Metric | Measured Value | Target / Assessment |
|---|---|---|
| **Router Hydration (100,000 mappings)** | `90.62 ms` | Fast startup hydration |
| **Event Normalization Latency (`parse_rtds_frame`)** | `p50 = 18.49 µs`, `p95 = 21.64 µs`, `p99 = 33.68 µs` | Sub-millisecond |
| **In-Memory Routing Latency (50 accounts matched)** | `p50 = 7.29 µs`, `p95 = 9.05 µs`, `p99 = 15.67 µs` | Sub-millisecond ($O(1)$ hash lookup) |
| **500-Event Burst (25,000 routed deliveries, in-memory)** | `14.8 ms` total (`29.6 µs/event`) | Zero event loss, 100% deduplication accuracy |
| **Queue Backpressure (`max_queue_size = 256`)** | `256` accepted, `44` rejected on 300-job burst | Bounded queue enforced per account |
| **PostgreSQL Synchronous Fan-Out (1 event -> 50 `ExecutionJob`s)** | **`123.82 ms`** (`2.48 ms/job`) | **Bottleneck (`A-03`)**: 50 sequential `SELECT` + `INSERT` queries inside `process_event` |
| **PostgreSQL Sequential Execution (50 Paper Fills)** | **`1,968.13 ms`** (`39.36 ms/account`) | **Bottleneck (`A-03`)**: ~10 sequential SQL queries per account on a single thread |
| **Process Memory Usage (`ru_maxrss`) at 110,000 Mappings** | `175.97 MB` | Low memory footprint |
| **CPU Usage (5,000 frames + 250,000 routings)** | `2.314s` user CPU, `0.138s` system CPU | Efficient in-memory CPU usage |

### Sustained Operation & Concurrency Assessment
- In-memory routing and deduplication (`AccountRouterIndex` and `EventProcessingService._seen_keys` capped at 200,000 entries) maintain stable memory (`~176 MB` RSS) without unbounded growth.
- **Database Bottleneck under Burst Load (`A-03`)**: Because `EventProcessingService.process_event` synchronously executes a separate `SELECT ExecutionJob WHERE idempotency_key = ...` and `INSERT INTO execution_jobs` for each of the 50 destination accounts on the hot path (`123.82 ms` per event), a burst of 20 whale trades/second copied by 50 accounts requires `>2.4 seconds` of sequential database time per second of wall-clock time, causing queue growth unless fan-out is batched (`INSERT ... ON CONFLICT DO NOTHING`) and decoupled via NATS JetStream / async worker pools.

---

## 8. Complete Findings Table & Severity Classification

| ID | Severity | Category | Title & Location |
|---|---|---|---|
| **A-01** | **CRITICAL** | Architecture | **Rust hot-path engine (`rust-engine/src/main.rs`) is a non-integrated stub** that parses one static string and exits; no live collector or worker loop runs in either Rust or Python. |
| **P-01** | **CRITICAL** | Polymarket Protocol | **Data API v1 (`/activity`) is retired (`HTTP 410 Gone`) and Data API v2 (`/v2/activity`) `snake_case` payloads are 100% dropped** by `MarketDataCollector` (`backend/app/services/collector_service.py:222-225`). |
| **P-02** | **CRITICAL** | Polymarket Protocol | **EIP-712 order signer hardcodes `v = 27`** (`backend/app/services/polymarket_signer.py:206`), causing invalid `ecrecover` signer address on ~50% of all signed live orders. |
| **E-03** | **CRITICAL** | Execution Reliability | **Worker crash after HTTP `POST /order` before DB commit causes duplicate live order submission** on recovery (`backend/app/services/copy_engine.py:592-672`, `reconciliation_service.py:196-236`). |
| **S-01** | **CRITICAL** | Security / Multi-Tenancy | **PostgreSQL RLS role `pommarket_rls` reverts to superuser `pommarket` after `session.commit()`** (`backend/app/db.py:45-58`) and is not set in background worker sessions. |
| **S-02** | **HIGH** | Security / Auth | **Logged-out and password-reset JWT access tokens remain valid until expiry** because `get_current_user` (`backend/app/main.py:77-98`) never checks `UserSession.is_revoked`. |
| **S-03** | **HIGH** | Security / Auth | **Unauthenticated password reset endpoint leaks `reset_token_preview`** whenever `ENVIRONMENT != "production"` (`backend/app/main.py:220-229`). |
| **P-03** | **HIGH** | Polymarket Protocol | **`snap_shares_to_cent_grid` ignores `limit_price`** (`backend/app/services/copy_engine.py:121-125`), producing `makerAmount` values with >2 decimal places that Polymarket CLOB rejects. |
| **P-05** | **HIGH** | Polymarket Protocol | **`PolymarketExchangeGateway` has no concrete HTTP client implementation** (`backend/app/services/live_execution_worker.py:59-82`) and lacks order cancellation (`DELETE /order`). |
| **E-01** | **HIGH** | Execution Correctness | **Skipped BUY orders still increment `vpos.source_observed_shares`** (`backend/app/services/copy_engine.py:381-387`), corrupting subsequent SELL exit fractions. |
| **A-02** | **HIGH** | Architecture | **Redis and NATS JetStream are unused in application code**, leaving deduplication, routing caches, and rate limiters uncoordinated across processes. |
| **A-03** | **MEDIUM** | Performance / DB | **Synchronous per-target SQL queries during fan-out (`123.8 ms` / 50 jobs) and sequential account execution (`1,968 ms` / 50 accounts)** bottleneck burst throughput. |
| **E-02** | **MEDIUM** | Execution Correctness | **Out-of-order arrival (newer SELL before older BUY) opens a stale position** (`backend/app/services/copy_engine.py:380-470`). |
| **E-04** | **MEDIUM** | Execution Correctness | **Multi-level ask walk up to `limit_price` can spend more than `available_cash_usd`** when shares are sized at `best_ask` (`backend/app/services/copy_engine.py:450-473`). |
| **E-05** | **MEDIUM** | Concurrency | **Missing `SELECT ... FOR UPDATE` row locking on `AccountBalance` and `VirtualPosition`** allows concurrent workers for the same account to double-spend cash/exposure. |
| **A-04** | **LOW** | Frontend | **Next.js sub-pages (`accounts/[id]`, `traders/[wallet]`, `admin`) display static hardcoded state** instead of fetching from `PlatformApiClient`. |
| **S-04** | **LOW** | Security / Config | **Static default fallback secrets in `Settings` and `allow_origins=["*"]` with `allow_credentials=True`**. |

---

## 9. Prioritized Remediation Roadmap

1. **Priority 1 (Blockers for Live Execution & Security Isolation)**:
   - **Fix `P-02` (EIP-712 `v` Recovery ID)**: Use `signing_key.sign_digest_deterministic` with recovery-key extraction (or `eth-account` / `coincurve` / Rust `alloy::signers`) so `v` is accurately set to `27` or `28` matching `signer_address`.
   - **Fix `E-03` (Crash-Safe Two-Phase Order Dispatch)**: Persist and **commit** `ExecutionJob.status = PROCESSING` and `Order(status = SUBMITTED, idempotency_key = ...)` in a dedicated transaction **before** sending the outbound HTTP `POST /order`. If the process crashes mid-flight, recovery will see the committed `SUBMITTED` order and reconcile it via `ReconciliationService` instead of re-sending.
   - **Fix `S-01` (Database Non-Superuser Role & Session-Scoped RLS)**: Connect the application pool using a dedicated non-superuser PostgreSQL role (`NOBYPASSRLS NOSUPERUSER`) instead of the superuser table owner `pommarket`, and register a SQLAlchemy `@event.listens_for(Session, "after_begin")` hook so every transaction automatically sets `app.current_user_id`.
   - **Fix `P-01` (Polymarket Data API v2 Compatibility)**: Update `MarketDataCollector` to query `/v2/activity`, unwrap `response["data"]`, and accept both `snake_case` (`proxy_wallet`, `transaction_hash`, `condition_id`, `token_id`) and `camelCase` keys.

2. **Priority 2 (Authentication Hardening & Position Accounting Accuracy)**:
   - **Fix `S-02` & `S-03`**: Check `UserSession.is_revoked` inside `get_current_user` using `payload["sid"]`, and remove `reset_token_preview` / `verification_token_preview` from HTTP responses unless an explicit `ALLOW_INSECURE_TOKEN_PREVIEW=true` test flag is set.
   - **Fix `E-01`, `E-02`, `E-04`, `E-05`, & `P-03`**:
     - Only increment `vpos.source_observed_shares` after all risk/slippage/balance checks pass and the copy order is actually filled (`apply_confirmed_fill`).
     - Snap share size using `gcd(price_mantissa, 10**(scale + 3))` so `price * size` always lands on the `$0.01` cent grid (`P-03`), and cap `simulate_book_walk_buy` total spend (`notional + fee`) at `min(target_usd, available_cash_usd)` (`E-04`).
     - Acquire `with_for_update()` on `AccountBalance` and `VirtualPosition` during order execution (`E-05`), and track `last_source_event_ts` on `VirtualPosition` to reject out-of-order stale BUYs (`E-02`).

3. **Priority 3 (Hot-Path Architecture & Throughput Scaling)**:
   - **Fix `A-01`, `A-02`, `A-03`, & `P-05`**: Wire `rust-engine` (or an async Python collector/worker supervisor) to `wss://ws-live-data.polymarket.com` and NATS JetStream, batch-insert `ExecutionJob` rows (`INSERT INTO execution_jobs ... ON CONFLICT DO NOTHING`), implement the concrete `HttpPolymarketExchangeGateway` client, and connect the Next.js sub-pages (`A-04`) to live API endpoints.
