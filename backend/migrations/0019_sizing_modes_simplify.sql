-- Simplify user-facing sizing: multiplier uses the same math as proportional (source_notional × coefficient).

UPDATE copy_strategies
SET sizing_mode = 'proportional',
    proportion = COALESCE(proportion, multiplier),
    multiplier = NULL
WHERE sizing_mode = 'multiplier';

COMMENT ON COLUMN copy_strategies.sizing_mode IS
    'User modes: position_value_ratio, fixed_amount, proportional. Legacy portfolio_ratio remains valid for existing rows; new strategies should use position_value_ratio.';
