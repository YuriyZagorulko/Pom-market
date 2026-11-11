-- Position Value Ratio sizing: scale copy BUY by leader open-position MTM share.

ALTER TABLE copy_strategies DROP CONSTRAINT IF EXISTS copy_strategies_sizing_mode_check;
ALTER TABLE copy_strategies ADD CONSTRAINT copy_strategies_sizing_mode_check
    CHECK (sizing_mode IN (
        'fixed_amount',
        'proportional',
        'multiplier',
        'portfolio_ratio',
        'position_value_ratio'
    ));

COMMENT ON COLUMN copy_strategies.sizing_mode IS
    'position_value_ratio: copy BUY notional = strategy allocation × (source BUY / leader open-position MTM from Data API /v2/value); no wallet USDC required.';
