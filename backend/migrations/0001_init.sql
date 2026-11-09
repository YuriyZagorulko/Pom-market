-- Linear migration 0001: schema, roles, and row level security.
-- Tables are owned by the migration role. Application roles are not owners and cannot bypass RLS.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pommarket_app') THEN
        CREATE ROLE pommarket_app NOLOGIN NOSUPERUSER NOBYPASSRLS NOCREATEDB;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pommarket_worker') THEN
        CREATE ROLE pommarket_worker NOLOGIN NOSUPERUSER NOBYPASSRLS NOCREATEDB;
    END IF;
END
$$;

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE OR REPLACE FUNCTION app_role() RETURNS text
LANGUAGE sql STABLE AS $$
    SELECT COALESCE(NULLIF(current_setting('app.role', true), ''), '')
$$;

CREATE OR REPLACE FUNCTION app_user_id() RETURNS uuid
LANGUAGE sql STABLE AS $$
    SELECT NULLIF(current_setting('app.user_id', true), '')::uuid
$$;

CREATE OR REPLACE FUNCTION app_is_worker() RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT app_role() = 'worker'
$$;

CREATE OR REPLACE FUNCTION app_is_admin() RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT app_role() = 'admin'
$$;

CREATE OR REPLACE FUNCTION app_owns(row_user uuid) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT app_is_worker() OR row_user = app_user_id()
$$;

CREATE TABLE users (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    email text NOT NULL UNIQUE,
    display_name text NOT NULL,
    password_hash text,
    role text NOT NULL CHECK (role IN ('admin', 'user')),
    status text NOT NULL CHECK (status IN ('invited', 'active', 'suspended')),
    failed_login_count integer NOT NULL DEFAULT 0,
    locked_until timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    last_login_at timestamptz
);

CREATE TABLE sessions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    refresh_token_hash text NOT NULL UNIQUE,
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    user_agent text,
    ip inet
);

