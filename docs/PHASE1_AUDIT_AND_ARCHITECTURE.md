# Phase 1: Repository & Technical Audit and Multi-Tenant Architecture

## 1. Repository Audit: `AndreySchurko/garnet-polymarket`

### 1.1 License and Commercial Restrictions
- **License**: Business Source License 1.1 (`BUSL-1.1`), Copyright (c) 2026 Andrey Schurko.
- **Change Date & Change License**: `2030-09-25` -> Apache License 2.0.
- **Additional Use Grant**:
  > *"You may use, modify and run the Licensed Work free of charge if you are a natural person operating it for your own account and trading solely your own funds. Any other use requires a commercial license. This includes, without limitation: use by or on behalf of any company, fund, partnership, trust or other legal entity; managing, trading or advising on funds belonging to third parties; providing the Licensed Work, or any output, signal or service derived from it, to third parties for a fee or other consideration..."*
- **License Implication for Our Platform**:
  - Because our platform is a **multi-tenant copy-trading platform** supporting multiple registered users and multiple trading accounts across tenants, **incorporating or copying code from `garnet-polymarket` into our production codebase is prohibited under BUSL-1.1 without a commercial license**.
  - Therefore, **zero source code files are copied from Garnet**. Instead, we perform a clean-room implementation informed by public Polymarket API specifications and documented protocol edge cases.

### 1.2 Source Code, Dependencies, and Build Scripts Audit
- **Workspace Layout**: 15 Rust crates (`garnet-types`, `garnet-db`, `garnet-core`, `garnet-copy-engine`, `garnet-risk`, `garnet-config`, `garnet-bin`, `garnet-bus`, `garnet-redis`, `garnet-clob`, `garnet-blockchain`, `garnet-feed`, `garnet-telegram`, `garnet-dash`, `garnet-watch`).
- **Build Scripts (`build.rs`)**: None (`0` `build.rs` files found across the repository).
- **Network Communication Audit**:
  - Inspected all outbound URLs across all crates.
  - Communicates only with:
    - `https://clob.polymarket.com` (CLOB v2 REST)
    - `https://gamma-api.polymarket.com` (Gamma market metadata & fee schedules)
    - `https://data-api.polymarket.com` (`/activity` REST polling)
    - `wss://ws-live-data.polymarket.com` (RTDS WebSocket live trade stream)
    - `https://polymarket.com/api/geoblock` (preflight IP check)
    - `https://gasstation.polygon.technology/v2` and operator-configured `POLYGON_RPC_URLS`
    - `https://api.telegram.org` (Telegram operator bot)
  - No hidden telemetry, key exfiltration, or phone-home endpoints exist in Garnet.
- **Credential Handling**:
  - Garnet loads a single private key (`PRIVATE_KEY`) and single CLOB L2 API key tuple (`POLY_ADDRESS`, `POLY_API_KEY`, `POLY_API_SECRET`, `POLY_API_PASSPHRASE`) from process environment variables (`.env`).
  - Redacts `api_secret` and `passphrase` in `Debug` implementations.
  - **Limitation**: Single-tenant only. Cannot store, encrypt, rotate, or isolate credentials for 50+ independent trading accounts owned by different users.
- **Signing & Order Execution**:
  - Uses Polymarket **CTF Exchange V2** EIP-712 `Order` struct (`domain.version = "2"`, `chainId = 137`, `Order(uint256 salt,address maker,address signer,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint8 side,uint8 signatureType,uint256 timestamp,bytes32 metadata,bytes32 builder)`).
  - Supports `SignatureType`: `EOA (0)`, `POLY_PROXY (1)`, `POLY_GNOSIS_SAFE (2)`, `POLY_1271 (3)`.
  - Uses L2 HMAC-SHA256 authentication headers (`POLY_ADDRESS`, `POLY_API_KEY`, `POLY_SIGNATURE`, `POLY_TIMESTAMP`, `POLY_PASSPHRASE`) with URL-safe Base64 secret decoding and signature encoding over `{timestamp}{METHOD}{path}{body}`.
  - Uses `FAK` (Fill-and-Kill / IOC) order type for taker copy execution.
  - Distinguishes `Rejected` (safe to retry once if exchange did not accept) vs `Unknown` (accepted by exchange or timed out — **never** automatically retried without reconciliation to prevent double-buying).
- **WebSocket & Data Collection**:
  - Uses `wss://ws-live-data.polymarket.com` (RTDS) subscribed to `activity` (`trades`) and `crypto_prices` (`update` as a liveness control topic).
  - Sends no unsolicited client keepalive pings (only replies `Pong` to server `Ping`) and uses a 45-second stall watchdog.
  - Uses `/activity?user=` polling and Polygon RPC `eth_subscribe` logs (`OrderFilled` V2 topic `0xd543adfd945773f1a62f74f0ee55a5e3b9b1a28262980ba90b1a89f2ea84d8ee`) as secondary/tertiary circuits.
