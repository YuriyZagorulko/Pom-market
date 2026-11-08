# Phase 9 — Critical and High-Severity Audit Fixes Documentation

**Date**: November 8, 2026  
**Scope**: Implementation and regression verification of the 8 authorized CRITICAL and HIGH findings (`S-01`, `S-02`, `S-03`, `P-01`, `P-02`, `P-03`, `E-03`, `E-01`) identified during the Phase 8 independent security and reliability audit.  
**Verification**: 22/22 automated test suites passing against PostgreSQL 16 using the non-superuser runtime role `pommarket_app`.

---

## 1. Finding `S-01` — PostgreSQL RLS Superuser Role Reset After Commit

- **Original Vulnerability**:
  The application database pool connected as PostgreSQL user `pommarket`, which is the database superuser (`rolsuper = true`, `rolbypassrls = true`) and table owner. `set_tenant_context()` issued `SET LOCAL ROLE pommarket_rls` and `set_config('app.current_user_id', ..., true)`. Because `SET LOCAL` and `set_config(..., is_local=true)` expire automatically when a transaction ends (`COMMIT` or `ROLLBACK`), any query executed on the same SQLAlchemy `Session` after `session.commit()` reverted to the superuser `pommarket` and bypassed `FORCE ROW LEVEL SECURITY`.
- **Root Cause**:
  Connecting the runtime application pool with a `SUPERUSER` role and relying on manual, one-shot transaction-local `SET LOCAL ROLE` calls without a SQLAlchemy transaction lifecycle listener (`after_begin`) or connection pool checkout reset hook.
- **Files Modified**:
  - `alembic/versions/0002_phase9_rls_and_intent.py`
  - `backend/app/config.py`
  - `backend/app/db.py`
  - `alembic/env.py`
  - `tests/conftest.py`
- **Fix Implemented**:
  1. Created a dedicated non-superuser login role `pommarket_app` (`LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS`) via Alembic migration `0002_phase9_rls_and_intent` and updated `Settings.database_url` to connect as `pommarket_app` (while keeping `Settings.migration_database_url` for DDL migrations).
  2. Stored the bound tenant context (`tenant_user_id`, `tenant_is_super_admin`) in `session.info` and registered a SQLAlchemy `@event.listens_for(Session, "after_begin")` hook (`_on_session_after_begin`) so every new transaction on that session — including transactions started after `session.commit()` or `session.rollback()` — automatically re-establishes `SET LOCAL ROLE pommarket_app` and `set_config('app.current_user_id', ...)`.
  3. Registered a connection-pool `@event.listens_for(engine, "checkout")` hook to reset `app.current_user_id = ''` and `app.is_super_admin = 'false'`, preventing cross-request or background-worker tenant context leakage on pooled connections.
- **Regression Tests**:
  - `tests/test_phase8_audit.py::test_fix_s01_rls_persists_across_commits_rollbacks_and_pooled_connections`
  - `tests/test_phase2_foundation.py::test_cross_tenant_isolation_and_postgresql_rls`
- **Test Results**: `PASSED` (verified `current_user == 'pommarket_app'`, `rolsuper == False`, `rolbypassrls == False`, and strict tenant filtering before commit, after commit, after rollback, and across pooled checkouts).
- **Remaining Risks**:
  - Background tasks that intentionally aggregate across all tenants must explicitly opt in via `tenant_session(user_id=None, is_super_admin=True)`.

---

## 2. Finding `S-02` — Revoked Access Tokens Remain Valid After Logout

- **Original Vulnerability**:
  Calling `POST /api/v1/auth/logout` or completing a password reset revoked the `UserSession` row in PostgreSQL, but `get_current_user()` only verified the JWT cryptographic signature and `user.is_suspended` without checking whether the session ID (`payload["sid"]`) had been revoked or expired.
- **Root Cause**:
  Missing database lookup of `UserSession(id=payload["sid"])` inside `get_current_user()` (`backend/app/main.py`).
- **Files Modified**:
  - `backend/app/main.py`
  - `backend/app/services/auth_service.py`
- **Fix Implemented**:
  1. Updated `get_current_user()` in `backend/app/main.py` to require the `"sid"` claim in every access token, load the corresponding `UserSession` row, and reject the request with `HTTP 401 Unauthorized` if the session is missing, belongs to a different user, has `is_revoked == True`, or has passed `expires_at`.
  2. Updated `AuthService.logout()` and the `/api/v1/auth/logout` endpoint to immediately revoke the active `session_id` from the caller's JWT even if no `refresh_token` body is supplied.
