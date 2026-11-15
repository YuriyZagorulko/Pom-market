-- PAPER market resolution settlement ledger types.

ALTER TABLE capital_ledger_entries
    DROP CONSTRAINT IF EXISTS capital_ledger_entries_entry_type_check;

ALTER TABLE capital_ledger_entries
    ADD CONSTRAINT capital_ledger_entries_entry_type_check CHECK (entry_type IN (
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
        'reconciliation_adjustment',
        'resolution_redeem',
        'resolution_loss',
        'resolution_refund'
    ));