- **Position Accounting**:
  - Garnet keys positions by `(wallet, token_id, mode)` where `wallet` is the source trader wallet, assuming a **single global destination account**.
  - **Limitation**: Does not include `destination_account_id` or `user_id`, does not support multiple destination accounts copying the same source trader with different configurations, and only supports fixed USD stake (`stake_usd`) rather than fixed amount, fixed percentage of source trade size, and proportional balance-based sizing.

### 1.3 Component Reuse vs. Rewrite Decision Matrix

| Area / Concept | Decision | Rationale |
|---|---|---|
| **Garnet Source Code (`crates/*`)** | **DO NOT USE** | Licensed under BUSL-1.1 (non-commercial / single-individual restriction until 2030-09-25) and architecturally single-tenant. |
| **Geo-proxy (`proxy.rs` / `POLYMARKET_PROXY_URL`)** | **DO NOT IMPLEMENT** | Explicitly prohibited by project safety rules: no geo-restriction bypasses or identity circumvention. |
| **Polymarket Protocol Facts (CLOB v2, RTDS, Gamma, CTF V2 EIP-712)** | **CLEAN-ROOM REIMPLEMENTATION** | Public exchange protocol specifications (EIP-712 struct layout, L2 HMAC format, cent-grid size snapping, `size_matched` verification, `unknown` vs `rejected` state machine, ascending ask / descending bid sorting). |
| **Multi-Tenant Data Model & RBAC/RLS** | **WRITE FROM SCRATCH** | Must enforce `user_id` ownership across all 17+ tables, PostgreSQL Row-Level Security (RLS) + centralized ownership guards, and 100,000+ `account_source_traders` mappings. |
| **Virtual Position Attribution** | **WRITE FROM SCRATCH** | Keyed by `(trading_account_id, source_wallet, token_id, operating_mode)` so that if Trader A and Trader B both buy Token X for Account 1, Trader A selling only sells A's attributed lot. |
| **Execution Workers & Credential Vault** | **WRITE FROM SCRATCH** | AES-256-GCM authenticated encryption at rest with HKDF per-account key derivation from an external master key (`MASTER_ENCRYPTION_KEY`), isolated per-account queues, rate limiters, and circuit breakers. |

---

## 2. Official Polymarket Data & Trading Interfaces

### 2.1 Public Data Feeds (Wallet-Attributed Trades vs. Price Updates)
1. **Primary Low-Latency Stream — Polymarket RTDS WebSocket (`wss://ws-live-data.polymarket.com`)**:
   - Subscription topic `{"topic": "activity", "type": "trades"}` delivers real-time trade payloads containing `proxyWallet` (source trader address), `asset` (`token_id`), `conditionId`, `side` (`BUY` / `SELL` / `MERGE`), `price`, `size`, `timestamp`, and `transactionHash`.
   - Note: Market price channels (`clob` price/book channels) do **not** include wallet attribution (`proxyWallet`) and must not be confused with source-wallet trade feeds.
2. **Independent Fallback / Reconciliation Polling — Polymarket Data API (`https://data-api.polymarket.com/activity?user=<wallet>`)**:
   - Returns wallet-attributed `TRADE`, `MERGE`, `SPLIT`, and `REDEEM` events.
   - Crucial rule: Only process events with `source_timestamp >= mapping_created_at` to prevent retroactively copying historical trades when a source wallet is first added.
3. **Optional On-Chain Verification — Polygon CTF Exchange V2 Logs**:
   - Contract addresses: `0xE111180000d2663C0091e4f400237545B87B996B` (CTF Exchange V2) and `0xe2222d279d744050d28e00520010520000310F59` (Neg-Risk CTF Exchange V2).
   - Event `OrderFilled` (`0xd543adfd945773f1a62f74f0ee55a5e3b9b1a28262980ba90b1a89f2ea84d8ee`), where `topics[2]` is the maker address. Confirmed after block inclusion (higher reliability, higher latency than RTDS).
4. **Order Book & Market Metadata**:
   - `GET https://clob.polymarket.com/book?token_id=<id>`: Returns `bids` and `asks` (must sort asks ascending by price, bids descending by price), `tick_size`, and `min_order_size`.
   - `GET https://clob.polymarket.com/markets/<condition_id>`: Returns `tokens[]` (`token_id`, `outcome`, `winner`), `closed`, `neg_risk`.
   - `GET https://gamma-api.polymarket.com/markets?clob_token_ids=<id>&closed=true`: Returns `conditionId` and `feeSchedule` (`rate`, `exponent`, `takerOnly`). Taker fee formula: $\text{fee} = \text{rate} \times (p \times (1 - p))^{\text{exponent}} \times \text{size}$.

