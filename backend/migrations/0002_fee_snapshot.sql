ALTER TABLE fills ADD COLUMN IF NOT EXISTS fee_model text;
ALTER TABLE fills ADD COLUMN IF NOT EXISTS fee_schedule jsonb;

GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO pommarket_app, pommarket_worker;
