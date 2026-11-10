-- Align source equity status with documented quality levels (never store legacy 'ok').

ALTER TABLE source_trader_equity_snapshots DROP CONSTRAINT IF EXISTS source_trader_equity_snapshots_status_check;
UPDATE source_trader_equity_snapshots SET status = 'partial' WHERE status = 'ok';
ALTER TABLE source_trader_equity_snapshots ADD CONSTRAINT source_trader_equity_snapshots_status_check
    CHECK (status IN ('complete', 'partial', 'stale', 'unavailable'));
