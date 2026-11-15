# Coolify production deployment — Pom Market

> **Auto-deploy / GitHub:** primary guide is **[deployment/COOLIFY.md](deployment/COOLIFY.md)** (migrations, health checks, webhook flow).

This guide prepares **Pom Market** for deployment on a Linux VPS using [Coolify](https://coolify.io) and `docker-compose.prod.yml`. It is documentation only; no production deploy is performed from the repository.

## Architecture (verified from repo)

| Service | Runtime | Container port | Public in production |
|---------|---------|----------------|----------------------|
| **postgres** | PostgreSQL 16 | 5432 | **No** — internal network only |
| **api** | Rust `pom-api` | 4321 | Optional (`pommarket-api.pomvix.com`) |
| **worker** | Rust `pom-worker` | — | **No** |
| **admin** | Next.js `@pom/admin` | 4322 | Yes (`pommarket-admin.pomvix.com`) |
| **user** | Next.js `@pom/user` | 4323 | Yes (`pommarket.pomvix.com`) |

**Local dev** (unchanged): `docker compose -f docker-compose.yml -f docker-compose.dev.yml up --build`

| Service | Host port (dev default) |
|---------|-------------------------|
| API | 4321 |
| Admin | 4322 |
| User | 4323 |
| PostgreSQL | 5440 (`127.0.0.1` only in base compose) |

Browsers talk to **User/Admin** over HTTPS. Next.js rewrites `/api/*` to `API_INTERNAL_URL` (`http://api:4321`). You do **not** need a public API domain unless you intentionally call the Rust API directly.

**No Redis/Celery** — not used by this codebase.

### Trading safety

- `CONFIRM_LIVE_TRADING` is forced to `"false"` in `docker-compose.prod.yml` for **api** and **worker** (env override).
- The worker does not auto-enable strategies on startup; live copy remains gated in application code (`LIVE_COPY_PRODUCT_READY`).
- Do not place Polymarket private keys in environment files.

---

## 1. Server prerequisites

- **OS:** Linux (Ubuntu 22.04+ or similar).
- **Coolify** installed on the VPS (current major version with Docker Compose resources).
- **GitHub** access to the Pom Market repository (read deploy key or OAuth).
- **DNS** (you configure later; single-level hosts, wildcard TLS `*.pomvix.com`):  
  - `pommarket.pomvix.com` → User  
  - `pommarket-admin.pomvix.com` → Admin  
  - `pommarket-api.pomvix.com` → API (optional direct access)

  | Type | Name | Content |
  |------|------|---------|
  | A | `pommarket` | VPS IP |
  | A | `pommarket-admin` | VPS IP |
  | A | `pommarket-api` | VPS IP |
- **Brevo** (or compatible) API key for email.
- **Resources (minimum):** 2 vCPU, 4 GB RAM, 40 GB disk (more for large analytics DBs).

---

## 2. Connect GitHub in Coolify

1. In Coolify: **Settings → Sources** → add your GitHub account / app.
2. Allow access to the Pom Market repository.
3. Use the default branch you deploy from (e.g. `main`).

---

## 3. Create project and environment

1. **New Project** → name e.g. `Pom Market`.
2. **New Environment** → e.g. `production` (Coolify terminology may vary).
3. Target: the VPS where Docker runs.

---

## 4. Docker Compose resource

1. **New Resource** → **Docker Compose**.
2. Repository: Pom Market.
3. **Compose file path:** `docker-compose.prod.yml` (repository root).
4. Build context is the repo root (all `build.context: .` paths assume this).

### What must be public vs internal

| Service | Coolify domain | Container port |
|---------|----------------|----------------|
| `user` | `https://pommarket.pomvix.com` | 4323 |
| `admin` | `https://pommarket-admin.pomvix.com` | 4322 |
| `api` | optional `https://pommarket-api.pomvix.com` | 4321 |
| `worker` | none | — |
| `postgres` | none | — |

Coolify’s proxy terminates TLS. Do **not** publish PostgreSQL to the host or the internet.

### Persistent volume

`docker-compose.prod.yml` declares volume `postgres-data`. In Coolify, ensure that volume is **persistent** across redeploys (default for named volumes). Do not delete it on routine updates.

---

## 5. Environment variables

1. On the server, copy `.env.production.example` → `.env` (Coolify UI: **Environment Variables** or env file upload).
2. Generate secrets on a trusted machine:

```bash
./scripts/generate-secrets.sh
```

3. Paste outputs and replace every `CHANGE_ME_*` placeholder.

**Required (minimum):**

| Variable | Purpose |
|----------|---------|
| `POSTGRES_PASSWORD` | Postgres superuser |
| `APP_DB_PASSWORD` | `pommarket_app` role |
| `WORKER_DB_PASSWORD` | `pommarket_worker` role |
| `MIGRATION_DATABASE_URL` | `postgres://pommarket:...@postgres:5432/pommarket` |
| `APP_DATABASE_URL` | `postgres://pommarket_app:...@postgres:5432/pommarket` |
| `WORKER_DATABASE_URL` | `postgres://pommarket_worker:...@postgres:5432/pommarket` |
| `JWT_SECRET` | Session signing (≥32 chars in production) |
| `ENCRYPTION_MASTER_KEY` | Credential vault (≥32 bytes) |
| `USER_ORIGIN` | `https://pommarket.pomvix.com` |
| `APP_BASE_URL` | same as `USER_ORIGIN` |
| `ADMIN_ORIGIN` | `https://pommarket-admin.pomvix.com` |
| `PUBLIC_API_ORIGIN` | `https://pommarket-api.pomvix.com` (optional; CORS) |
| `BREVO_*` | Transactional email |

**Safety:**

- `CONFIRM_LIVE_TRADING=false` (never enable until deliberate live rollout).
- `ENVIRONMENT=production` for strict secret validation in the Rust config.

**Pre-deploy validation (on server or CI):**

```bash
COMPOSE_FILE=docker-compose.prod.yml bash scripts/deploy/validate.sh
```

---

## 6. Domain configuration (operator)

1. Create DNS A/AAAA records for the three hostnames → VPS IP.
2. In Coolify, assign domains to **user**, **admin**, and optionally **api** services on the ports above.
3. Enable HTTPS (Let’s Encrypt) in Coolify for each public service.
4. Set env vars to match **exact** browser URLs (scheme + host, no trailing slash):

```env
USER_ORIGIN=https://pommarket.pomvix.com
APP_BASE_URL=https://pommarket.pomvix.com
ADMIN_ORIGIN=https://pommarket-admin.pomvix.com
PUBLIC_API_ORIGIN=https://pommarket-api.pomvix.com
```

5. Leave `API_INTERNAL_URL=http://api:4321` (Docker DNS).

**CORS:** The API allows `USER_ORIGIN`, `ADMIN_ORIGIN`, and `PUBLIC_API_ORIGIN` automatically. Use `CORS_ORIGINS` only for additional browser origins. Same-origin `/api` via Next.js is the primary path.

**Cookies / proxy:** Auth uses HTTP-only refresh cookies via the Next.js proxy. Keep `TRUSTED_PROXIES` empty unless you document a trusted edge that terminates TLS and connects directly to the API (see `docs/DEPLOYMENT.md`).

**WebSockets:** RTDS uses `POLYMARKET_RTDS_URL` from the worker/API outbound; no inbound WS through Coolify for the app UI.

---

## 7. First deployment

1. Run validation (`scripts/deploy/validate.sh`).
2. In Coolify: **Deploy** the Compose stack (build all images).
3. Wait until **postgres** is healthy, then **api** healthy (`GET /health` → `{"status":"ok"}`).
4. **Worker**, **admin**, and **user** start after API is healthy.

### Migrations (recommended explicit step)

API runs migrations on startup; for a controlled first install:

```bash
COMPOSE_FILE=docker-compose.prod.yml bash scripts/deploy/migrate.sh
```

Then deploy/start the full stack. Do not run multiple API instances racing migrations on first boot.

5. Open `https://pommarket-admin.pomvix.com`, sign in with `BOOTSTRAP_ADMIN_*`, change the password.
6. Open `https://pommarket.pomvix.com` and confirm login / `/health` on the user app.

---

## 8. Health checks and logs

| Service | Check |
|---------|--------|
| api | `curl -fsS http://127.0.0.1:4321/health` inside container |
| user | `GET /health` (proxies API health) |
| admin | HTTP 200 on `/` |
| postgres | `pg_isready -U pommarket -d pommarket` |

Coolify: use service health status and **Logs** for `api`, `worker`, `user`, `admin`.

```bash
docker compose -f docker-compose.prod.yml logs -f --tail=200 worker
docker compose -f docker-compose.prod.yml logs -f --tail=200 api
```

---

## 9. Backup and restore

**Backup:**

```bash
set -a && source .env && set +a
bash scripts/backup.sh
# or: bash scripts/deploy/backup.sh if using deploy wrapper
```

Store dumps off-host.

**Restore:** See `docs/DEPLOYMENT.md` — restore to a **new** database name first; never overwrite production without a maintenance window.

---

## 10. Updates

1. `bash scripts/backup.sh`
2. Pull new image/build in Coolify (redeploy Compose).
3. Prefer rolling **one** API instance through migration window (`scripts/deploy/migrate.sh` optional).
4. Verify `/health`, worker logs, paper strategy on a test account.
5. `CONFIRM_LIVE_TRADING` must remain `false` unless you have a signed-off live rollout.

---

## 11. Rollback

1. In Coolify, redeploy a previous **successful** deployment / image tag.
2. SQL migrations are forward-only; rollback DB only if a bad migration shipped (restore from backup).
3. Do not delete `postgres-data` volume during rollback.

---

## 12. Safe shutdown and restart

```bash
docker compose -f docker-compose.prod.yml stop
docker compose -f docker-compose.prod.yml up -d
```

`stop_grace_period` is set on long-running services for cleaner worker shutdown.

---

## Local production-stack verification (developer machine)

Without publishing ports to the internet:

```bash
cp .env.production.example .env
# Fill secrets (or use test DB URLs for build-only config check)
docker compose -f docker-compose.prod.yml config
COMPOSE_FILE=docker-compose.prod.yml bash scripts/deploy/validate.sh
docker compose -f docker-compose.prod.yml build
```

Optional loopback smoke with host ports:

```bash
docker compose -f docker-compose.prod.yml -f docker-compose.prod-local-ports.yml up -d --build
curl -fsS http://127.0.0.1:4321/health
curl -fsS http://127.0.0.1:4323/health
```

**Dev stack unchanged:**

```bash
docker compose -f docker-compose.yml -f docker-compose.dev.yml up --build
```

**Legacy prod overlay** (old two-file layout):

```bash
docker compose -f docker-compose.yml -f docker-compose.prod-legacy-overlay.yml up -d --build
```

---

## Coolify checklist (manual)

- [ ] Compose path: `docker-compose.prod.yml`
- [ ] `.env` / secrets in Coolify (no dev passwords)
- [ ] Domains on user:4323, admin:4322, optional api:4321
- [ ] `postgres-data` volume persistent
- [ ] `CONFIRM_LIVE_TRADING=false`
- [ ] `USER_ORIGIN` / `ADMIN_ORIGIN` / `APP_BASE_URL` match HTTPS URLs
- [ ] First login + change bootstrap admin password
- [ ] Backup cron / procedure documented for ops

---

## Related docs

- [DEPLOYMENT.md](DEPLOYMENT.md) — reverse proxy, `TRUSTED_PROXIES`, backup details  
- [deployment/coolify.md](deployment/coolify.md) — older `market.pomvix.com` staging compose (`docker-compose.coolify.yml`)