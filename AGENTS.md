# Pom-Market

Rust API and worker in `backend/`. Admin Next.js app in `apps/admin` (port 4322). User Next.js app in `apps/user` (port 4323). PostgreSQL with RLS. Do not add a Python trading backend.

Do not copy source from `AndreySchurko/garnet-polymarket`. That repository is BUSL-1.1 and its free grant does not cover this multi-user product.

Never enable `CONFIRM_LIVE_TRADING` in tests. Never log or return private keys. Schema changes go only through the linear SQL migration in `backend/migrations`. Do not commit or push unless the user asks.

## Local dev: login 500 / API down

In `docker-compose.dev.yml`, API and worker use hot reload. If `cargo build` fails during edits, the old `cargo watch` setup **stopped** the API → login returns 500 or “Could not reach the API”. Dev containers now use `docker/dev-api-watch.sh`, which **keeps the last good binary** when a build fails.

Other common causes: editing an already-applied migration (checksum mismatch — worker/API exit on startup). Run `scripts/verify-migration-checksums.sh`. Never modify applied migration SQL; add the next `00NN_*.sql` instead.

Quick checks: `curl -s http://127.0.0.1:4321/health` → `{"status":"ok"}`. `POST /api/v1/auth/login` should return 200 or 401, not 500.
