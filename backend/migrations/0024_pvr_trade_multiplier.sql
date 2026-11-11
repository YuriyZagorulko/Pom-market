-- PVR trade multiplier reuses copy_strategies.multiplier (NULL = 1×). Legacy sizing_mode=multiplier is retired.

COMMENT ON COLUMN copy_strategies.multiplier IS
    'When sizing_mode is position_value_ratio: extra coefficient applied after PVR base (NULL = 1). Not used for proportional/fixed_amount.';
