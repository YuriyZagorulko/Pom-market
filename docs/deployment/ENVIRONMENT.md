# Environment variables (deployment reference)

Use **`.env.production.example`** for Coolify/production and **`.env.example`** for local dev. Never commit `.env`. All files are listed in `.gitignore`.

Legend: **R** = required for production API start, **O** = optional, **S** = secret (generate, do not reuse dev values). **R†** = required when `ENVIRONMENT=production` (`scripts/deploy/validate.sh`).

## Core

| Variable | R/O | Used by | Description |
|----------|-----|---------|-------------|
| `ENVIRONMENT` | R | API, Worker | `production` enables stricter secret validation |
| `LOG_FORMAT` | O | API, Worker | `json` recommended in production |
| `RUST_LOG` | O | API, Worker | e.g. `info,sqlx=warn` |
| `CONFIRM_LIVE_TRADING` | R | API, Worker | Must stay `false` until deliberate live rollout (compose also forces false) |
| `POM_SKIP_STARTUP_MIGRATION` | O | API, Worker | Set `true` in `docker-compose.prod.yml` after one-shot `migrate` service |
| `PAPER_ALLOW_SYNTHETIC_FILLS` | O | Worker | Paper-only testing; keep `false` in production |

## PostgreSQL

| Variable | R/O | Used by | Description |
|----------|-----|---------|-------------|
| `POSTGRES_PASSWORD` | R | Postgres, compose | Superuser password (not in git) |
| `MIGRATION_DATABASE_URL` | R | migrate, API, Worker | `postgres://pommarket:…@postgres:5432/pommarket` |
| `APP_DATABASE_URL` | R | API | `pommarket_app` role |
| `WORKER_DATABASE_URL` | R | Worker | `pommarket_worker` role |
| `APP_DB_PASSWORD` | R | API, Worker | App role password (applied on startup) |
| `WORKER_DB_PASSWORD` | R | API, Worker | Worker role password |

## HTTP API

| Variable | R/O | Used by | Description |
|----------|-----|---------|-------------|
| `API_BIND` | R | API | `0.0.0.0:4321` in containers |
| `TRUSTED_PROXIES` | O | API | Comma-separated proxy CIDRs for X-Forwarded-For (empty = peer IP only) |
| `CORS_ORIGINS` | O | API | Extra origins (in addition to `USER_ORIGIN`, `ADMIN_ORIGIN`, `PUBLIC_API_ORIGIN`) |

## Public URLs (no hardcoding in source)

Production hostnames are single-level under `pomvix.com` (wildcard TLS `*.pomvix.com`).

| Variable | R/O | Used by | Description |
|----------|-----|---------|-------------|
| `USER_ORIGIN` | R | API | User dashboard, e.g. `https://pommarket.pomvix.com` |
| `APP_BASE_URL` | R | API | Invitation links; usually same as `USER_ORIGIN` |
| `ADMIN_ORIGIN` | R | API | Admin console, e.g. `https://pommarket-admin.pomvix.com` |
| `PUBLIC_API_ORIGIN` | R† | API | Public API, e.g. `https://pommarket-api.pomvix.com` (included in CORS) |
| `API_INTERNAL_URL` | R | Admin, User (build/runtime) | `http://api:4321` inside Docker — do not use public API hostname here |

## Auth & encryption

| Variable | R/O | Used by | Description |
|----------|-----|---------|-------------|
| `JWT_SECRET` | S | API | Session signing (≥32 chars in production) |
| `ACCESS_TTL_SECONDS` | O | API | Default 900 |
| `REFRESH_TTL_SECONDS` | O | API | Default 1209600 |
| `ENCRYPTION_MASTER_KEY` | S | API, Worker | Credential vault |
| `ENCRYPTION_KEY_ID` | O | API, Worker | e.g. `k1` |
| `ENCRYPTION_MASTER_KEY_PREVIOUS` | O | API | Key rotation |
| `BOOTSTRAP_ADMIN_EMAIL` | R | API | First admin (one-time) |
| `BOOTSTRAP_ADMIN_PASSWORD` | S | API | Change after first login |
| `BOOTSTRAP_ADMIN_NAME` | O | API | Display name |

## Email

| Variable | R/O | Used by | Description |
|----------|-----|---------|-------------|
| `BREVO_API_KEY` | S | API | Transactional email |
| `BREVO_SENDER_EMAIL` | R | API | Verified sender in Brevo |
| `BREVO_SENDER_NAME` | O | API | From name |
| `BREVO_API_BASE` | O | API | Override API base (tests only) |

## Polymarket & worker tuning

| Variable | R/O | Used by | Description |
|----------|-----|---------|-------------|
| `POLYMARKET_CLOB_HOST` | O | API, Worker | Default public CLOB host |
| `POLYMARKET_DATA_API_HOST` | O | API, Worker | Data API |
| `POLYMARKET_GAMMA_HOST` | O | API, Worker | Gamma API |
| `POLYMARKET_RTDS_URL` | O | Worker | RTDS WebSocket |
| `WORKER_POLL_SECONDS` | O | Worker | Poll interval |
| `RECONCILIATION_SECONDS` | O | Worker | Live order reconciliation interval |
| `POLYGON_LOGS_ENABLED` | O | Worker | On-chain log ingestion |

See also [COOLIFY.md](COOLIFY.md) and [../COOLIFY_DEPLOYMENT.md](../COOLIFY_DEPLOYMENT.md).