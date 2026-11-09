-- Idempotent correction of the two paper fills that used base_fee=1000 as a multiplier.
-- The market schedule, read from Gamma on 2026-11-09, is crypto_fees_v2:
-- feesEnabled=true, rate=0.07, exponent=1, takerOnly=true, rebateRate=0.20.
-- fee = shares * 0.07 * price * (1 - price), rounded to 5 decimal places.
-- Opening paper cash was 1000 for Paper A and 40 for Paper B.
-- The later assignment of paper_cash=5000 was not a ledger entry and is reversed here.

BEGIN;

INSERT INTO audit_log (action, target, metadata)
SELECT
    'paper_fee_correction',
    f.id::text,
    jsonb_build_object(
        'model', 'polymarket-fee-v1',
        'fee_schedule', jsonb_build_object('feesEnabled', true, 'rate', '0.07', 'exponent', 1, 'takerOnly', true, 'rebateRate', '0.20', 'source', 'gamma.feeSchedule', 'feeType', 'crypto_fees_v2'),
        'previous_fee_usd', f.fee_usd,
        'corrected_fee_usd', CASE f.id::text
            WHEN '7476c1e6-b20f-4a18-bc0e-70dfb1aee760' THEN 0.14
            WHEN '3dfef1ba-8918-4ead-8967-c8fda0a935bf' THEN 0.196
        END,
        'previous_paper_cash_usd', r.paper_cash_usd,
        'opening_paper_cash_usd', CASE f.trading_account_id::text
            WHEN 'ff1191a4-fad9-4013-96db-a64ea8fce284' THEN 1000
            WHEN '897b7a64-748f-48b0-92d7-a5728616bbd2' THEN 40
        END,
        'note', 'base_fee 1000 was applied as a multiplier; 5000 cash reset is reversed'
    )
FROM fills f
JOIN account_risk_limits r ON r.trading_account_id = f.trading_account_id
WHERE f.id IN (
    '7476c1e6-b20f-4a18-bc0e-70dfb1aee760',
    '3dfef1ba-8918-4ead-8967-c8fda0a935bf'
)
AND f.fee_usd > 100
AND NOT EXISTS (
    SELECT 1 FROM audit_log a WHERE a.action = 'paper_fee_correction' AND a.target = f.id::text
);

UPDATE fills
SET fee_usd = 0.14000,
    fee_status = 'estimated',
    fee_model = 'polymarket-fee-v1',
    fee_schedule = '{"feesEnabled":true,"rate":"0.07","exponent":1,"takerOnly":true,"rebateRate":"0.20","feeType":"crypto_fees_v2"}'::jsonb
WHERE id = '7476c1e6-b20f-4a18-bc0e-70dfb1aee760' AND fee_usd > 100;

UPDATE fills
SET fee_usd = 0.19600,
    fee_status = 'estimated',
    fee_model = 'polymarket-fee-v1',
    fee_schedule = '{"feesEnabled":true,"rate":"0.07","exponent":1,"takerOnly":true,"rebateRate":"0.20","feeType":"crypto_fees_v2"}'::jsonb
WHERE id = '3dfef1ba-8918-4ead-8967-c8fda0a935bf' AND fee_usd > 100;

UPDATE positions
SET cost_basis_usd = 10.14000,
    avg_price = 0.81120000
WHERE trading_account_id = 'ff1191a4-fad9-4013-96db-a64ea8fce284'
  AND open_shares = 12.5
  AND cost_basis_usd > 100;

UPDATE positions
SET cost_basis_usd = 14.19600,
    avg_price = 0.81120000
WHERE trading_account_id = '897b7a64-748f-48b0-92d7-a5728616bbd2'
  AND open_shares = 17.5
  AND cost_basis_usd > 100;

UPDATE account_risk_limits
SET paper_cash_usd = 989.86000,
    updated_at = now()
WHERE trading_account_id = 'ff1191a4-fad9-4013-96db-a64ea8fce284'
  AND paper_cash_usd = 5000
  AND EXISTS (
    SELECT 1 FROM audit_log WHERE action = 'paper_fee_correction' AND target = '7476c1e6-b20f-4a18-bc0e-70dfb1aee760'
  );

UPDATE account_risk_limits
SET paper_cash_usd = 25.80400,
    updated_at = now()
WHERE trading_account_id = '897b7a64-748f-48b0-92d7-a5728616bbd2'
  AND paper_cash_usd = 5000
  AND EXISTS (
    SELECT 1 FROM audit_log WHERE action = 'paper_fee_correction' AND target = '3dfef1ba-8918-4ead-8967-c8fda0a935bf'
  );

COMMIT;
