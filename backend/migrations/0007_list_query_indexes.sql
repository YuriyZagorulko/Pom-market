CREATE INDEX IF NOT EXISTS timeline_events_user_at_id ON timeline_events (user_id, at DESC, id DESC);
CREATE INDEX IF NOT EXISTS orders_user_created_id ON orders (user_id, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS fills_user_filled_id ON fills (user_id, filled_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS positions_user_updated_id ON positions (user_id, updated_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS source_events_ts_id ON source_events (event_ts DESC, id DESC);
