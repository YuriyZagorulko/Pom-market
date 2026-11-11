-- Label PAPER fills as CLOB walk vs synthetic source-price simulation.

ALTER TABLE fills ADD COLUMN IF NOT EXISTS paper_execution_mode text;
ALTER TABLE fills ADD COLUMN IF NOT EXISTS paper_execution_context jsonb NOT NULL DEFAULT '{}';

ALTER TABLE fills DROP CONSTRAINT IF EXISTS fills_paper_execution_mode_check;
ALTER TABLE fills
    ADD CONSTRAINT fills_paper_execution_mode_check
    CHECK (
        paper_execution_mode IS NULL
        OR paper_execution_mode = ANY (ARRAY['clob'::text, 'synthetic_source_price'::text])
    );
