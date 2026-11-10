# Deployment (clean server)

## Prerequisites

- Docker with Compose v2
- DNS and TLS termination (reverse proxy) in front of `admin`, `user`, and optionally `api`
- Brevo API key for email

## Reverse proxy and client IP

### Request path in this project

There is **no edge reverse proxy in the repository**. The checked-in stack is:

`Browser → Admin/User (Next.js, ports 4322/4323) → Rust API (internal DNS api:4321)`

Optional **external** edge (nginx, Caddy, Traefik, etc.) is only shown in diagrams for production; you must supply and configure it yourself.

| Hop | TCP peer seen downstream | Who sets `X-Forwarded-For` | Notes |
|-----|-------------------------|-----------------------------|--------|
| Browser → Next | Next.js process | Browser **may** send a spoofed header; Next does not validate it | Next **rewrites** proxy `/api/*` and forwards request headers unless middleware strips them |
| Next → API | Admin/User container IP on the Docker network | Admin/User **middleware strips** `x-forwarded-for`, `x-real-ip`, `forwarded` on `/api/*` and `/health` before the rewrite | API should not receive browser-supplied forwarding headers through Next |
| Browser → API (dev only) | Client or host gateway | Client if present | **Not published** in `docker-compose.prod.yml`; with empty `TRUSTED_PROXIES`, API ignores XFF and uses peer IP |
| Edge → Next (your deploy) | Next.js | **Your edge** must **overwrite or append** using the real client IP — never forward browser XFF blindly | Not verifiable from this repo alone |

### `TRUSTED_PROXIES` (API)

- **Empty (default):** safest. API rate limits use **socket peer IP** only; spoofed `X-Forwarded-For` is ignored.
- **Non-empty:** comma-separated IPs/CIDRs of proxies whose **TCP connection** to the API may carry a trusted `X-Forwarded-For`. Parsed right-to-left; first untrusted hop wins. Invalid entries fail API startup; `*`, `0.0.0.0/0`, and `::/0` are rejected.
- **Do not** add Admin/User/Next.js or whole Docker subnets “so rate limits see the browser” — that re-enables XFF spoofing if anything forwards client headers.
- **Do not** trust `X-Real-IP` or `Forwarded` on the API (not used for rate limits).

With empty `TRUSTED_PROXIES` and Next middleware, login/recovery limits are keyed by **Next container peer IP** (shared across users behind the same Admin/User instance). Per-client IP at the API requires a path where **only your edge** sets forwarding headers and the API trusts **only that edge’s** TCP addresses—not Next.js.

### After deploy (smoke test)

1. Leave `TRUSTED_PROXIES` empty until edge placement is documented.
2. From a host that can reach User/Admin, run `bash scripts/verify-client-ip-chain.sh` (expects User on `USER_ORIGIN`, default `http://127.0.0.1:4323`).
3. Rebuild API and User/Admin after changing middleware or `TRUSTED_PROXIES` (`docker compose up -d --build api admin user`).

Integration tests set `TRUSTED_PROXIES=127.0.0.0/8` only on the **test API** process so direct localhost calls can simulate a trusted proxy; that is not production defaults.

## First install

1. Clone the repository and `cd` into it.
2. `cp .env.production.example .env`
3. Run `./scripts/generate-secrets.sh` and paste outputs into `.env`.
4. Set `ADMIN_ORIGIN`, `USER_ORIGIN`, `APP_BASE_URL`, `BREVO_*`, and one-time `BOOTSTRAP_ADMIN_*`.
5. Install proposed `pg_hba.conf` (see `scripts/pg_hba.conf.proposed`) and restrict Postgres host publish — see PostgreSQL hardening runbook.
6. `docker compose -f docker-compose.yml -f docker-compose.prod.yml up -d --build`
7. Verify: `curl -fsS http://127.0.0.1:4321/health`
8. Sign in to admin UI and change the bootstrap password if still using the one-time value.

The API applies SQL migrations from `backend/migrations/` on startup. With multiple API replicas, start **one** API instance first (or run `pom-api` once) so migrations are not raced.

## Backup

```bash
set -a && source .env && set +a
bash scripts/backup.sh
```

Store `backups/*.dump` off-host. Globals: `pg_dumpall --globals-only` (see security notes in hardening docs).

## Upgrade (new application version)

1. `bash scripts/backup.sh`
2. `git pull` / deploy new images
3. `docker compose -f docker-compose.yml -f docker-compose.prod.yml build`
4. Stop API replicas; run **one** new API container until migrations finish
5. `docker compose -f docker-compose.yml -f docker-compose.prod.yml up -d`
6. Check `/health`, worker logs, and a paper trade on a test account

Rollback application **without** DB restore: redeploy previous image tags; migrations are forward-only — only restore DB if a migration was incompatible.

## Restore backup (non-destructive verification)

Backups from `scripts/backup.sh` are custom-format dumps (`pg_dump -Fc`). **Do not** restore over the live `pommarket` database without a fresh backup and a maintenance window. Prefer restoring into a **separate database** first and comparing counts.

1. Confirm the Postgres service name and credentials from `.env` (`POSTGRES_*`, container name from `docker compose ps`).
2. Create a target database (does not touch existing data):

```bash
set -a && source .env && set +a
docker compose exec -T postgres psql -U "${POSTGRES_USER:-pommarket}" -d postgres -c \
  "SELECT 1 FROM pg_database WHERE datname = 'pommarket_restore_verify';" \
  | grep -q 1 || \
docker compose exec -T postgres psql -U "${POSTGRES_USER:-pommarket}" -d postgres -c \
  "CREATE DATABASE pommarket_restore_verify OWNER ${POSTGRES_USER:-pommarket};"
```

3. Restore the dump into that database:

```bash
DUMP=backups/pommarket-YYYYMMDD.dump   # your file
docker compose exec -T postgres pg_restore \
  -U "${POSTGRES_USER:-pommarket}" \
  -d pommarket_restore_verify \
  --no-owner --no-privileges \
  < "$DUMP"
```

Pipe form (same container, no host `pg_restore` required):

```bash
cat "$DUMP" | docker compose exec -i postgres pg_restore \
  -U "${POSTGRES_USER:-pommarket}" \
  -d pommarket_restore_verify \
  --no-owner --no-privileges
```

4. Spot-check (example):

```bash
docker compose exec -T postgres psql -U "${POSTGRES_USER:-pommarket}" -d pommarket_restore_verify -c \
  "SELECT COUNT(*) AS source_events FROM source_events;"
```

Compare with the primary DB:

```bash
docker compose exec -T postgres psql -U "${POSTGRES_USER:-pommarket}" -d pommarket -c \
  "SELECT COUNT(*) AS source_events FROM source_events;"
```

**Warning:** `pg_restore --clean` drops objects in the target database before restore. Use only when you intend to wipe that database’s schema/data. Never point `--clean` at production `pommarket` without explicit approval and a current backup.

To replace the primary database, use a dedicated maintenance procedure (backup → new DB or controlled cutover), not an in-place `--clean` on the only copy.

## Local production-like check

```bash
cp .env.production.example .env  # fill secrets for local trial
docker compose -f docker-compose.yml -f docker-compose.prod.yml config
docker compose -f docker-compose.yml -f docker-compose.prod.yml up -d --build
```
