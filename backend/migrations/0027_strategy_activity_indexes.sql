-- Strategy activity feed (decisions + ledger settlements): list by strategy and time.

CREATE INDEX IF NOT EXISTS copy_decisions_strategy_decided_at
    ON copy_decisions (strategy_id, decided_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS capital_ledger_strategy_created_at
    ON capital_ledger_entries (strategy_id, created_at DESC, id DESC)
    WHERE strategy_id IS NOT NULL;
