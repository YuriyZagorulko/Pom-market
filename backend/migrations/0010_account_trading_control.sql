-- Per-account trading control (danger zone) and hard-close operations.

ALTER TABLE trading_accounts
    ADD COLUMN trading_control_state text NOT NULL DEFAULT 'normal'
        CHECK (trading_control_state IN (
            'normal',
            'kill_switch',
            'soft_close',
            'hard_close_in_progress',
            'hard_close_completed',
            'hard_close_partial'
        ));

UPDATE trading_accounts
SET trading_control_state = 'kill_switch'
WHERE emergency_stop = true;

ALTER TABLE orders
    ALTER COLUMN decision_id DROP NOT NULL,
    ALTER COLUMN source_event_id DROP NOT NULL;

ALTER TABLE orders
    ADD COLUMN origin text NOT NULL DEFAULT 'copy'
        CHECK (origin IN ('copy', 'hard_close'));

ALTER TABLE orders
    ADD COLUMN position_id uuid REFERENCES positions(id) ON DELETE SET NULL;

CREATE TABLE hard_close_operations (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    trading_account_id uuid NOT NULL REFERENCES trading_accounts(id) ON DELETE CASCADE,
    idempotency_key text,
    status text NOT NULL DEFAULT 'queued' CHECK (status IN (
        'queued', 'in_progress', 'completed', 'partial', 'failed', 'paused'
    )),
    error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    started_at timestamptz,
    completed_at timestamptz
);

CREATE UNIQUE INDEX hard_close_idempotency_key
    ON hard_close_operations (trading_account_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE UNIQUE INDEX hard_close_one_active
    ON hard_close_operations (trading_account_id)
    WHERE status IN ('queued', 'in_progress');

CREATE TABLE hard_close_items (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    operation_id uuid NOT NULL REFERENCES hard_close_operations(id) ON DELETE CASCADE,
    position_id uuid NOT NULL REFERENCES positions(id) ON DELETE CASCADE,
    token_id text NOT NULL,
    market_title text,
    status text NOT NULL DEFAULT 'pending' CHECK (status IN (
        'pending', 'in_progress', 'closed', 'partial', 'failed', 'skipped'
    )),
    requested_shares numeric(24, 8) NOT NULL,
    closed_shares numeric(24, 8) NOT NULL DEFAULT 0,
    remaining_shares numeric(24, 8) NOT NULL DEFAULT 0,
    order_id uuid REFERENCES orders(id) ON DELETE SET NULL,
    error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (operation_id, position_id)
);

CREATE INDEX hard_close_items_operation ON hard_close_items (operation_id);
CREATE INDEX hard_close_ops_account ON hard_close_operations (trading_account_id, created_at DESC);

ALTER TABLE hard_close_operations ENABLE ROW LEVEL SECURITY;
ALTER TABLE hard_close_operations FORCE ROW LEVEL SECURITY;
ALTER TABLE hard_close_items ENABLE ROW LEVEL SECURITY;
ALTER TABLE hard_close_items FORCE ROW LEVEL SECURITY;

CREATE POLICY hard_close_ops_all ON hard_close_operations FOR ALL
    USING (user_id = current_setting('app.user_id', true)::uuid)
    WITH CHECK (user_id = current_setting('app.user_id', true)::uuid);

CREATE POLICY hard_close_items_all ON hard_close_items FOR ALL
    USING (
        EXISTS (
            SELECT 1 FROM hard_close_operations o
            WHERE o.id = hard_close_items.operation_id
              AND o.user_id = current_setting('app.user_id', true)::uuid
        )
    )
    WITH CHECK (
        EXISTS (
            SELECT 1 FROM hard_close_operations o
            WHERE o.id = hard_close_items.operation_id
              AND o.user_id = current_setting('app.user_id', true)::uuid
        )
    );

CREATE POLICY hard_close_ops_worker ON hard_close_operations FOR ALL
    TO pommarket_worker
    USING (true)
    WITH CHECK (true);

CREATE POLICY hard_close_items_worker ON hard_close_items FOR ALL
    TO pommarket_worker
    USING (true)
    WITH CHECK (true);

GRANT SELECT, INSERT, UPDATE, DELETE ON hard_close_operations, hard_close_items TO pommarket_app, pommarket_worker;
