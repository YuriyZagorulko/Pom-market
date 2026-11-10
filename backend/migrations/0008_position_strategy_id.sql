-- Tie open positions to a specific copy strategy so SELL cannot close unrelated lots.

ALTER TABLE positions ADD COLUMN strategy_id uuid REFERENCES copy_strategies(id) ON DELETE CASCADE;

UPDATE positions p
SET strategy_id = s.id
FROM copy_strategies s
JOIN source_traders st ON st.id = s.source_trader_id
WHERE p.strategy_id IS NULL
  AND p.trading_account_id = s.trading_account_id
  AND lower(p.source_wallet) = lower(st.wallet_address);

DELETE FROM positions WHERE strategy_id IS NULL;

ALTER TABLE positions ALTER COLUMN strategy_id SET NOT NULL;

DO $$
DECLARE cname text;
BEGIN
  SELECT conname INTO cname
  FROM pg_constraint
  WHERE conrelid = 'public.positions'::regclass
    AND contype = 'u'
    AND pg_get_constraintdef(oid) LIKE '%trading_account_id%source_wallet%token_id%operating_mode%';
  IF cname IS NOT NULL THEN
    EXECUTE format('ALTER TABLE positions DROP CONSTRAINT %I', cname);
  END IF;
END $$;

CREATE UNIQUE INDEX positions_strategy_token_mode_key ON positions (strategy_id, token_id, operating_mode);
CREATE INDEX positions_strategy_open ON positions (strategy_id) WHERE open_shares > 0;
