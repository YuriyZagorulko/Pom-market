ALTER TABLE users ADD COLUMN IF NOT EXISTS email_verified_at timestamptz;

ALTER TABLE invitation_tokens DROP CONSTRAINT IF EXISTS invitation_tokens_purpose_check;
ALTER TABLE invitation_tokens
    ADD CONSTRAINT invitation_tokens_purpose_check
    CHECK (purpose IN ('invite', 'reset', 'verify'));

DROP FUNCTION IF EXISTS auth_record_login_failure(uuid);

CREATE OR REPLACE FUNCTION auth_record_login_failure(p_user uuid)
RETURNS TABLE (failed_login_count integer, locked_until timestamptz)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
AS $$
    UPDATE users
    SET failed_login_count = users.failed_login_count + 1,
        locked_until = CASE
            WHEN users.failed_login_count + 1 >= 8 THEN now() + interval '15 minutes'
            ELSE users.locked_until
        END,
        updated_at = now()
    WHERE users.id = p_user
    RETURNING users.failed_login_count, users.locked_until
$$;

CREATE OR REPLACE FUNCTION auth_consume_verification(p_token uuid, p_user uuid)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public
AS $$
BEGIN
    UPDATE invitation_tokens SET used_at = now() WHERE id = p_token AND used_at IS NULL;
    UPDATE users
    SET email_verified_at = now(),
        updated_at = now()
    WHERE id = p_user;
END
$$;

REVOKE ALL ON FUNCTION auth_consume_verification(uuid, uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION auth_consume_verification(uuid, uuid) TO pommarket_app;

CREATE OR REPLACE FUNCTION auth_create_reset_token(p_email text, p_token_hash text, p_ttl_hours integer)
RETURNS TABLE (email text, display_name text)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public
AS $$
DECLARE
    v_user users%ROWTYPE;
BEGIN
    SELECT * INTO v_user
    FROM users u
    WHERE lower(u.email) = lower(p_email)
      AND u.status = 'active'
      AND u.password_hash IS NOT NULL;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    INSERT INTO invitation_tokens (user_id, token_hash, purpose, expires_at)
    VALUES (v_user.id, p_token_hash, 'reset', now() + make_interval(hours => p_ttl_hours));
    email := v_user.email;
    display_name := v_user.display_name;
    RETURN NEXT;
END
$$;

REVOKE ALL ON FUNCTION auth_create_reset_token(text, text, integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION auth_create_reset_token(text, text, integer) TO pommarket_app;
