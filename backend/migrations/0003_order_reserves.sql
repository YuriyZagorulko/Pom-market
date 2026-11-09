ALTER TABLE orders ADD COLUMN IF NOT EXISTS position_reserved_usd numeric(24, 8) NOT NULL DEFAULT 0;
ALTER TABLE orders ADD COLUMN IF NOT EXISTS fee_reserved_usd numeric(24, 8) NOT NULL DEFAULT 0;

GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO pommarket_app, pommarket_worker;
