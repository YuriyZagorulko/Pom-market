# Databases and test isolation

| Database | Purpose |
|----------|---------|
| `pommarket` | Local development data (not used by automated tests) |
| `pommarket_test` | Rust integration tests (`./scripts/test-integration.sh`) |
| `pommarket_e2e` | Playwright E2E (`./scripts/run-e2e.sh`) |

Guards:

- Integration tests refuse any URL whose database name is not exactly `pommarket_test`.
- E2E `global-setup` checks `GET /health` → `database_name` must be `pommarket_e2e`.
- Paper event inject (`E2E_MODE=1`) writes only to `pommarket_e2e`.

Create test databases: `bash scripts/ensure-test-databases.sh`.

E2E stack (ports 4324–4326 by default). Admin/User images are built with `API_INTERNAL_URL=http://api-e2e:4321` (rewrites are fixed at build time). `api-e2e` sets `APP_BASE_URL` / `USER_ORIGIN` to the E2E user port so invitation links hit `user-e2e`, not dev `:4323`.

```bash
docker compose -f docker-compose.yml -f docker-compose.e2e.yml --profile e2e up -d api-e2e worker-e2e admin-e2e user-e2e
```

Run full Playwright suite:

```bash
bash scripts/run-e2e.sh
```
