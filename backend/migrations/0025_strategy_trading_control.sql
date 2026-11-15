-- Per-strategy trading control (danger zone) and strategy-scoped hard close.

ALTER TABLE copy_strategies
    ADD COLUMN trading_control_state text NOT NULL DEFAULT 'normal'
        CHECK (trading_control_state IN (
            'normal',
            'kill_switch',
            'soft_close',
            'hard_close_in_progress',
            'hard_close_completed',
            'hard_close_partial'
        ));

ALTER TABLE hard_close_operations
    ADD COLUMN strategy_id uuid REFERENCES copy_strategies(id) ON DELETE CASCADE;

DROP INDEX IF EXISTS hard_close_one_active;

CREATE UNIQUE INDEX hard_close_one_active_account
    ON hard_close_operations (trading_account_id)
    WHERE strategy_id IS NULL AND status IN ('queued', 'in_progress');

CREATE UNIQUE INDEX hard_close_one_active_strategy
    ON hard_close_operations (strategy_id)
    WHERE strategy_id IS NOT NULL AND status IN ('queued', 'in_progress');

CREATE INDEX hard_close_ops_strategy ON hard_close_operations (strategy_id, created_at DESC)
    WHERE strategy_id IS NOT NULL;
