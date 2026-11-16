-- Strategy capital isolation invariants (forward-only; NOT VALID avoids failing legacy drift).

ALTER TABLE strategy_capital_accounts
    ADD CONSTRAINT strategy_capital_funded_identity
    CHECK (
        abs(
            funded_capital_usd
            - (available_cash_usd + reserved_cash_usd + invested_capital_usd)
        ) <= 0.01
    ) NOT VALID;

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

ALTER TABLE capital_ledger_entries
    ADD CONSTRAINT capital_ledger_strategy_owned
    CHECK (
        entry_type IN ('account_paper_funding', 'reconciliation_adjustment')
        OR strategy_id IS NOT NULL
    ) NOT VALID;