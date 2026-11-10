ALTER TABLE source_events ADD COLUMN IF NOT EXISTS market_icon text;
ALTER TABLE source_events ADD COLUMN IF NOT EXISTS event_slug text;
ALTER TABLE source_events ADD COLUMN IF NOT EXISTS activity_type text NOT NULL DEFAULT 'trade';
ALTER TABLE source_events ADD COLUMN IF NOT EXISTS usdc_size numeric(24, 8);

UPDATE source_events
SET market_icon = NULLIF(TRIM(BOTH FROM raw->>'icon'), '')
WHERE market_icon IS NULL
  AND raw ? 'icon'
  AND NULLIF(TRIM(BOTH FROM raw->>'icon'), '') IS NOT NULL;

UPDATE source_events
SET event_slug = COALESCE(NULLIF(TRIM(BOTH FROM raw->>'event_slug'), ''), NULLIF(TRIM(BOTH FROM raw->>'eventSlug'), ''))
WHERE event_slug IS NULL
  AND (raw ? 'event_slug' OR raw ? 'eventSlug');

UPDATE source_events
SET usdc_size = (raw->>'usdc_size')::numeric
WHERE usdc_size IS NULL
  AND raw ? 'usdc_size'
  AND jsonb_typeof(raw->'usdc_size') = 'number';

UPDATE source_events
SET usdc_size = (raw->>'usdcSize')::numeric
WHERE usdc_size IS NULL
  AND raw ? 'usdcSize'
  AND jsonb_typeof(raw->'usdcSize') = 'number';

UPDATE source_events
SET activity_type = lower(TRIM(BOTH FROM raw->>'type'))
WHERE raw ? 'type'
  AND NULLIF(TRIM(BOTH FROM raw->>'type'), '') IS NOT NULL
  AND activity_type = 'trade';