- **Regression Tests**:
  - `tests/test_phase8_audit.py::test_fix_s02_access_token_revoked_immediately_on_logout_and_password_reset`
- **Test Results**: `PASSED` (verified `GET /api/v1/auth/me` immediately returns `401 Unauthorized` after `POST /api/v1/auth/logout` and after `POST /api/v1/auth/password-reset/confirm`).
- **Remaining Risks**:
  - Validating `UserSession` per request adds one indexed primary-key lookup (`SELECT ... FROM user_sessions WHERE id = :sid`); if ultra-high API read throughput is needed in the future, revoked session IDs can also be cached in Redis with a short TTL.

---

## 3. Finding `S-03` — Password Reset Token Leakage in Development Configuration

- **Original Vulnerability**:
  `POST /api/v1/auth/password-reset/initiate` returned `reset_token_preview` in the JSON HTTP response whenever `ENVIRONMENT != "production"`, and `POST /api/v1/auth/register` returned `verification_token_preview` in the HTTP response. Because `ENVIRONMENT` defaulted to `"development"`, an unauthenticated caller could request a password reset for any user and read the raw reset token directly from the HTTP response.
- **Root Cause**:
  Inlining out-of-band verification/reset tokens into public REST responses instead of routing them through an email delivery sink.
- **Files Modified**:
  - `backend/app/services/email_service.py` (new out-of-band mailbox sink `DEV_MAILBOX`)
  - `backend/app/services/auth_service.py`
  - `backend/app/main.py`
- **Fix Implemented**:
  1. Removed `reset_token_preview` from `/api/v1/auth/password-reset/initiate` and stopped populating `verification_token_preview` in `AuthService.register_user` across all environments (`development`, `staging`, `production`, or unset).
  2. Created `DevMailboxSink` (`backend/app/services/email_service.py`) to capture outbound verification and password-reset tokens out-of-band in memory for local development and automated test assertions without ever exposing them in HTTP responses or logs.
- **Regression Tests**:
  - `tests/test_phase8_audit.py::test_fix_s03_password_reset_token_never_leaked_in_any_environment`
- **Test Results**: `PASSED` (verified across `ENVIRONMENT=""`, `"development"`, `"staging"`, and `"production"`).
- **Remaining Risks**: None.

---

## 4. Finding `P-01` — Retired Polymarket Data API v1 & Incompatible v2 Parsing

- **Original Vulnerability**:
  Live probes confirmed Polymarket retired Data API v1 (`/activity` -> `HTTP 410 Gone`) in favor of Data API v2 (`/v2/activity`), which returns `{"data": [...], "pagination": {"has_more": ..., "next_cursor": ...}}` with `snake_case` field names (`proxy_wallet`, `transaction_hash`, `condition_id`, `token_id`). `MarketDataCollector` only checked `camelCase` keys (`proxyWallet`, `transactionHash`, `conditionId`, `asset`) and dropped 100% of `/v2/activity` rows.
- **Root Cause**:
  Relying on legacy v1 `camelCase` field names and an unwrapped array response structure.
- **Files Modified**:
  - `backend/app/services/collector_service.py`
- **Fix Implemented**:
  1. Added `MarketDataCollector.parse_activity_v2_response()` and updated `parse_activity_poll_rows()` to unwrap the `/v2/activity` `{"data": [...], "pagination": {...}}` envelope as well as direct row lists.
  2. Updated `_normalize_row()` to extract `snake_case` fields (`proxy_wallet`, `transaction_hash`, `condition_id`, `token_id`) alongside RTDS `camelCase` fields, with explicit validation of wallet addresses and token IDs.
  3. Added `fetch_wallet_activity_v2()` supporting cursor pagination (`next_cursor` / `has_more`), `HTTP 429` (`Retry-After`) backoff, and transient `5xx` / timeout retries.
- **Regression Tests**:
  - `tests/test_phase8_audit.py::test_fix_p01_data_api_v2_envelope_pagination_retry_and_cross_feed_dedup`
