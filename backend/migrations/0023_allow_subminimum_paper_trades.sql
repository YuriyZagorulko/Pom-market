-- Per-strategy PAPER-only: model copy trades below Polymarket exchange minimums.

ALTER TABLE copy_strategies
    ADD COLUMN IF NOT EXISTS allow_subminimum_paper_trades boolean NOT NULL DEFAULT false;

COMMENT ON COLUMN copy_strategies.allow_subminimum_paper_trades IS
    'When true and account is paper_trading: skip exchange $1 / market min-shares rejection for copy sizing and fill simulation. LIVE unaffected.';
