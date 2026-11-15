-- Per-strategy execution mode (paper vs live). Existing rows default to paper.
ALTER TABLE copy_strategies
    ADD COLUMN IF NOT EXISTS operating_mode text NOT NULL DEFAULT 'paper_trading'
        CHECK (operating_mode IN ('paper_trading', 'live_trading'));

COMMENT ON COLUMN copy_strategies.operating_mode IS
    'Execution mode for this copy strategy; independent of sibling strategies on the same trading account.';
