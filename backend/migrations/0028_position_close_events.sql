-- Immutable position lifecycle close snapshots for Activity (display only; P&L already in positions/ledger).

CREATE TABLE position_close_events (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    trading_account_id uuid NOT NULL REFERENCES trading_accounts(id) ON DELETE CASCADE,
    strategy_id uuid NOT NULL REFERENCES copy_strategies(id) ON DELETE CASCADE,
    position_id uuid NOT NULL REFERENCES positions(id) ON DELETE CASCADE,
    token_id text NOT NULL,
    market_title text,
    outcome text,
    close_reason text NOT NULL CHECK (close_reason IN (
        'leader_sell', 'hard_close', 'resolution_win', 'resolution_loss', 'resolution_refund'
    )),
    closed_shares numeric(24, 8) NOT NULL,
    avg_entry_price numeric(24, 8),
    avg_exit_price numeric(24, 8),
    total_cost_basis_usd numeric(24, 8) NOT NULL,
    total_proceeds_usd numeric(24, 8) NOT NULL,
    total_fees_usd numeric(24, 8) NOT NULL DEFAULT 0,
    realized_pnl_usd numeric(24, 8) NOT NULL,
    idempotency_key text NOT NULL UNIQUE,
    closed_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX position_close_events_strategy_closed_at
    ON position_close_events (strategy_id, closed_at DESC);

ALTER TABLE position_close_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE position_close_events FORCE ROW LEVEL SECURITY;

CREATE POLICY position_close_events_all ON position_close_events
    FOR ALL
    USING (app_owns(user_id))
    WITH CHECK (app_owns(user_id));
