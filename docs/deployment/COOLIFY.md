# Coolify auto-deploy — POM Market

**Repository preparation only.** You connect GitHub and Coolify manually; nothing in this repo deploys production by itself.

## Architecture

| Service | Image build | Port | Public HTTP | Health check |
|---------|-------------|------|-------------|--------------|
| `postgres` | `postgres:16-alpine` | 5432 | **No** | `pg_isready` |
| `migrate` | `docker/backend.Dockerfile` | — | No | exits 0 after `pom-api --migrate-only` |
| `api` | same | 4321 | Optional domain | `GET /health` |
| `worker` | same | — | **No** | `curl` → `api:4321/health` |
| `admin` | `docker/admin.Dockerfile` | 4322 | Yes | HTTP `/` |
| `user` | `docker/user.Dockerfile` | 4323 | Yes | `GET /health` |

Compose file: **`docker-compose.prod.yml`** (repository root).

Browser traffic: HTTPS → Coolify proxy → **user** / **admin**. Next.js proxies `/api/*` to `http://api:4321`. Worker and Postgres are internal only.

**Trading safety:** `CONFIRM_LIVE_TRADING` is forced to `"false"` on api/worker in compose. No auto-enable of strategies on restart.

---

## GitHub + Coolify setup (manual)

1. **Push** this repository to GitHub (you do this later; not part of agent work).
2. Coolify → **Sources** → connect GitHub, authorize the POM Market repo.
3. **New project** → e.g. `POM Market` → **production** environment on your VPS.
4. **New resource** → **Docker Compose**:
   - Repository: POM Market
   - Branch: e.g. `main` (your production branch)
   - **Compose file:** `docker-compose.prod.yml`
   - Build context: repo root (default)
5. **Environment variables:** paste from `.env.production.example`, run `./scripts/generate-secrets.sh` locally, fill all `CHANGE_ME_*`. Upload or paste in Coolify secrets UI.
6. **Domains** (Coolify UI, HTTPS). All hosts are single-level names under `pomvix.com` (wildcard cert `*.pomvix.com`):
   - `user` → `https://pommarket.pomvix.com` → port **4323**
   - `admin` → `https://pommarket-admin.pomvix.com` → port **4322**
   - `api` → `https://pommarket-api.pomvix.com` → port **4321** (optional if all traffic uses Next.js `/api` proxy)
7. **DNS** (Cloudflare or your DNS; point to Coolify/VPS IP):

   | Type | Name | Content |
   |------|------|---------|
   | A | `pommarket` | VPS IP |
   | A | `pommarket-admin` | VPS IP |
   | A | `pommarket-api` | VPS IP |

8. **Volume:** ensure `postgres-data` is persistent (do not delete on redeploy).
9. **Auto deploy:** enable webhook / “deploy on push” for the selected branch in Coolify.
10. First deploy → verify health (below) → change bootstrap admin password.

Pre-deploy on server or CI:

```bash
COMPOSE_FILE=docker-compose.prod.yml bash scripts/deploy/validate.sh
```

---

## Environment variables

Full table: [ENVIRONMENT.md](ENVIRONMENT.md). Template: `.env.production.example`.

Minimum for a green deploy:

```env
ENVIRONMENT=production
CONFIRM_LIVE_TRADING=false
POSTGRES_PASSWORD=…
APP_DB_PASSWORD=…
WORKER_DB_PASSWORD=…
MIGRATION_DATABASE_URL=postgres://pommarket:…@postgres:5432/pommarket
APP_DATABASE_URL=postgres://pommarket_app:…@postgres:5432/pommarket
WORKER_DATABASE_URL=postgres://pommarket_worker:…@postgres:5432/pommarket
JWT_SECRET=…
ENCRYPTION_MASTER_KEY=…
USER_ORIGIN=https://pommarket.pomvix.com
APP_BASE_URL=https://pommarket.pomvix.com
ADMIN_ORIGIN=https://pommarket-admin.pomvix.com
PUBLIC_API_ORIGIN=https://pommarket-api.pomvix.com
API_INTERNAL_URL=http://api:4321
BREVO_API_KEY=…
```

Domains must match Coolify URLs exactly (scheme + host).

---

## Migrations (no destructive ops)

