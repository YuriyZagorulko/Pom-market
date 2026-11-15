-- Baseline for incremental realized P&L per close cycle (positions.realized_pnl_usd is cumulative).

ALTER TABLE positions
    ADD COLUMN IF NOT EXISTS close_cycle_realized_baseline numeric(24, 8) NOT NULL DEFAULT 0;
