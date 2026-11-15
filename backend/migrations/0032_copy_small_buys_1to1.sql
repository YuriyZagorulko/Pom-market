-- PVR-only: copy leader BUY notional 1:1 when below a USD threshold.

ALTER TABLE copy_strategies
    ADD COLUMN IF NOT EXISTS copy_small_buys_1to1 boolean NOT NULL DEFAULT false,
    ADD COLUMN IF NOT EXISTS small_buy_threshold_usd numeric(20, 8) NULL;

COMMENT ON COLUMN copy_strategies.copy_small_buys_1to1 IS
    'When sizing_mode is position_value_ratio: copy leader BUY USD notional 1:1 if strictly below small_buy_threshold_usd.';
COMMENT ON COLUMN copy_strategies.small_buy_threshold_usd IS
    'USD threshold for copy_small_buys_1to1; required when copy_small_buys_1to1 is true.';