- **Test Results**: `PASSED` (verified multi-page v2 parsing, `REDEEM` filtering, `HTTP 429` retry, and canonical deduplication key equivalence with RTDS `camelCase` frames).
- **Remaining Risks**:
  - Polymarket may evolve `/v2/activity` fields in future releases; schema validation errors should be monitored via `events_received_total` vs. `events_normalized_total` metrics.

---

## 5. Finding `P-02` — Incorrect Hardcoded EIP-712 Signature Recovery ID (`v = 27`)

- **Original Vulnerability**:
  `build_and_sign_v2_order()` (`backend/app/services/polymarket_signer.py`) hardcoded `v = 27` on every 65-byte EIP-712 signature. Whenever the secp256k1 ephemeral point $R.y$ had odd parity (`recovery_id = 1`, requiring `v = 28`), on-chain/CLOB `ecrecover` recovered the wrong signer address (~50% of all signed orders).
- **Root Cause**:
  Appending a constant byte `27` instead of recovering the candidate public keys for `recovery_id in (0, 1)` and matching against the signer's public key / Ethereum address.
- **Files Modified**:
  - `backend/app/services/polymarket_signer.py`
- **Fix Implemented**:
  1. Added `ethereum_address_from_verifying_key()` and `recover_eip712_signer_address()` using `ecdsa.VerifyingKey.from_public_key_recovery_with_digest`.
  2. Verified that the supplied `private_key_hex` derives to `signer_address` prior to signing.
  3. Computed the true recovery byte `v = 27 + rec_id` (`27` or `28`) that recovers `signer_address`, and self-verified `recover_eip712_signer_address(signing_digest, sig_hex) == signer_address` before returning `SignedPolymarketOrder`.
- **Regression Tests**:
  - `tests/test_phase8_audit.py::test_fix_p02_eip712_recovery_id_handles_both_v27_and_v28_and_verifies_signer`
  - `tests/test_phase6_live_execution.py::test_eip712_v2_order_signing_and_l2_hmac_vectors`
- **Test Results**: `PASSED` (verified 100% recovery accuracy across 25 deterministic salts covering both `v = 27` and `v = 28`, plus negative tests for mismatched signer addresses).
- **Remaining Risks**:
  - `POLY_1271` smart-contract signatures (if used by institutional contract wallets) require EIP-1271 contract-specific signature wrapping beyond standard EOA/Safe `ecrecover`.

---

## 6. Finding `P-03` — Polymarket `makerAmount` Precision Violation

- **Original Vulnerability**:
  Polymarket CLOB V2 requires USD notional amounts (`BUY makerAmount` or `SELL takerAmount`) to have at most 2 decimal places (`$0.01` cent grid, i.e., a multiple of `10,000` in 6-decimal base units) and share quantities to have at most 5 decimal places. `snap_shares_to_cent_grid(shares, limit_price)` previously ignored `limit_price` and simply rounded shares to `0.01`, producing fractional-cent USD notionals (e.g. `10.11 * $0.48 = $4.8528`).
- **Root Cause**:
  Rounding share size independently of the limit price's decimal denominator.
- **Files Modified**:
  - `backend/app/services/copy_engine.py`
  - `backend/app/services/polymarket_signer.py`
- **Fix Implemented**:
  1. Implemented `cent_grid_share_step(limit_price)` in `backend/app/services/copy_engine.py` using `math.gcd(mantissa, 10**(scale + 5 - 2))` so that `snap_shares_to_cent_grid(shares, limit_price)` always snaps shares to an exact step whose product `shares * limit_price` has $\le 2$ decimal places.
  2. Added `validate_polymarket_order_amounts()` in `backend/app/services/polymarket_signer.py` and enforced it inside `build_and_sign_v2_order()` before signing any order.
- **Regression Tests**:
  - `tests/test_phase8_audit.py::test_fix_p03_cent_grid_notional_precision_across_prices_and_signing_validation`
- **Test Results**: `PASSED` (verified exact cent-grid multiples across 2-digit and 3-digit tick prices `0.48`, `0.021`, `0.625`, `0.33`, `0.50`, and verified rejection of un-snapped orders).
- **Remaining Risks**: None.

---

## 7. Finding `E-03` — Duplicate Live Order Submission After Worker Crash