### 2.2 Supported Wallet & Signature Models
Each Polymarket trading account supports:
- **`EOA` (`signature_type = 0`)**: Direct externally owned account. `maker == signer_address`, `funder_address` is the EOA.
- **`POLY_PROXY` (`signature_type = 1`)**: Magic Link / email proxy wallet. `maker == funder_address` (proxy wallet), `signer == signer_address` (EOA signing key).
- **`POLY_GNOSIS_SAFE` (`signature_type = 2`)**: MetaMask / browser wallet Gnosis Safe proxy. `maker == funder_address` (Safe contract), `signer == signer_address` (owner EOA).
- **`POLY_1271` (`signature_type = 3`)**: EIP-1271 smart contract wallet signature.

---

## 3. Multi-Tenant Platform Architecture

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                        Next.js + TypeScript Web Dashboard                    │
│     (User Dashboard, Trading Accounts, Source Traders, Live Feed, Admin)     │
└──────────────────────────────────────┬───────────────────────────────────────┘
                                       │ HTTPS / REST + WebSocket
                                       ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                     API Backend & Core Services (FastAPI / Rust)             │
│  ┌────────────────────┐  ┌─────────────────────┐  ┌───────────────────────┐  │
│  │ Auth & RBAC/TOTP   │  │ Tenant Isolation &  │  │ AES-256-GCM Vault     │  │
│  │ (Argon2id + JWT)   │  │ PostgreSQL RLS      │  │ (External Master Key) │  │
│  └────────────────────┘  └─────────────────────┘  └───────────────────────┘  │
└───────────┬──────────────────────────┬───────────────────────────┬───────────┘
            │                          │                           │
            ▼                          ▼                           ▼
┌───────────────────────┐  ┌───────────────────────┐  ┌────────────────────────┐
│ Market Data Collector │  │ Event Processor &     │  │ Independent Account    │
│ - RTDS WebSocket      │─▶│ O(1) Account Router   │─▶│ Execution Workers      │
│ - /activity Fallback  │  │ - Validate & Normalize│  │ - MONITORING mode      │
│ - Reconnect & Backoff │  │ - Dedup (tx,w,tok,sd) │  │ - PAPER_TRADING mode   │
│ - 2,000+ Wallets      │  │ - 100k+ Mappings      │  │ - LIVE_TRADING mode    │
└───────────────────────┘  └───────────────────────┘  └───────────┬────────────┘
                                                                  │
                                                                  ▼
                                                      ┌────────────────────────┐
                                                      │ Reconciliation Service │
                                                      │ - Ambiguous HTTP sync  │
                                                      │ - Order/Fill/Balance   │
                                                      │ - Recovery on restart  │
                                                      └────────────────────────┘
```

### 3.1 Seven Stage Latency Telemetry
Every event and execution tracks timestamps across all 7 required stages:
1. `source_event_ts`: Timestamp when the trade occurred on Polymarket.
2. `event_received_ts`: Timestamp when the Market Data Collector received the raw frame.
3. `event_normalized_ts`: Timestamp when the Event Processing Service validated and normalized the event.
4. `event_routed_ts`: Timestamp when the Account Router matched destination accounts and created execution jobs.
5. `order_submitted_ts`: Timestamp when the Execution Worker dispatched the order.
6. `order_ack_ts`: Timestamp when Polymarket (or simulator) acknowledged the order.
7. `fill_ts`: Timestamp when the fill was confirmed and booked to `fills` and `virtual_positions`.

### 3.2 Technical Risks and Mitigations
1. **Ambiguous Order Outcomes (HTTP Timeouts / 5xx)**:
   - *Risk*: Retrying an order after an HTTP timeout can double-buy if the exchange accepted the first request.
   - *Mitigation*: Orders that time out after dispatch enter `AMBIGUOUS` / `UNKNOWN` status. The worker refuses to retry until the Reconciliation Service queries `GET /data/order/{id}` and `GET /data/trades` to resolve actual state.
2. **Shared Source Trader Attribution**:
   - *Risk*: Aggregating positions by `(account_id, token_id)` causes Trader A's sell to liquidate Trader B's tokens.
   - *Mitigation*: `virtual_positions` is strictly keyed by `(trading_account_id, source_trader_id, token_id, operating_mode)`. A SELL signal from `source_trader_id` only calculates exit size against that specific virtual lot and never sells more than `open_size`.
3. **API Throughput Across 50+ Accounts**:
   - *Risk*: 50 accounts copying the same popular whale simultaneously can trigger Cloudflare/CLOB IP rate limits.
   - *Mitigation*: Two-tier rate limiting: per-account token bucket AND global egress token bucket with bounded queues and backpressure.
