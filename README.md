# Pom-Market

Self-hosted, multi-user Polymarket copy-trading platform.

- Rust API and worker (`pom-api`, `pom-worker`) on port 4321
- Admin console at `http://localhost:4322`
- User dashboard at `http://localhost:4323`
- PostgreSQL is the source of truth, with row-level security for the application roles

Garnet (`github.com/AndreySchurko/garnet-polymarket`) is licensed under BUSL-1.1. Its additional use grant does not cover a multi-user product that trades other people's funds. This repository does not copy Garnet source. The engine follows the same safety ideas — one dedup key, FAK-style execution, an intent row committed before a live submit, and no blind retry of an unknown order — implemented independently against the official Polymarket CLOB V2 Rust client and public APIs.

## Start

```bash
cp .env.example .env
./scripts/generate-secrets.sh
# paste the generated values into .env and set BOOTSTRAP_ADMIN_EMAIL

docker compose up -d --build
```

### Docker development (hot reload)

Production images bake a `next build` output, so UI changes require a rebuild (`docker compose up -d --build user`). For day-to-day work, use the development overlay:

```bash
./scripts/docker-dev.sh up --build
# or: docker compose -f docker-compose.yml -f docker-compose.dev.yml up --build
```

This keeps the same Postgres volume and ports, runs Next.js in dev mode (Fast Refresh) for admin and user, and runs `cargo watch` for the API and worker. Stop with `./scripts/docker-dev.sh down` (omit `-v` so `postgres-data` is preserved).

On an empty database the API applies `backend/migrations/0001_init.sql`, creates login passwords for `pommarket_app` and `pommarket_worker`, and inserts the bootstrap administrator if none exists.

Then:

1. Open `http://localhost:4322` and sign in as the bootstrap admin.
2. Create a user. Copy the one-time setup URL.
3. Open that URL on `http://localhost:4323`, set a password, and sign in.
4. Add a Polymarket account. It starts in monitoring.
5. Add source wallets and a strategy per account. Strategies start disabled.
6. Switch an account to paper trading only after you set paper cash and risk limits.
7. Leave `CONFIRM_LIVE_TRADING=false` until you have checked credentials, balance, and risk and you intend to send real orders.

## Local processes without Docker

```bash
# Postgres must already be running and the URLs in .env must point at it.
cargo run --manifest-path backend/Cargo.toml --bin pom-api
cargo run --manifest-path backend/Cargo.toml --bin pom-worker
pnpm install
pnpm dev:admin
pnpm dev:user
```

## Checks

```bash
cargo test --manifest-path backend/Cargo.toml --lib
./scripts/test-integration.sh
bash scripts/run-e2e.sh
pnpm --filter @pom/admin build
pnpm --filter @pom/user build
```

Test databases: see [docs/TESTING.md](docs/TESTING.md) (`pommarket`, `pommarket_test`, `pommarket_e2e`).

Production deployment notes: [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) and [.env.production.example](.env.production.example).

## Encryption rotation

1. Put the current `ENCRYPTION_MASTER_KEY` into `ENCRYPTION_MASTER_KEY_PREVIOUS`.
2. Put its id into `ENCRYPTION_KEY_ID_PREVIOUS`.
3. Generate a new master key and set `ENCRYPTION_KEY_ID` to a new id.
4. Run `pom-api --rotate-keys` before serving traffic with the new key alone.

Private keys are encrypted with AES-256-GCM. The additional authenticated data binds the ciphertext to the user and the trading account. API responses never include the key.

## Polymarket connectivity

`https://clob.polymarket.com` currently answers `GET /version` with `2`. The name `clob-v2.polymarket.com` has no public DNS record, so the default host is `clob.polymarket.com`. Data API v1 paths return HTTP 410; the worker uses `/v2/activity`, `/v2/trades`, `/v2/positions`, and `/v2/value`.

If the host resolver returns `NOERROR` with an empty answer for these names, glibc reports “no address”. That was the failure on the development machine: systemd-resolved was using `192.168.43.205`, while `1.1.1.1` returned the Cloudflare addresses. Docker services are configured with `1.1.1.1` and `8.8.8.8` for that reason. The application does not invent a successful response when DNS fails.

An order left in `unknown` stays `unknown` until an exchange status is known. A timeout is not treated as `failed`, and the order is not sent again.

## Limits that are intentional

- Invitations are shown once in the admin console. There is no mail transport.
- Polygon log ingestion is off unless `POLYGON_LOGS_ENABLED=true`. The decoder understands the standard CTF `OrderFilled` event (collateral asset id 0) and ignores logs it cannot decode.
- Live orders are refused unless the account completed the confirmation phrase, the server has `CONFIRM_LIVE_TRADING=true`, and the global kill switch is off.
- A timed-out live submit is marked `unknown` and then `failed` after reconciliation. It is not sent again automatically.
- Taker fees are recorded as unknown when Polymarket does not return a fee schedule and `TAKER_FEE_RATE=0`.
