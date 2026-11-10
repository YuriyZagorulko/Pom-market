ALTER TABLE copy_strategies
    ADD COLUMN IF NOT EXISTS display_name text;

COMMENT ON COLUMN copy_strategies.display_name IS
    'Optional user label for this strategy; API falls back to source trader alias when null.';
