-- Separate experimental synthetic PAPER fills from market-realistic ledger updates.

ALTER TABLE fills ADD COLUMN IF NOT EXISTS contributes_to_paper_ledger boolean NOT NULL DEFAULT true;

COMMENT ON COLUMN fills.contributes_to_paper_ledger IS
    'When false, fill is audit-only (e.g. synthetic_source_price); does not drive strategy capital or positions.';
