-- Portfolio-relative copy sizing and per-strategy capital allocation.

ALTER TABLE copy_strategies DROP CONSTRAINT IF EXISTS copy_strategies_sizing_mode_check;
ALTER TABLE copy_strategies ADD CONSTRAINT copy_strategies_sizing_mode_check
    CHECK (sizing_mode IN ('fixed_amount', 'proportional', 'multiplier', 'portfolio_ratio'));

ALTER TABLE copy_strategies
    ADD COLUMN IF NOT EXISTS allocation_mode text NOT NULL DEFAULT 'fixed_usd'
        CHECK (allocation_mode IN ('fixed_usd', 'percent_of_account')),
    ADD COLUMN IF NOT EXISTS allocation_percent numeric(24, 8);

COMMENT ON COLUMN copy_strategies.allocation_mode IS
    'fixed_usd: strategy_budget_usd is the cap; percent_of_account: cap scales with paper/live account equity';
COMMENT ON COLUMN copy_strategies.allocation_percent IS
    'When allocation_mode=percent_of_account, percent of account equity (0–100) assigned to this strategy';

CREATE TABLE source_trader_equity_snapshots (
    source_trader_id uuid PRIMARY KEY REFERENCES source_traders(id) ON DELETE CASCADE,
    wallet_queried text NOT NULL,
    positions_value_usd numeric(24, 8),
    combo_positions_value_usd numeric(24, 8),
    wallet_collateral_usd numeric(24, 8),
    total_equity_usd numeric(24, 8),
    status text NOT NULL CHECK (status IN ('ok', 'stale', 'unavailable', 'partial')),
    quality_note text,
    observed_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    raw jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX source_trader_equity_expires ON source_trader_equity_snapshots (expires_at);

ALTER TABLE source_trader_equity_snapshots ENABLE ROW LEVEL SECURITY;
ALTER TABLE source_trader_equity_snapshots FORCE ROW LEVEL SECURITY;

CREATE POLICY source_trader_equity_snapshots_all ON source_trader_equity_snapshots FOR ALL
    USING (
        app_is_worker()
        OR EXISTS (
            SELECT 1 FROM source_traders t
            WHERE t.id = source_trader_equity_snapshots.source_trader_id
              AND t.user_id = app_user_id()
        )
    )
    WITH CHECK (
        app_is_worker()
        OR EXISTS (
            SELECT 1 FROM source_traders t
            WHERE t.id = source_trader_equity_snapshots.source_trader_id
              AND t.user_id = app_user_id()
        )
    );

GRANT SELECT, INSERT, UPDATE, DELETE ON source_trader_equity_snapshots TO pommarket_app, pommarket_worker;
