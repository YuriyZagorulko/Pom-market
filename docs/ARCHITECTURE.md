# Architecture

One Rust crate, `backend/`, builds two binaries:

- `pom-api` serves `/api/v1`, applies migrations, bootstraps the first admin, and listens for `pom_activity` notifications.
- `pom-worker` polls the Data API, keeps an RTDS socket with reconnect, optionally reads Polygon logs, and runs reconciliation.

PostgreSQL roles:

- `pommarket` owns the schema and is used only for migrations and the one-time admin insert.
- `pommarket_app` and `pommarket_worker` are not superusers and cannot bypass row level security.
- Every request sets `app.user_id` and `app.role` for the transaction. Unauthenticated login uses narrow security-definer functions that never return trading ciphertext.

Copy decisions are unique per strategy and source event. The dedup key is `tx_hash|wallet|token|side`. Extra fills of that key update the source event and do not create another decision. A buy that is rejected does not increase `source_observed_shares`. A sell cannot exceed the open shares attributed to that source wallet, account, and mode.

Live submission commits `submitting` before the HTTP call. Timeouts become `unknown` and are not resent.
