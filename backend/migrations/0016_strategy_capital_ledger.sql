-- Independent strategy virtual cash (PAPER): ledger + balance rows.

CREATE TABLE strategy_capital_accounts (
    strategy_id uuid PRIMARY KEY REFERENCES copy_strategies(id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    trading_account_id uuid NOT NULL REFERENCES trading_accounts(id) ON DELETE CASCADE,
    funded_capital_usd numeric(24, 8) NOT NULL DEFAULT 0,
    available_cash_usd numeric(24, 8) NOT NULL DEFAULT 0,
    reserved_cash_usd numeric(24, 8) NOT NULL DEFAULT 0,
    invested_capital_usd numeric(24, 8) NOT NULL DEFAULT 0,
    bootstrap_quality text NOT NULL DEFAULT 'pending'
        CHECK (bootstrap_quality IN ('pending', 'migrated', 'estimated', 'needs_review')),
    bootstrap_note text,
    initialized_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT strategy_capital_available_nonneg CHECK (available_cash_usd >= 0),
    CONSTRAINT strategy_capital_reserved_nonneg CHECK (reserved_cash_usd >= 0),
    CONSTRAINT strategy_capital_funded_nonneg CHECK (funded_capital_usd >= 0),
    CONSTRAINT strategy_capital_invested_nonneg CHECK (invested_capital_usd >= 0)
);

CREATE INDEX strategy_capital_accounts_account ON strategy_capital_accounts (trading_account_id);

CREATE TABLE capital_ledger_entries (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    trading_account_id uuid NOT NULL REFERENCES trading_accounts(id) ON DELETE CASCADE,
    strategy_id uuid REFERENCES copy_strategies(id) ON DELETE SET NULL,
    order_id uuid REFERENCES orders(id) ON DELETE SET NULL,
    fill_id uuid REFERENCES fills(id) ON DELETE SET NULL,
    position_id uuid REFERENCES positions(id) ON DELETE SET NULL,
    entry_type text NOT NULL CHECK (entry_type IN (
        'account_paper_funding',
        'strategy_allocation',
        'strategy_withdrawal',
        'order_reserve',
        'reserve_release',
        'buy_fill',
        'sell_fill',
        'trading_fee',
        'order_cancelled',
        'hard_close_sell',
        'reconciliation_adjustment'
    )),
    amount_usd numeric(24, 8) NOT NULL,
    idempotency_key text NOT NULL UNIQUE,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX capital_ledger_strategy_created ON capital_ledger_entries (strategy_id, created_at DESC);
CREATE INDEX capital_ledger_account_created ON capital_ledger_entries (trading_account_id, created_at DESC);

ALTER TABLE strategy_capital_accounts ENABLE ROW LEVEL SECURITY;
ALTER TABLE strategy_capital_accounts FORCE ROW LEVEL SECURITY;
ALTER TABLE capital_ledger_entries ENABLE ROW LEVEL SECURITY;
ALTER TABLE capital_ledger_entries FORCE ROW LEVEL SECURITY;

CREATE POLICY strategy_capital_accounts_all ON strategy_capital_accounts
    FOR ALL
    USING (app_owns(user_id))
    WITH CHECK (app_owns(user_id));

CREATE POLICY capital_ledger_entries_all ON capital_ledger_entries
    FOR ALL
    USING (app_owns(user_id))
    WITH CHECK (app_owns(user_id));

GRANT SELECT, INSERT, UPDATE, DELETE ON strategy_capital_accounts, capital_ledger_entries
    TO pommarket_app, pommarket_worker;
