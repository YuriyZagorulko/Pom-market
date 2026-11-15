-- Historical activity cash flows (REDEEM / resolution) for analytics + Calculate.

CREATE TABLE source_trader_analytics_cashflows (
    id bigserial PRIMARY KEY,
    source_trader_id uuid NOT NULL REFERENCES source_traders(id) ON DELETE CASCADE,
    dedup_key text NOT NULL,
    event_type text NOT NULL CHECK (event_type IN ('redeem')),
    resolution_class text NOT NULL CHECK (resolution_class IN ('winning', 'losing', 'unresolved')),
    event_ts timestamptz NOT NULL,
    token_id text NOT NULL,
    condition_id text,
    size_shares numeric(24, 8) NOT NULL,
    payout_usd numeric(24, 8),
    tx_hash text,
    origin text NOT NULL DEFAULT 'data_api_activity',
    raw jsonb NOT NULL DEFAULT '{}'::jsonb,
    UNIQUE (source_trader_id, dedup_key)
);

CREATE INDEX source_trader_analytics_cashflows_trader_ts
    ON source_trader_analytics_cashflows (source_trader_id, event_ts ASC);

ALTER TABLE source_trader_analytics_cashflows ENABLE ROW LEVEL SECURITY;
ALTER TABLE source_trader_analytics_cashflows FORCE ROW LEVEL SECURITY;

CREATE POLICY source_trader_analytics_cashflows_read ON source_trader_analytics_cashflows FOR SELECT
    USING (EXISTS (
        SELECT 1 FROM source_traders t
        WHERE t.id = source_trader_analytics_cashflows.source_trader_id AND app_owns(t.user_id)
    ));

CREATE POLICY source_trader_analytics_cashflows_write ON source_trader_analytics_cashflows FOR ALL
    USING (app_role() = 'worker') WITH CHECK (app_role() = 'worker');

GRANT SELECT, INSERT, UPDATE, DELETE ON source_trader_analytics_cashflows TO pommarket_worker;
GRANT SELECT ON source_trader_analytics_cashflows TO pommarket_app;
GRANT USAGE, SELECT ON SEQUENCE source_trader_analytics_cashflows_id_seq TO pommarket_worker;

ALTER TABLE source_trader_analytics_sync
    ADD COLUMN IF NOT EXISTS activity_cursor text,
    ADD COLUMN IF NOT EXISTS activity_backfill_complete boolean NOT NULL DEFAULT false,
    ADD COLUMN IF NOT EXISTS activity_backfill_stop_reason text
        CHECK (activity_backfill_stop_reason IS NULL OR activity_backfill_stop_reason IN (
            'api_end', 'page_limit', 'error'
        )),
    ADD COLUMN IF NOT EXISTS activity_backfill_pages_fetched bigint NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS activity_ingested_count bigint NOT NULL DEFAULT 0;

ALTER TABLE source_trader_analytics_sync DROP CONSTRAINT IF EXISTS source_trader_analytics_sync_sync_phase_check;
ALTER TABLE source_trader_analytics_sync ADD CONSTRAINT source_trader_analytics_sync_sync_phase_check
    CHECK (sync_phase IN (
        'idle', 'profile', 'trades_incremental', 'trades_backfill',
        'activity_incremental', 'activity_backfill', 'backfill_partial', 'complete'
    ));