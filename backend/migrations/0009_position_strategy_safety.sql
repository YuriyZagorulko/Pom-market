-- Correct 0008 gaps: keep orphan/manual lots, enforce partial uniqueness.
-- 0008 assigned copy rows via (trading_account_id, source_wallet); orphans were deleted.
-- After 0009, strategy_id is required for copy pipeline rows but nullable for future manual lots.

ALTER TABLE positions ALTER COLUMN strategy_id DROP NOT NULL;

DROP INDEX IF EXISTS positions_strategy_token_mode_key;

CREATE UNIQUE INDEX positions_strategy_token_mode_key
  ON positions (strategy_id, token_id, operating_mode)
  WHERE strategy_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS positions_manual_lot_key
  ON positions (trading_account_id, token_id, operating_mode, lower(source_wallet))
  WHERE strategy_id IS NULL;
