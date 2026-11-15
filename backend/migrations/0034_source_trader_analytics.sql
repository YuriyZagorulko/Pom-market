-- Source trader Polymarket analytics cache (Data API v2).

CREATE TABLE source_trader_analytics_sync (
    source_trader_id uuid PRIMARY KEY REFERENCES source_traders(id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status text NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'syncing', 'ready', 'error', 'stale')),
    sync_phase text NOT NULL DEFAULT 'idle'
        CHECK (sync_phase IN ('idle', 'profile', 'trades_incremental', 'trades_backfill', 'complete')),
    last_success_at timestamptz,
    last_attempt_at timestamptz,
    last_error text,
    profile_stats jsonb NOT NULL DEFAULT '{}'::jsonb,
    positions_open jsonb NOT NULL DEFAULT '[]'::jsonb,
    positions_redeemable jsonb NOT NULL DEFAULT '[]'::jsonb,
    trades_cursor text,
    trades_backfill_complete boolean NOT NULL DEFAULT false,
    trades_ingested_count bigint NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX source_trader_analytics_sync_status ON source_trader_analytics_sync (status, updated_at DESC);

CREATE TABLE source_trader_pnl_points (
    id bigserial PRIMARY KEY,
    source_trader_id uuid NOT NULL REFERENCES source_traders(id) ON DELETE CASCADE,
    point_ts timestamptz NOT NULL,
    pnl_interval text NOT NULL,
    economic_pnl numeric(24, 8),
    trade_pnl numeric(24, 8),
    realized_pnl numeric(24, 8),
    unrealized_pnl numeric(24, 8),
    position_pnl numeric(24, 8),
    settled_pnl numeric(24, 8),
    fees numeric(24, 8),
    wallet_income numeric(24, 8),
    volume_usdc numeric(24, 8),
    raw jsonb NOT NULL DEFAULT '{}'::jsonb,
    fetched_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (source_trader_id, pnl_interval, point_ts)
);

CREATE INDEX source_trader_pnl_points_trader_ts
    ON source_trader_pnl_points (source_trader_id, point_ts);

CREATE TABLE source_trader_analytics_trades (
    id bigserial PRIMARY KEY,
    source_trader_id uuid NOT NULL REFERENCES source_traders(id) ON DELETE CASCADE,
    dedup_key text NOT NULL,
    event_ts timestamptz NOT NULL,
    side text NOT NULL CHECK (side IN ('buy', 'sell')),
    notional_usd numeric(24, 8) NOT NULL,
    price numeric(24, 8) NOT NULL,
    size_shares numeric(24, 8) NOT NULL,
    token_id text,
    condition_id text,
    market_title text,
    outcome text,
    market_slug text,
    tx_hash text,
    origin text NOT NULL DEFAULT 'data_api',
    raw jsonb NOT NULL DEFAULT '{}'::jsonb,
    UNIQUE (source_trader_id, dedup_key)
);

CREATE INDEX source_trader_analytics_trades_trader_ts
    ON source_trader_analytics_trades (source_trader_id, event_ts DESC);

CREATE INDEX source_trader_analytics_trades_trader_side_ts
    ON source_trader_analytics_trades (source_trader_id, side, event_ts DESC);

ALTER TABLE source_trader_analytics_sync ENABLE ROW LEVEL SECURITY;
ALTER TABLE source_trader_analytics_sync FORCE ROW LEVEL SECURITY;
ALTER TABLE source_trader_pnl_points ENABLE ROW LEVEL SECURITY;
ALTER TABLE source_trader_pnl_points FORCE ROW LEVEL SECURITY;
ALTER TABLE source_trader_analytics_trades ENABLE ROW LEVEL SECURITY;
ALTER TABLE source_trader_analytics_trades FORCE ROW LEVEL SECURITY;

CREATE POLICY source_trader_analytics_sync_read ON source_trader_analytics_sync FOR SELECT
    USING (app_owns(user_id) OR app_is_worker() OR app_is_admin());

CREATE POLICY source_trader_analytics_sync_user_write ON source_trader_analytics_sync
    FOR INSERT
    WITH CHECK (app_owns(user_id));

CREATE POLICY source_trader_analytics_sync_user_update ON source_trader_analytics_sync
    FOR UPDATE
    USING (app_owns(user_id))
    WITH CHECK (app_owns(user_id));

CREATE POLICY source_trader_analytics_sync_worker_write ON source_trader_analytics_sync FOR ALL
    USING (app_is_worker())
    WITH CHECK (app_is_worker());

CREATE POLICY source_trader_pnl_points_read ON source_trader_pnl_points FOR SELECT
    USING (
        app_is_worker() OR app_is_admin()
        OR EXISTS (
            SELECT 1 FROM source_traders t
            WHERE t.id = source_trader_pnl_points.source_trader_id AND app_owns(t.user_id)
        )
    );

CREATE POLICY source_trader_pnl_points_write ON source_trader_pnl_points FOR ALL
    USING (app_is_worker())
    WITH CHECK (app_is_worker());

CREATE POLICY source_trader_analytics_trades_read ON source_trader_analytics_trades FOR SELECT
    USING (
        app_is_worker() OR app_is_admin()
        OR EXISTS (
            SELECT 1 FROM source_traders t
            WHERE t.id = source_trader_analytics_trades.source_trader_id AND app_owns(t.user_id)
        )
    );

CREATE POLICY source_trader_analytics_trades_write ON source_trader_analytics_trades FOR ALL
    USING (app_is_worker())
    WITH CHECK (app_is_worker());

GRANT SELECT, INSERT, UPDATE, DELETE
    ON source_trader_analytics_sync,
       source_trader_pnl_points,
       source_trader_analytics_trades
    TO pommarket_app, pommarket_worker;

GRANT USAGE, SELECT ON SEQUENCE source_trader_pnl_points_id_seq TO pommarket_app, pommarket_worker;
GRANT USAGE, SELECT ON SEQUENCE source_trader_analytics_trades_id_seq TO pommarket_app, pommarket_worker;