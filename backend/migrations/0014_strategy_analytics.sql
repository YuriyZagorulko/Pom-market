-- Strategy-level analytics: snapshots + closed position attribution.

ALTER TABLE closed_positions
    ADD COLUMN IF NOT EXISTS strategy_id uuid REFERENCES copy_strategies(id) ON DELETE SET NULL;

UPDATE closed_positions cp
SET strategy_id = s.id
FROM copy_strategies s
JOIN source_traders st ON st.id = s.source_trader_id
WHERE cp.strategy_id IS NULL
  AND cp.trading_account_id = s.trading_account_id
  AND lower(cp.source_wallet) = lower(st.wallet_address);

CREATE INDEX IF NOT EXISTS closed_positions_strategy_closed_at
    ON closed_positions (strategy_id, closed_at DESC)
    WHERE strategy_id IS NOT NULL;

CREATE TABLE strategy_analytics_snapshots (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    trading_account_id uuid NOT NULL REFERENCES trading_accounts(id) ON DELETE CASCADE,
    strategy_id uuid NOT NULL REFERENCES copy_strategies(id) ON DELETE CASCADE,
    bucket_at timestamptz NOT NULL,
    allocated_capital_usd numeric(24, 8) NOT NULL,
    deployed_capital_usd numeric(24, 8) NOT NULL,
    reserved_capital_usd numeric(24, 8) NOT NULL,
    positions_market_value_usd numeric(24, 8) NOT NULL,
    realized_pnl_usd numeric(24, 8) NOT NULL,
    unrealized_pnl_usd numeric(24, 8) NOT NULL,
    fees_usd numeric(24, 8) NOT NULL,
    equity_metric_usd numeric(24, 8) NOT NULL,
    data_quality text NOT NULL DEFAULT 'partial',
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (strategy_id, bucket_at)
);

CREATE INDEX strategy_analytics_snapshots_strategy_bucket
    ON strategy_analytics_snapshots (strategy_id, bucket_at DESC);

ALTER TABLE strategy_analytics_snapshots ENABLE ROW LEVEL SECURITY;
ALTER TABLE strategy_analytics_snapshots FORCE ROW LEVEL SECURITY;

CREATE POLICY strategy_analytics_snapshots_all ON strategy_analytics_snapshots
    FOR ALL
    USING (app_owns(user_id))
    WITH CHECK (app_owns(user_id));
