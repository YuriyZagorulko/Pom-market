-- Trade history backfill diagnostics (resume cursor, honest sync phases).

ALTER TABLE source_trader_analytics_sync
    ADD COLUMN IF NOT EXISTS trades_backfill_stop_reason text
        CHECK (trades_backfill_stop_reason IS NULL OR trades_backfill_stop_reason IN (
            'api_end', 'page_limit', 'error'
        )),
    ADD COLUMN IF NOT EXISTS trades_backfill_pages_fetched bigint NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS volume_stats jsonb NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE source_trader_analytics_sync DROP CONSTRAINT IF EXISTS source_trader_analytics_sync_sync_phase_check;
ALTER TABLE source_trader_analytics_sync ADD CONSTRAINT source_trader_analytics_sync_sync_phase_check
    CHECK (sync_phase IN (
        'idle', 'profile', 'trades_incremental', 'trades_backfill', 'backfill_partial', 'complete'
    ));