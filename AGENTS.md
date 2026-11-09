# Pom-Market

Rust API and worker in `backend/`. Admin Next.js app in `apps/admin` (port 4322). User Next.js app in `apps/user` (port 4323). PostgreSQL with RLS. Do not add a Python trading backend.

Do not copy source from `AndreySchurko/garnet-polymarket`. That repository is BUSL-1.1 and its free grant does not cover this multi-user product.

Never enable `CONFIRM_LIVE_TRADING` in tests. Never log or return private keys. Schema changes go only through the linear SQL migration in `backend/migrations`. Do not commit or push unless the user asks.
