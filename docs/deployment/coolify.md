# Deploy POM Market on Coolify (staging: market.pomvix.com)

This guide covers **connecting the repository to Coolify** and operating the stack. Repository preparation is separate from running deploy on a server.

## Architecture

| Service   | Role                         | Published |
|-----------|------------------------------|-----------|
| `user`    | Next.js user UI              | **Yes** — map domain `market.pomvix.com` to port **4323** |
| `api`     | Rust HTTP API                | No — `http://api:4321` on Docker network |
| `worker`  | Rust background worker       | No |
| `postgres`| PostgreSQL 16                | No — volume `postgres-data` |
| `admin`   | Next.js admin (optional)     | Only if profile `admin` enabled |

Browser traffic uses **same-origin** `/api/*` on the user app (Next.js rewrites to `API_INTERNAL_URL`). Internal Docker hostnames are never exposed to the browser.

**Live trading:** `CONFIRM_LIVE_TRADING` is forced to `false` in `docker-compose.coolify.yml`. Do not enable real CLOB submission or wallet keys in staging.

## Prerequisites

- Coolify host with Docker Compose v2
- Git repository access (this repo)
- Brevo (or compatible) for email
- DNS: `market.pomvix.com` → Coolify proxy (you configure DNS; this doc does not)

## 1. Connect the Git repository

1. In Coolify, create a **Docker Compose** resource.
2. Point it at this repository and branch.
3. Set **Compose file** to: `docker-compose.coolify.yml`

## 2. Public service and domain

1. Mark **`user`** as the publicly exposed service (or the service Coolify should terminate TLS for).
2. Set domain: **`market.pomvix.com`**
3. Container port: **4323** (`USER_PORT` in `.env` should match, default 4323).
4. Do **not** publish `postgres`, `api`, or `worker` to the public internet.

## 3. Environment variables

1. On the server, create `.env` from **`.env.coolify.example`** (never commit `.env`).
2. Run `./scripts/generate-secrets.sh` locally or on the server and paste values.
3. Set at minimum:

   - `POSTGRES_PASSWORD`, `APP_DB_PASSWORD`, `WORKER_DB_PASSWORD`
   - `MIGRATION_DATABASE_URL`, `APP_DATABASE_URL`, `WORKER_DATABASE_URL` (hosts must be `postgres`, not `localhost`)
   - `JWT_SECRET`, `ENCRYPTION_MASTER_KEY`
   - `USER_ORIGIN` and `APP_BASE_URL` → `https://market.pomvix.com`
   - `BREVO_*`
   - `BOOTSTRAP_ADMIN_*` (one-time; change password after first login)

4. Keep **`CONFIRM_LIVE_TRADING=false`**.
5. Use **`ENVIRONMENT=staging`** for this remote dev/staging host (`production` enables stricter `Config::validate()` secret rules).

In Coolify, map the same variables into the compose project (env file or UI).

## 4. Persistent volumes

Ensure Coolify preserves:

| Volume           | Purpose              |
|------------------|----------------------|
| `postgres-data`  | PostgreSQL data      |

Application state lives in Postgres; no extra app volumes are required unless you add file uploads later.

## 5. HTTPS

Enable HTTPS on **`user`** in Coolify for `market.pomvix.com`. Coolify terminates TLS; set `USER_ORIGIN` and `APP_BASE_URL` to `https://market.pomvix.com`.

Leave **`TRUSTED_PROXIES` empty** unless the API is reached directly from an edge that sets `X-Forwarded-For` (this stack does not publish API publicly). See `docs/DEPLOYMENT.md` for rate-limit / XFF notes.

## 6. Build and start services

From the project root (or let Coolify build):

```bash
docker compose -f docker-compose.coolify.yml --env-file .env config
docker compose -f docker-compose.coolify.yml --env-file .env up -d --build
```

Order: `postgres` → `api` (healthy) → `worker` + `user`.

Validate locally or on server before/after first deploy:

```bash
COMPOSE_FILE=docker-compose.coolify.yml ENV_FILE=.env ./scripts/deploy/validate.sh
```

## 7. Run migrations (explicit step)

API and worker still run migrations on startup (idempotent). For a controlled first deploy, run migrations **once** before scaling:

```bash
./scripts/deploy/migrate.sh
```

This starts Postgres, waits for readiness, runs `pom-api --migrate-only` (migrations + login roles + optional bootstrap admin), then exits.

After migrations, start or restart the full stack from Coolify.

## 8. Verify health

| Check | Command / URL |
|-------|----------------|
| User UI | `https://market.pomvix.com` |
| User health | `https://market.pomvix.com/health` (or internal `wget` on :4323) |
| API via proxy | `https://market.pomvix.com/api/v1/...` (e.g. health if exposed through Next) |
| API internal | `docker compose exec api curl -fsS http://127.0.0.1:4321/health` |
| Worker | `docker compose logs worker --tail=50` (no HTTP health endpoint) |

## 9. Database persistence

1. Create a test row or sign up a test user.
2. Recreate only the `user` container — data should remain.
3. Recreate `postgres` **only** if the `postgres-data` volume is attached; never `docker volume rm` without a backup.

## 10. Backup and restore

**Backup:**

```bash
./scripts/deploy/backup.sh
```

Writes `backups/pommarket-<timestamp>.dump` (custom format). Copy off-host.

**Restore (destructive):**

```bash
CONFIRM_RESTORE=YES ./scripts/deploy/restore.sh backups/pommarket-YYYYMMDD.dump
```

Stops api/worker, restores into `pommarket`, then restart services manually.

Legacy scripts `scripts/backup.sh` / `scripts/restore.sh` target `docker-compose.prod.yml`; prefer `scripts/deploy/*` for Coolify.

## 11. Optional admin UI

Enable compose profile `admin` and attach a separate Coolify domain to port **4322**. Set `ADMIN_ORIGIN` in `.env`. Not required for `market.pomvix.com` user staging.

## 12. Troubleshooting

| Symptom | Likely cause | Action |
|---------|--------------|--------|
| Login 500 / “Could not reach API” | API down or build failed | `docker compose logs api`; `curl` internal health |
| API exits on start | Migration checksum mismatch or bad `.env` | `bash scripts/verify-migration-checksums.sh`; check `MIGRATION_DATABASE_URL` |
| `Config::validate` error | Weak secrets with `ENVIRONMENT=production` | Use `staging` or generate strong secrets |
| CORS errors | Wrong `USER_ORIGIN` | Must match browser URL exactly (https) |
| Worker idle errors | Postgres not ready | Check `depends_on` and worker logs |
| Migrations raced | Multiple cold API starts | Run `./scripts/deploy/migrate.sh` first |

## Manual checklist before first deployment

1. Copy `.env.coolify.example` → `.env` on server; generate secrets.
2. Coolify: compose file `docker-compose.coolify.yml`, domain on `user:4323`.
3. `./scripts/deploy/validate.sh`
4. `./scripts/deploy/migrate.sh`
5. `up -d --build` full stack
6. Login, change bootstrap admin password, confirm paper-only behavior.

No live trading, no production wallet keys, no `CONFIRM_LIVE_TRADING=true`.