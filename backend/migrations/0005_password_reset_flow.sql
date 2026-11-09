DROP FUNCTION IF EXISTS auth_create_reset_token(text, text, integer);

CREATE OR REPLACE FUNCTION auth_create_reset_token(p_email text, p_token_hash text, p_ttl_minutes integer)
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
    VALUES (v_user.id, p_token_hash, 'reset', now() + (p_ttl_minutes * interval '1 minute'));
    email := v_user.email;
    display_name := v_user.display_name;
    RETURN NEXT;
END
$$;

CREATE OR REPLACE FUNCTION auth_consume_password_reset(p_token uuid, p_user uuid, p_password_hash text)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public
AS $$
DECLARE
    consumed integer;
BEGIN
    UPDATE invitation_tokens
    SET used_at = now()
    WHERE id = p_token
      AND user_id = p_user
      AND purpose = 'reset'
      AND used_at IS NULL
      AND expires_at > now();
    GET DIAGNOSTICS consumed = ROW_COUNT;
    IF consumed = 0 THEN
        RAISE EXCEPTION 'reset token is no longer valid';
    END IF;

    UPDATE users
    SET password_hash = p_password_hash,
        updated_at = now()
    WHERE id = p_user;

    UPDATE invitation_tokens
    SET used_at = now()
    WHERE user_id = p_user
      AND purpose = 'reset'
      AND used_at IS NULL
      AND id <> p_token;

    UPDATE sessions
    SET revoked_at = now()
    WHERE user_id = p_user
      AND revoked_at IS NULL;
END
$$;

REVOKE ALL ON FUNCTION auth_consume_password_reset(uuid, uuid, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION auth_consume_password_reset(uuid, uuid, text) TO pommarket_app;