1. **`migrate` service** runs `pom-api --migrate-only` once per deploy (`restart: "no"`).
2. **API** and **worker** start only after `migrate` **completes successfully** (`service_completed_successfully`).
3. API/worker set `POM_SKIP_STARTUP_MIGRATION=true` to avoid racing duplicate SQLx runs.
4. Migrations are forward-only SQL in `backend/migrations/` (single chain).
5. **Never** delete `postgres-data` volume on routine updates.
6. Manual fallback: `COMPOSE_FILE=docker-compose.prod.yml bash scripts/deploy/migrate.sh`

If `migrate` fails, API/worker do not start → deploy fails visibly in Coolify logs.

---

## Health checks & auto-deploy reliability

Coolify should treat:

- **postgres** — healthy when `pg_isready` passes
- **migrate** — success = exit 0 (one-shot)
- **api** — `curl http://127.0.0.1:4321/health` → `{"status":"ok",…}`
- **worker** — can reach `http://api:4321/health` (process + dependency)
- **user** — `GET /health` (checks upstream API)
- **admin** — HTTP 200 on `/`

A running container with a crash-looping app should fail health checks (api/user/admin).

After auto-deploy:

```bash
# on server, if you have shell access
docker compose -f docker-compose.prod.yml ps
docker compose -f docker-compose.prod.yml logs migrate --tail=50
docker compose -f docker-compose.prod.yml logs api --tail=100
docker compose -f docker-compose.prod.yml logs worker --tail=100
```

---

## Rollback

1. Coolify → redeploy previous successful build / git SHA.
2. Do **not** drop `postgres-data` unless restoring from backup.
3. If a bad migration shipped: restore DB from `scripts/backup.sh` dump (maintenance window).

---

## Troubleshooting

| Symptom | Likely cause |
|---------|----------------|
| `migrate` exits non-zero | DB URL, password, or migration SQL error — read migrate logs |
| API unhealthy | Migrations incomplete, bad secrets, or DB unreachable |
| User 502 / API errors | `API_INTERNAL_URL` wrong; api not healthy |
| CORS / login redirect | `USER_ORIGIN` / `ADMIN_ORIGIN` mismatch with browser URL |
| Worker restarts | API not healthy; check `CONFIRM_LIVE_TRADING` and DB |
| Live orders | Should be impossible: `CONFIRM_LIVE_TRADING=false` + product gates in code |

---

## Local validation (developers)

```bash
POSTGRES_PASSWORD=test docker compose -f docker-compose.prod.yml config
COMPOSE_FILE=docker-compose.prod.yml bash scripts/deploy/validate.sh
docker compose -f docker-compose.prod.yml build
# Optional loopback ports:
docker compose -f docker-compose.prod.yml -f docker-compose.prod-local-ports.yml up -d
```

### Isolated production smoke test (do not use dev `.env`)

Root `.env` is loaded by Compose for variable substitution and can override `--env-file` if the same keys exist in your shell. Use a separate project name, env file, and loopback ports:

```bash
cp .env.production.example .env.smoke && ./scripts/generate-secrets.sh >> .env.smoke
# Quote values that contain spaces (e.g. BOOTSTRAP_ADMIN_NAME, BREVO_SENDER_NAME).
# Point origins at smoke ports, e.g. USER_ORIGIN=http://127.0.0.1:14323

export COMPOSE_PROJECT_NAME=pommarket-smoke
export COMPOSE_EXTRA_FILES="/tmp/pommarket-smoke-compose.yml"  # overlay: .env.smoke + ports 14321–14323
set -a && source .env.smoke && set +a

COMPOSE_FILE=docker-compose.prod.yml ENV_FILE=.env.smoke bash scripts/deploy/validate.sh
docker compose -p "$COMPOSE_PROJECT_NAME" -f docker-compose.prod.yml -f /tmp/pommarket-smoke-compose.yml \
  --env-file .env.smoke up -d --build
```

Teardown (only the smoke project):

```bash
docker compose -p pommarket-smoke -f docker-compose.prod.yml -f /tmp/pommarket-smoke-compose.yml down -v
```

Dev stack unchanged: `docker compose -f docker-compose.yml -f docker-compose.dev.yml up --build`

---

## Related

- [../COOLIFY_DEPLOYMENT.md](../COOLIFY_DEPLOYMENT.md) — extended operator guide  
- [../DEPLOYMENT.md](../DEPLOYMENT.md) — `TRUSTED_PROXIES`, backup/restore  
- [coolify.md](coolify.md) — legacy single-UI `docker-compose.coolify.yml`