CREATE TABLE invitation_tokens (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash text NOT NULL UNIQUE,
    purpose text NOT NULL CHECK (purpose IN ('invite', 'reset')),
    expires_at timestamptz NOT NULL,
    used_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE trading_accounts (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name text NOT NULL,
    signature_type text NOT NULL CHECK (signature_type IN ('eoa', 'poly_proxy', 'poly_gnosis_safe', 'poly_1271')),
    signer_address text NOT NULL,
    funder_address text NOT NULL,
    operating_mode text NOT NULL DEFAULT 'monitoring' CHECK (operating_mode IN ('monitoring', 'paper_trading', 'live_trading')),
    status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'connected', 'error', 'disconnected')),
    emergency_stop boolean NOT NULL DEFAULT false,
    live_confirmed_at timestamptz,
    last_exchange_balance_usd numeric(24, 8),
    last_balance_checked_at timestamptz,
    last_connection_error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE trading_credentials (
    trading_account_id uuid PRIMARY KEY REFERENCES trading_accounts(id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    ciphertext bytea NOT NULL,
    nonce bytea NOT NULL,
    key_id text NOT NULL,
    aad text NOT NULL,
    api_key_hint text,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE account_risk_limits (
    trading_account_id uuid PRIMARY KEY REFERENCES trading_accounts(id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    max_order_usd numeric(24, 8) NOT NULL,
    max_daily_loss_usd numeric(24, 8) NOT NULL,
    max_exposure_usd numeric(24, 8) NOT NULL,
    max_slippage_bps integer NOT NULL,
    max_orders_per_interval integer NOT NULL,
    interval_seconds integer NOT NULL,
    max_stale_data_seconds integer NOT NULL,
    paper_cash_usd numeric(24, 8) NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE source_traders (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    wallet_address text NOT NULL,
    alias text NOT NULL,
    monitoring_enabled boolean NOT NULL DEFAULT true,
    profile jsonb NOT NULL DEFAULT '{}'::jsonb,
    last_trade_at timestamptz,
    observed_trade_count integer NOT NULL DEFAULT 0,
    copied_trade_count integer NOT NULL DEFAULT 0,
    rejected_trade_count integer NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (user_id, wallet_address)
);

CREATE TABLE copy_strategies (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    trading_account_id uuid NOT NULL REFERENCES trading_accounts(id) ON DELETE CASCADE,
    source_trader_id uuid NOT NULL REFERENCES source_traders(id) ON DELETE CASCADE,
    enabled boolean NOT NULL DEFAULT false,
    sizing_mode text NOT NULL CHECK (sizing_mode IN ('fixed_amount', 'proportional', 'multiplier')),
    fixed_amount_usd numeric(24, 8),
    proportion numeric(24, 8),
    multiplier numeric(24, 8),
    strategy_budget_usd numeric(24, 8) NOT NULL,
    max_position_usd numeric(24, 8) NOT NULL,
    max_exposure_usd numeric(24, 8) NOT NULL,
    max_open_positions integer NOT NULL,
    max_slippage_bps integer NOT NULL,
    min_order_usd numeric(24, 8) NOT NULL,
    copy_buy boolean NOT NULL DEFAULT true,
    copy_sell boolean NOT NULL DEFAULT true,
    cooldown_seconds integer NOT NULL DEFAULT 0,
    allowed_markets jsonb NOT NULL DEFAULT '[]'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (trading_account_id, source_trader_id)
);

CREATE TABLE source_events (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    dedup_key text NOT NULL UNIQUE,
    source_wallet text NOT NULL,
    tx_hash text,
    condition_id text,
    token_id text NOT NULL,
    outcome text,
    market_title text,
    market_slug text,
    event_title text,
    side text NOT NULL CHECK (side IN ('buy', 'sell')),
    price numeric(24, 8) NOT NULL,
    size_shares numeric(24, 8) NOT NULL,
    notional_usd numeric(24, 8) NOT NULL,
    event_ts timestamptz NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now(),
    origin text NOT NULL,
    status text NOT NULL CHECK (status IN ('detected', 'normalized', 'duplicate', 'processing', 'processed', 'failed')),
    raw jsonb NOT NULL,
    fragment_count integer NOT NULL DEFAULT 1,
    processed_at timestamptz,
    error text
);

CREATE INDEX source_events_wallet_ts ON source_events (source_wallet, event_ts DESC);

CREATE TABLE source_event_fragments (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    source_event_id uuid NOT NULL REFERENCES source_events(id) ON DELETE CASCADE,
    fragment_key text NOT NULL UNIQUE,
    origin text NOT NULL,
    size_shares numeric(24, 8) NOT NULL,
    price numeric(24, 8) NOT NULL,
    raw jsonb NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE copy_decisions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    trading_account_id uuid NOT NULL REFERENCES trading_accounts(id) ON DELETE CASCADE,
    source_trader_id uuid NOT NULL REFERENCES source_traders(id) ON DELETE CASCADE,
    source_event_id uuid NOT NULL REFERENCES source_events(id) ON DELETE CASCADE,
    strategy_id uuid NOT NULL REFERENCES copy_strategies(id) ON DELETE CASCADE,
    status text NOT NULL CHECK (status IN ('approved', 'rejected', 'skipped')),
    reason text NOT NULL,
    operating_mode text NOT NULL,
    side text NOT NULL,
    target_notional_usd numeric(24, 8),
    target_shares numeric(24, 8),
    limit_price numeric(24, 8),
    book_price numeric(24, 8),
    available_balance_usd numeric(24, 8),
    strategy_snapshot jsonb NOT NULL,
    checks jsonb NOT NULL,
    detected_at timestamptz NOT NULL,
    decided_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (strategy_id, source_event_id)
);

CREATE INDEX copy_decisions_user_time ON copy_decisions (user_id, decided_at DESC);

CREATE TABLE orders (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    trading_account_id uuid NOT NULL REFERENCES trading_accounts(id) ON DELETE CASCADE,
    decision_id uuid NOT NULL UNIQUE REFERENCES copy_decisions(id) ON DELETE CASCADE,
    source_event_id uuid NOT NULL REFERENCES source_events(id) ON DELETE CASCADE,
    client_order_id uuid NOT NULL UNIQUE,
    exchange_order_id text,
    operating_mode text NOT NULL CHECK (operating_mode IN ('paper_trading', 'live_trading')),
    order_type text NOT NULL,
    side text NOT NULL,
    token_id text NOT NULL,
    condition_id text,
    limit_price numeric(24, 8) NOT NULL,
    size_shares numeric(24, 8) NOT NULL,
    status text NOT NULL CHECK (status IN (
        'pending', 'submitting', 'submitted', 'partially_filled', 'filled',
        'cancelled', 'rejected', 'unknown', 'failed'
    )),
    error text,
    exchange_response jsonb,
    retry_count integer NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now(),
    submitted_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE fills (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    trading_account_id uuid NOT NULL REFERENCES trading_accounts(id) ON DELETE CASCADE,
    order_id uuid NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    price numeric(24, 8) NOT NULL,
    shares numeric(24, 8) NOT NULL,
    notional_usd numeric(24, 8) NOT NULL,
    fee_usd numeric(24, 8),
    fee_status text NOT NULL,
    tx_hash text,
    slippage_vs_source_bps integer,
    slippage_vs_limit_bps integer,
    filled_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE positions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    trading_account_id uuid NOT NULL REFERENCES trading_accounts(id) ON DELETE CASCADE,
    source_wallet text NOT NULL,
    token_id text NOT NULL,
    condition_id text,
    outcome text,
    market_title text,
    operating_mode text NOT NULL,
    open_shares numeric(24, 8) NOT NULL DEFAULT 0,
    avg_price numeric(24, 8) NOT NULL DEFAULT 0,
    cost_basis_usd numeric(24, 8) NOT NULL DEFAULT 0,
    realized_pnl_usd numeric(24, 8) NOT NULL DEFAULT 0,
    source_observed_shares numeric(24, 8) NOT NULL DEFAULT 0,
    pending_exit_shares numeric(24, 8) NOT NULL DEFAULT 0,
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (trading_account_id, source_wallet, token_id, operating_mode),
    CHECK (open_shares >= 0),
    CHECK (pending_exit_shares >= 0),
    CHECK (source_observed_shares >= 0)
);

CREATE TABLE closed_positions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    trading_account_id uuid NOT NULL REFERENCES trading_accounts(id) ON DELETE CASCADE,
    source_wallet text NOT NULL,
    token_id text NOT NULL,
    market_title text,
    operating_mode text NOT NULL,
    closed_shares numeric(24, 8) NOT NULL,
    realized_pnl_usd numeric(24, 8) NOT NULL,
    closed_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE timeline_events (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    source_event_id uuid REFERENCES source_events(id) ON DELETE CASCADE,
    decision_id uuid REFERENCES copy_decisions(id) ON DELETE CASCADE,
    order_id uuid REFERENCES orders(id) ON DELETE CASCADE,
    kind text NOT NULL,
    message text NOT NULL,
    at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX timeline_source ON timeline_events (source_event_id, at);

CREATE TABLE audit_log (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    actor_user_id uuid REFERENCES users(id) ON DELETE SET NULL,
    action text NOT NULL,
    target text,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE system_events (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    severity text NOT NULL CHECK (severity IN ('info', 'warning', 'error')),
    component text NOT NULL,
    message text NOT NULL,
    details jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE system_settings (
    key text PRIMARY KEY,
    value jsonb NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO system_settings (key, value)
VALUES
    ('global_kill_switch', '{"enabled": false}'::jsonb),
    ('worker_heartbeat', '{"at": null}'::jsonb),
    ('collector_status', '{"rtds": "starting", "data_api": "starting", "polygon": "disabled"}'::jsonb);

CREATE TABLE collector_checkpoints (
    source_wallet text PRIMARY KEY,
    last_event_ts timestamptz,
    cursor text,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE market_cache (
    token_id text PRIMARY KEY,
    payload jsonb NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE users ENABLE ROW LEVEL SECURITY;
ALTER TABLE users FORCE ROW LEVEL SECURITY;
ALTER TABLE sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE sessions FORCE ROW LEVEL SECURITY;
ALTER TABLE invitation_tokens ENABLE ROW LEVEL SECURITY;
ALTER TABLE invitation_tokens FORCE ROW LEVEL SECURITY;
ALTER TABLE trading_accounts ENABLE ROW LEVEL SECURITY;
ALTER TABLE trading_accounts FORCE ROW LEVEL SECURITY;
ALTER TABLE trading_credentials ENABLE ROW LEVEL SECURITY;
ALTER TABLE trading_credentials FORCE ROW LEVEL SECURITY;
ALTER TABLE account_risk_limits ENABLE ROW LEVEL SECURITY;
ALTER TABLE account_risk_limits FORCE ROW LEVEL SECURITY;
ALTER TABLE source_traders ENABLE ROW LEVEL SECURITY;
ALTER TABLE source_traders FORCE ROW LEVEL SECURITY;
ALTER TABLE copy_strategies ENABLE ROW LEVEL SECURITY;
ALTER TABLE copy_strategies FORCE ROW LEVEL SECURITY;
ALTER TABLE copy_decisions ENABLE ROW LEVEL SECURITY;
ALTER TABLE copy_decisions FORCE ROW LEVEL SECURITY;
ALTER TABLE orders ENABLE ROW LEVEL SECURITY;
ALTER TABLE orders FORCE ROW LEVEL SECURITY;
ALTER TABLE fills ENABLE ROW LEVEL SECURITY;
ALTER TABLE fills FORCE ROW LEVEL SECURITY;
ALTER TABLE positions ENABLE ROW LEVEL SECURITY;
ALTER TABLE positions FORCE ROW LEVEL SECURITY;
ALTER TABLE closed_positions ENABLE ROW LEVEL SECURITY;
ALTER TABLE closed_positions FORCE ROW LEVEL SECURITY;
ALTER TABLE timeline_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE timeline_events FORCE ROW LEVEL SECURITY;
ALTER TABLE audit_log ENABLE ROW LEVEL SECURITY;
ALTER TABLE audit_log FORCE ROW LEVEL SECURITY;

CREATE POLICY users_select ON users FOR SELECT
    USING (app_is_worker() OR app_is_admin() OR id = app_user_id());
CREATE POLICY users_update ON users FOR UPDATE
    USING (app_is_admin() OR id = app_user_id())
    WITH CHECK (app_is_admin() OR id = app_user_id());
CREATE POLICY users_insert ON users FOR INSERT
    WITH CHECK (app_is_admin() OR app_is_worker());

CREATE POLICY sessions_all ON sessions FOR ALL
    USING (app_is_worker() OR app_is_admin() OR user_id = app_user_id())
    WITH CHECK (app_is_worker() OR app_is_admin() OR user_id = app_user_id());

CREATE POLICY invites_all ON invitation_tokens FOR ALL
    USING (app_is_worker() OR app_is_admin() OR user_id = app_user_id())
    WITH CHECK (app_is_worker() OR app_is_admin() OR user_id = app_user_id());

CREATE POLICY accounts_all ON trading_accounts FOR ALL
    USING (app_owns(user_id))
    WITH CHECK (app_owns(user_id));

CREATE POLICY creds_all ON trading_credentials FOR ALL
    USING (app_is_worker() OR user_id = app_user_id())
    WITH CHECK (app_is_worker() OR user_id = app_user_id());

CREATE POLICY risk_all ON account_risk_limits FOR ALL
    USING (app_owns(user_id))
    WITH CHECK (app_owns(user_id));

CREATE POLICY traders_all ON source_traders FOR ALL
    USING (app_owns(user_id))
    WITH CHECK (app_owns(user_id));

CREATE POLICY strategies_all ON copy_strategies FOR ALL
    USING (app_owns(user_id))
    WITH CHECK (app_owns(user_id));

CREATE POLICY decisions_all ON copy_decisions FOR ALL
    USING (app_owns(user_id))
    WITH CHECK (app_owns(user_id));

CREATE POLICY orders_all ON orders FOR ALL
    USING (app_owns(user_id))
    WITH CHECK (app_owns(user_id));

CREATE POLICY fills_all ON fills FOR ALL
    USING (app_owns(user_id))
    WITH CHECK (app_owns(user_id));

CREATE POLICY positions_all ON positions FOR ALL
    USING (app_owns(user_id))
    WITH CHECK (app_owns(user_id));

CREATE POLICY closed_all ON closed_positions FOR ALL
    USING (app_owns(user_id))
    WITH CHECK (app_owns(user_id));

CREATE POLICY timeline_all ON timeline_events FOR ALL
    USING (app_owns(user_id))
    WITH CHECK (app_owns(user_id));

CREATE POLICY audit_select ON audit_log FOR SELECT
    USING (app_is_admin() OR actor_user_id = app_user_id());
CREATE POLICY audit_insert ON audit_log FOR INSERT
    WITH CHECK (app_is_admin() OR app_is_worker() OR actor_user_id = app_user_id());

-- Global market data is not tenant-owned. Workers write it. Authenticated readers can read it.
ALTER TABLE source_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE source_events FORCE ROW LEVEL SECURITY;
ALTER TABLE source_event_fragments ENABLE ROW LEVEL SECURITY;
ALTER TABLE source_event_fragments FORCE ROW LEVEL SECURITY;
ALTER TABLE system_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE system_events FORCE ROW LEVEL SECURITY;
ALTER TABLE system_settings ENABLE ROW LEVEL SECURITY;
ALTER TABLE system_settings FORCE ROW LEVEL SECURITY;
ALTER TABLE collector_checkpoints ENABLE ROW LEVEL SECURITY;
ALTER TABLE collector_checkpoints FORCE ROW LEVEL SECURITY;
ALTER TABLE market_cache ENABLE ROW LEVEL SECURITY;
ALTER TABLE market_cache FORCE ROW LEVEL SECURITY;

CREATE POLICY source_events_read ON source_events FOR SELECT
    USING (
        app_is_worker()
        OR app_is_admin()
        OR EXISTS (
            SELECT 1 FROM source_traders st
            WHERE st.user_id = app_user_id()
              AND lower(st.wallet_address) = lower(source_events.source_wallet)
        )
    );
CREATE POLICY source_events_write ON source_events FOR ALL
    USING (app_is_worker())
    WITH CHECK (app_is_worker());

CREATE POLICY fragments_read ON source_event_fragments FOR SELECT
    USING (
        app_is_worker()
        OR app_is_admin()
        OR EXISTS (
            SELECT 1
            FROM source_events e
            JOIN source_traders st
              ON lower(st.wallet_address) = lower(e.source_wallet)
            WHERE e.id = source_event_fragments.source_event_id
              AND st.user_id = app_user_id()
        )
    );
CREATE POLICY fragments_write ON source_event_fragments FOR ALL
    USING (app_is_worker())
    WITH CHECK (app_is_worker());

CREATE POLICY system_events_read ON system_events FOR SELECT
    USING (app_role() IN ('worker', 'admin'));
CREATE POLICY system_events_write ON system_events FOR INSERT
    WITH CHECK (app_role() IN ('worker', 'admin'));

CREATE POLICY settings_read ON system_settings FOR SELECT
    USING (app_role() IN ('worker', 'admin', 'user'));
CREATE POLICY settings_write ON system_settings FOR ALL
    USING (app_role() IN ('worker', 'admin'))
    WITH CHECK (app_role() IN ('worker', 'admin'));

CREATE POLICY checkpoints_all ON collector_checkpoints FOR ALL
    USING (app_is_worker())
    WITH CHECK (app_is_worker());

CREATE POLICY market_read ON market_cache FOR SELECT
    USING (app_role() IN ('worker', 'admin', 'user'));
CREATE POLICY market_write ON market_cache FOR ALL
    USING (app_is_worker())
    WITH CHECK (app_is_worker());

DO $$
BEGIN
    EXECUTE format(
        'GRANT CONNECT ON DATABASE %I TO pommarket_app, pommarket_worker',
        current_database()
    );
END
$$;

-- Narrow security-definer entry points for unauthenticated auth flows.
-- They never return credential ciphertext.

CREATE OR REPLACE FUNCTION auth_user_by_email(p_email text)
RETURNS TABLE (
    id uuid,
    email text,
    display_name text,
    password_hash text,
    role text,
    status text,
    failed_login_count integer,
    locked_until timestamptz
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
AS $$
    SELECT u.id, u.email, u.display_name, u.password_hash, u.role, u.status,
           u.failed_login_count, u.locked_until
    FROM users u
    WHERE lower(u.email) = lower(p_email)
$$;

CREATE OR REPLACE FUNCTION auth_record_login_failure(p_user uuid)
RETURNS void
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
AS $$
    UPDATE users
    SET failed_login_count = failed_login_count + 1,
        locked_until = CASE
            WHEN failed_login_count + 1 >= 8 THEN now() + interval '15 minutes'
            ELSE locked_until
        END,
        updated_at = now()
    WHERE id = p_user
$$;

CREATE OR REPLACE FUNCTION auth_record_login_success(p_user uuid)
RETURNS void
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
AS $$
    UPDATE users
    SET failed_login_count = 0,
        locked_until = NULL,
        last_login_at = now(),
        updated_at = now()
    WHERE id = p_user
$$;

CREATE OR REPLACE FUNCTION auth_insert_session(
    p_user uuid,
    p_hash text,
    p_expires timestamptz,
    p_user_agent text,
    p_ip text
) RETURNS uuid
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public
AS $$
DECLARE
    new_id uuid;
BEGIN
    INSERT INTO sessions (user_id, refresh_token_hash, expires_at, user_agent, ip)
    VALUES (p_user, p_hash, p_expires, p_user_agent, NULLIF(p_ip, '')::inet)
    RETURNING id INTO new_id;
    RETURN new_id;
END
$$;

CREATE OR REPLACE FUNCTION auth_session_by_hash(p_hash text)
RETURNS TABLE (
    session_id uuid,
    user_id uuid,
    expires_at timestamptz,
    revoked_at timestamptz,
    role text,
    status text,
    email text,
    display_name text
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
AS $$
    SELECT s.id, s.user_id, s.expires_at, s.revoked_at, u.role, u.status, u.email, u.display_name
    FROM sessions s
    JOIN users u ON u.id = s.user_id
    WHERE s.refresh_token_hash = p_hash
$$;

CREATE OR REPLACE FUNCTION auth_revoke_session(p_session uuid)
RETURNS void
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
AS $$
    UPDATE sessions SET revoked_at = now() WHERE id = p_session AND revoked_at IS NULL
$$;

CREATE OR REPLACE FUNCTION auth_invitation_by_hash(p_hash text)
RETURNS TABLE (
    token_id uuid,
    user_id uuid,
    purpose text,
    expires_at timestamptz,
    used_at timestamptz,
    email text,
    status text
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
AS $$
    SELECT t.id, t.user_id, t.purpose, t.expires_at, t.used_at, u.email, u.status
    FROM invitation_tokens t
    JOIN users u ON u.id = t.user_id
    WHERE t.token_hash = p_hash
$$;

CREATE OR REPLACE FUNCTION auth_consume_invitation(p_token uuid, p_user uuid, p_password_hash text)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public
AS $$
BEGIN
    UPDATE invitation_tokens SET used_at = now() WHERE id = p_token AND used_at IS NULL;
    UPDATE users
    SET password_hash = p_password_hash,
        status = 'active',
        updated_at = now()
    WHERE id = p_user;
END
$$;

REVOKE ALL ON FUNCTION auth_user_by_email(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_record_login_failure(uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_record_login_success(uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_insert_session(uuid, text, timestamptz, text, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_session_by_hash(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_revoke_session(uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_invitation_by_hash(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_consume_invitation(uuid, uuid, text) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION auth_user_by_email(text) TO pommarket_app, pommarket_worker;
GRANT EXECUTE ON FUNCTION auth_record_login_failure(uuid) TO pommarket_app;
GRANT EXECUTE ON FUNCTION auth_record_login_success(uuid) TO pommarket_app;
GRANT EXECUTE ON FUNCTION auth_insert_session(uuid, text, timestamptz, text, text) TO pommarket_app;
GRANT EXECUTE ON FUNCTION auth_session_by_hash(text) TO pommarket_app;
GRANT EXECUTE ON FUNCTION auth_revoke_session(uuid) TO pommarket_app;
GRANT EXECUTE ON FUNCTION auth_invitation_by_hash(text) TO pommarket_app;
GRANT EXECUTE ON FUNCTION auth_consume_invitation(uuid, uuid, text) TO pommarket_app;
GRANT USAGE ON SCHEMA public TO pommarket_app, pommarket_worker;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO pommarket_app, pommarket_worker;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO pommarket_app, pommarket_worker;

-- Credentials stay unreadable to the admin role through RLS. Revoke nothing extra:
-- pommarket_app can read ciphertext only when app.role is worker or the owning user.