- **Original Vulnerability**:
  `CopyTradingEngine._dispatch_and_book_order` created the `Order` row in memory (`session.flush()`) without committing the transaction before calling `submit_signed_order()`. If the worker process crashed immediately after the HTTP `POST /order` reached Polymarket, the uncommitted database transaction rolled back (`ExecutionJob.status` reverted to `QUEUED` with zero `Order` rows), causing `recover_unfinished_jobs_after_restart()` to submit a **duplicate live order** upon restart.
- **Root Cause**:
  Performing an external side-effecting HTTP `POST /order` inside an uncommitted database transaction without first committing a durable execution intent.
- **Files Modified**:
  - `alembic/versions/0002_phase9_rls_and_intent.py`
  - `backend/app/models.py`
  - `backend/app/services/copy_engine.py`
  - `backend/app/services/live_execution_worker.py`
  - `backend/app/services/reconciliation_service.py`
- **Fix Implemented**:
  1. Added `client_order_id`, `eip712_order_hash`, and `signed_payload_json` columns to `orders` and expanded `OrderStatus` to model the full lifecycle (`PREPARED`, `SUBMITTING`, `ACKNOWLEDGED`, `PARTIAL`, `FILLED`, `AMBIGUOUS`, `CANCELLED`, `REJECTED`).
  2. In `LiveExecutionWorkerPool.execute_live_order()`, the deterministic EIP-712 order payload and hash are built first, persisted in `PREPARED` state (`session.commit()`), and then transitioned to `SUBMITTING` and **durably committed (`session.commit()`) before sending `POST /order`**.
  3. On worker restart, `ReconciliationService.recover_unfinished_jobs_after_restart()` first runs `reconcile_ambiguous_orders()` for all orders in `SUBMITTING`, `ACKNOWLEDGED`, `AMBIGUOUS`, or `SUBMITTED` state against Polymarket, and **never re-submits** any order that had already entered `SUBMITTING`.
  4. Added an idempotency guard on `CopyTradingEngine.apply_confirmed_fill()` (`select(Fill).where(Fill.order_id == order.id)`) so repeated reconciliation passes can never double-book a fill.
- **Regression Tests**:
  - `tests/test_phase8_audit.py::test_fix_e03_crash_safe_order_intent_prevents_duplicate_live_submission`
- **Test Results**: `PASSED` (verified simulated process crash immediately after Polymarket accepts `POST /order` leaves `Order(status='SUBMITTING')` committed in PostgreSQL, and restart recovery reconciles the fill with `gateway.post_count == 1`).
- **Remaining Risks**:
  - If the worker crashes in the microsecond window between `session.commit()` (`status = SUBMITTING`) and the TCP socket write of `POST /order`, reconciliation will see `NOT_FOUND` on Polymarket and safely mark the order `REJECTED` (preferring at-most-once execution over risking a double-buy).

---

## 8. Finding `E-01` — Incorrect `source_observed_shares` Accounting for Skipped BUY Orders

- **Original Vulnerability**:
  `CopyTradingEngine._handle_buy()` incremented `vpos.source_observed_shares` at the very start of `_handle_buy()` before checking slippage, market liquidity, exposure caps, or account balance. When a source trader's BUY was skipped (e.g., a 900-share BUY skipped due to `slippage_exceeded` after an initial 100-share BUY was copied), `source_observed_shares` became `1,000`, causing a subsequent 100-share SELL by the source trader to only sell `10%` of the destination account's position instead of `100%`.
- **Root Cause**:
  Mutating `vpos.source_observed_shares` before order execution succeeded.
- **Files Modified**:
  - `backend/app/services/copy_engine.py`
- **Fix Implemented**:
  1. Removed the premature `vpos.source_observed_shares += source_event.size` mutation from `_handle_buy()`.
  2. Moved `source_observed_shares` accounting into `apply_confirmed_fill()` so `source_observed_shares` is only incremented when a BUY order actually fills (scaled proportionally by `filled_shares / requested_size_shares` on partial fills) and reduced proportionally when a SELL order fills.
- **Regression Tests**:
  - `tests/test_phase8_audit.py::test_fix_e01_skipped_buy_does_not_inflate_source_observed_shares`
  - `tests/test_phase5_paper_trading.py::test_virtual_position_source_attribution_and_partial_exits`
- **Test Results**: `PASSED` (verified 100-share copied BUY + 900-share skipped BUY + 100-share SELL closes 100% of the 40-share copied position, leaving `open_shares == 0`).
- **Remaining Risks**: None.
