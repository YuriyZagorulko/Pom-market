-- Close-cycle idempotency (re-open safe) and historical open shares after each order fill.

ALTER TABLE positions
    ADD COLUMN IF NOT EXISTS close_cycle bigint NOT NULL DEFAULT 0;

ALTER TABLE position_close_events
    ADD COLUMN IF NOT EXISTS close_cycle bigint;

UPDATE position_close_events SET close_cycle = 0 WHERE close_cycle IS NULL;

ALTER TABLE position_close_events
    ALTER COLUMN close_cycle SET NOT NULL,
    ALTER COLUMN close_cycle SET DEFAULT 0;

CREATE UNIQUE INDEX IF NOT EXISTS position_close_events_position_cycle_key
    ON position_close_events (position_id, close_cycle);

ALTER TABLE orders
    ADD COLUMN IF NOT EXISTS position_open_shares_after numeric(24, 8) NULL;
