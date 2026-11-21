-- Allow copy decisions to wait for short liquidity retry windows (BUY only).
ALTER TABLE copy_decisions DROP CONSTRAINT IF EXISTS copy_decisions_status_check;
ALTER TABLE copy_decisions ADD CONSTRAINT copy_decisions_status_check
    CHECK (status IN ('approved', 'rejected', 'skipped', 'awaiting_liquidity'));