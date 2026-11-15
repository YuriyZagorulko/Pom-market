-- READ-ONLY: estimate account funding from ledger (does not UPDATE anything).
-- Use to compare with account_risk_limits.paper_cash_usd after the buy-debit semantics change.
-- Restores: SUM(account_paper_funding) + opening balance from account creation (paper_cash at insert if never ledger-funded).

\echo 'Per-account ledger funding vs stored paper_cash_usd'
SELECT
    a.id AS trading_account_id,
    a.name,
    r.paper_cash_usd AS stored_paper_cash,
    COALESCE(SUM(cle.amount_usd) FILTER (WHERE cle.entry_type = 'account_paper_funding'), 0) AS ledger_funding_sum,
    r.paper_cash_usd
        - COALESCE(SUM(cle.amount_usd) FILTER (WHERE cle.entry_type = 'account_paper_funding'), 0)
        AS gap_vs_ledger_only,
    COALESCE(SUM(cle.amount_usd) FILTER (WHERE cle.entry_type = 'buy_fill'), 0) AS ledger_buy_fill_sum
FROM trading_accounts a
JOIN account_risk_limits r ON r.trading_account_id = a.id
LEFT JOIN capital_ledger_entries cle ON cle.trading_account_id = a.id
WHERE a.operating_mode = 'paper_trading'
GROUP BY a.id, a.name, r.paper_cash_usd
ORDER BY ABS(
    r.paper_cash_usd
    - COALESCE(SUM(cle.amount_usd) FILTER (WHERE cle.entry_type = 'account_paper_funding'), 0)
) DESC;