# Pom-Market: Multi-Tenant Polymarket Copy-Trading Platform

A self-hosted, production-oriented, multi-tenant Polymarket copy-trading platform supporting multiple registered users, multiple independent Polymarket trading accounts per user (`EOA`, `POLY_PROXY`, `POLY_GNOSIS_SAFE`, `POLY_1271`), 2,000+ monitored source trader wallets, 100,000+ account-to-source-trader mappings, source-attributed virtual positions, and three safety operating modes (`MONITORING` default, `PAPER_TRADING`, and `LIVE_TRADING`).

---

## 1. Architecture Overview

| Service / Component | Responsibility | Location |
|---|---|---|
| **API Backend (FastAPI)** | Multi-user Auth (Argon2id + JWT + TOTP 2FA), RBAC (`USER`, `SUPER_ADMIN`), Tenant Isolation, Account & Source Trader CRUD, Dashboard & Admin REST + WebSocket APIs | `backend/app/main.py` |
| **Credential Vault** | AES-256-GCM authenticated encryption at rest with HKDF-SHA256 per-tenant/per-account AAD binding and key rotation | `backend/app/security.py` |
| **Market Data Collector** | Normalizes public trades from Polymarket RTDS WebSocket (`wss://ws-live-data.polymarket.com`), Data API (`/v2/activity`), and Polygon CTF Exchange V2 `OrderFilled` logs | `backend/app/services/collector_service.py`, `rust-engine/src/lib.rs` |
| **Event Processor & Account Router** | Deduplicates on `(tx_hash, source_wallet, token_id, side)`, prevents retroactive copying of historical trades, and routes in $O(1)$ across 100,000+ mappings | `backend/app/services/event_processor.py`, `backend/app/services/account_router.py` |
| **Copy-Trading Engine** | Implements 4 sizing modes (`FIXED_AMOUNT`, `FIXED_PERCENTAGE`, `PROPORTIONAL_BALANCE`, `MAX_ORDER_SIZE`), source-attributed `virtual_positions`, book-walk simulation, and 10+ risk gates | `backend/app/services/copy_engine.py` |
| **Live Execution Workers** | Per-account isolated signing (EIP-712 CTF Exchange V2 + L2 URL-safe Base64 HMAC), bounded queues, per-account & global rate limiters, circuit breakers, and ambiguous HTTP outcome protection | `backend/app/services/live_execution_worker.py`, `backend/app/services/polymarket_signer.py` |
| **Reconciliation Service** | Resolves `AMBIGUOUS` / timed-out orders against Polymarket before any retry, recovers unfinished jobs after restarts, and detects position divergences | `backend/app/services/reconciliation_service.py` |
| **Web Dashboard (Next.js + TS)** | User Dashboard, Trading Account Page, Source Trader Page, and `SUPER_ADMIN` Operations Console | `frontend/src/app/` |

Full Phase 1 audit findings (including the BUSL-1.1 license analysis of `garnet-polymarket`) are documented in [`docs/PHASE1_AUDIT_AND_ARCHITECTURE.md`](docs/PHASE1_AUDIT_AND_ARCHITECTURE.md), and the Phase 8 independent security & reliability audit is in [`docs/PHASE8_SECURITY_AND_RELIABILITY_AUDIT.md`](docs/PHASE8_SECURITY_AND_RELIABILITY_AUDIT.md).

---

## 2. Local Development

To prevent port collisions with other locally running projects, all host-facing development ports use dedicated non-standard assignments while internal Docker container-to-container ports remain standard.

### 2.1 Development Port Allocation Table

| Service | Environment Variable | Host Port | Container Port | Internal Service DNS |
|---|---|---:|---:|---|
| **Next.js Frontend** | `FRONTEND_PORT` | `4319` | `3000` | `web-dashboard:3000` |
| **FastAPI Backend** | `BACKEND_PORT` | `4320` | `8000` | `api-backend:8000` |
| **Rust Engine** | `RUST_ENGINE_PORT` | `4321` | `8080` | `rust-engine:8080` |
| **PostgreSQL 16** | `POSTGRES_PORT` | `5440` | `5432` | `postgres:5432` |
| **Redis 7** | `REDIS_PORT` | `6390` | `6379` | `redis:6379` |
| **NATS Client** | `NATS_PORT` | `4225` | `4222` | `nats:4222` |
| **NATS Monitoring** | `NATS_MONITOR_PORT` | `8225` | `8222` | `nats:8222` |
| **Prometheus** | `PROMETHEUS_PORT` | `9095` | `9090` | `prometheus:9090` |
| **Grafana** | `GRAFANA_PORT` | `3015` | `3000` | `grafana:3000` |

### 2.2 Local Development Service URLs

- **Web Dashboard (Frontend)**: `http://localhost:4319`
- **Backend REST API**: `http://localhost:4320`
- **Backend Swagger / OpenAPI Docs**: `http://localhost:4320/docs`
- **Backend Real-Time WebSocket**: `ws://localhost:4320/ws/activity`
- **Rust Engine Health / Metrics**: `http://localhost:4321/health` and `http://localhost:4321/metrics`
- **Prometheus UI**: `http://localhost:9095`
- **Grafana UI**: `http://localhost:3015`
- **NATS Monitoring HTTP**: `http://localhost:8225/varz`
- **PostgreSQL Host Connection**: `postgresql://pommarket:pommarket_dev_pass@localhost:5440/pommarket`
- **Redis Host Connection**: `redis://localhost:6390/0`

### 2.3 Environment Setup

```bash
cp .env.example .env
# Optional: customize JWT_SECRET_KEY and MASTER_ENCRYPTION_KEY in .env
```

### 2.4 Startup Instructions

#### Option A: Start Full Stack with Docker Compose
```bash
docker compose up -d --build
```

#### Option B: Run Infrastructure in Docker + Backend/Frontend Locally on Host
```bash
# 1. Start PostgreSQL (5440), Redis (6390), and NATS (4225/8225)
docker compose up -d postgres redis nats

# 2. Apply Alembic migrations against localhost:5440
alembic upgrade head

# 3. Start FastAPI Backend on port 4320
uvicorn backend.app.main:app --reload --host 0.0.0.0 --port 4320

# 4. Start Next.js Frontend on port 4319
cd frontend && npm install && PORT=4319 npm run dev
```

### 2.5 Running the Automated Test Suite
```bash
pytest -v
```

### 2.6 Shutdown Instructions
```bash
# Stop containers while preserving PostgreSQL volume data
docker compose down

# Stop containers and remove volumes (clean reset)
docker compose down -v
```

---

## 3. Backup and Point-in-Time Recovery

Use `deploy/scripts/backup_and_restore.sh` (defaults to `POSTGRES_PORT=5440`):
```bash
chmod +x deploy/scripts/backup_and_restore.sh
./deploy/scripts/backup_and_restore.sh backup pommarket_snapshot.dump
./deploy/scripts/backup_and_restore.sh restore pommarket_snapshot.dump
```
After any database restore or ungraceful restart, run the Reconciliation Service before re-enabling `LIVE_TRADING` so all `AMBIGUOUS` or in-flight orders are reconciled with Polymarket.
