-- Skip broken multiplier rows with no coefficient (idempotent after 0019).

UPDATE copy_strategies
SET sizing_mode = 'proportional',
    proportion = COALESCE(proportion, multiplier),
    multiplier = NULL
WHERE sizing_mode = 'multiplier'
  AND COALESCE(proportion, multiplier) IS NOT NULL;
