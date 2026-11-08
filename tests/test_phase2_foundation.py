"""Phase 2 Automated Tests: Authentication, RBAC, TOTP, Tenant Isolation (App + PostgreSQL RLS), and Trading Accounts."""

from __future__ import annotations

from decimal import Decimal
import pytest
from sqlalchemy import select

from backend.app.db import set_tenant_context
from backend.app.models import (
    OperatingMode,
    SignatureType,
    TradingAccount,
    TradingAccountCredential,
)
from backend.app.schemas import (
    CredentialRotateRequest,
    EmergencyStopRequest,
    OperatingModeChangeRequest,
    TradingAccountCreateRequest,
    UserLoginRequest,
    UserRegisterRequest,
)
from backend.app.security import (
    CredentialVault,
    compute_totp_code,
    redact_sensitive_dict,
)
from backend.app.services.account_service import (
    AccountValidationError,
    TenantAccessDeniedError,
    TradingAccountService,
)
from backend.app.services.auth_service import AuthError, AuthService
from backend.app.services.email_service import DEV_MAILBOX


def test_user_registration_login_refresh_and_totp(db_session):
    auth = AuthService()

    # 1. Register user (verification token is delivered via out-of-band DEV_MAILBOX, never in HTTP response)
    reg = auth.register_user(
        db_session,
        UserRegisterRequest(
            email="Alice@Example.com",
            password="StrongPassword!123",
            full_name="Alice Trader",
        ),
    )
    assert reg.user.email == "alice@example.com"
    assert reg.user.roles == ["USER"]
    assert reg.user.email_verified is False
    assert reg.verification_token_preview is None

    # 2. Verify email using token retrieved from out-of-band DEV_MAILBOX
    verify_token = DEV_MAILBOX.get_latest_token("alice@example.com", "email_verification")
    assert verify_token is not None
    verified_user = auth.verify_email(db_session, verify_token)
    assert verified_user.email_verified is True

    # 3. Refresh session rotates token and invalidates old refresh token
    refreshed = auth.refresh_session(db_session, reg.refresh_token)
    assert refreshed.access_token
    assert refreshed.refresh_token != reg.refresh_token
    with pytest.raises(AuthError):
        auth.refresh_session(db_session, reg.refresh_token)

    # 4. Enable TOTP 2FA
    totp_setup = auth.setup_totp(db_session, reg.user.id)
    valid_code = compute_totp_code(totp_setup.secret)
    enabled_user = auth.enable_totp(db_session, reg.user.id, valid_code)
    assert enabled_user.totp_enabled is True

    # Login without TOTP fails once TOTP is enabled
    with pytest.raises(AuthError):
        auth.login(
            db_session,
            UserLoginRequest(email="alice@example.com", password="StrongPassword!123"),
        )

    # Login with valid TOTP succeeds
    logged_in = auth.login(
        db_session,
        UserLoginRequest(
            email="alice@example.com",
            password="StrongPassword!123",
            totp_code=compute_totp_code(totp_setup.secret),
        ),
    )
    assert logged_in.user.id == reg.user.id

    # 5. Password reset revokes sessions
    reset_token = auth.initiate_password_reset(db_session, "alice@example.com")
    assert reset_token is not None
    auth.confirm_password_reset(db_session, reset_token, "NewStrongPassword!456")
    with pytest.raises(AuthError):
        auth.refresh_session(db_session, logged_in.refresh_token)


def test_multiple_accounts_signature_types_and_secret_protection(db_session):
    auth = AuthService()
    acct_svc = TradingAccountService()

    user = auth.register_user(
        db_session,
        UserRegisterRequest(email="trader1@example.com", password="Password123!"),
    ).user

    # POLY_GNOSIS_SAFE without funder_wallet_address must fail validation
    with pytest.raises(AccountValidationError):
        acct_svc.create_trading_account(
            db_session,
            user_id=user.id,
            req=TradingAccountCreateRequest(
                display_name="Invalid Safe Account",
                signer_wallet_address="0x90f79bf6eb2c4f870365e785982e1f101e93b906",
                funder_wallet_address=None,
                signature_type=SignatureType.POLY_GNOSIS_SAFE,
            ),
        )

    # Create Account 1 (Gnosis Safe) and Account 2 (EOA) for the same user
    acc1 = acct_svc.create_trading_account(
        db_session,
        user_id=user.id,
        req=TradingAccountCreateRequest(
            display_name="Main Safe Account",
            signer_wallet_address="0x90f79bf6eb2c4f870365e785982e1f101e93b906",
            funder_wallet_address="0x1111111111111111111111111111111111111111",
            signature_type=SignatureType.POLY_GNOSIS_SAFE,
            private_key="0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6",
            api_key="api-key-1",
            api_secret="c2VjcmV0XzE=",
            api_passphrase="pass-1",
            initial_paper_balance_usd=Decimal("2500.0"),
        ),
    )
    acc2 = acct_svc.create_trading_account(
        db_session,
        user_id=user.id,
        req=TradingAccountCreateRequest(
            display_name="Secondary EOA Account",
            signer_wallet_address="0x2222222222222222222222222222222222222222",
            signature_type=SignatureType.EOA,
        ),
    )

    assert acc1.operating_mode == OperatingMode.MONITORING.value
    assert acc1.has_signing_key is True
    assert acc1.has_api_credentials is True
    assert acc2.funder_wallet_address == "0x2222222222222222222222222222222222222222"

    # Verify secrets are encrypted in DB and never returned in response serialization
    dumped = acc1.model_dump()
    assert "private_key" not in dumped
    assert "api_secret" not in dumped
    cred_row = db_session.scalar(
        select(TradingAccountCredential).where(TradingAccountCredential.trading_account_id == acc1.id)
    )
    assert cred_row is not None
    assert "7c852118" not in (cred_row.encrypted_private_key or "")
    assert cred_row.encrypted_private_key.startswith("v1:")

    # Switching to LIVE_TRADING without explicit confirmation must fail
    with pytest.raises(AccountValidationError):
        acct_svc.change_operating_mode(
            db_session,
            user_id=user.id,
            account_id=acc1.id,
            req=OperatingModeChangeRequest(operating_mode=OperatingMode.LIVE_TRADING),
        )

    # Switching with explicit confirmation succeeds for acc1 (has credentials), fails for acc2 (no credentials)
    live_acc1 = acct_svc.change_operating_mode(
        db_session,
        user_id=user.id,
        account_id=acc1.id,
        req=OperatingModeChangeRequest(
            operating_mode=OperatingMode.LIVE_TRADING,
            explicit_confirmation="CONFIRM_LIVE_TRADING",
        ),
    )
    assert live_acc1.operating_mode == OperatingMode.LIVE_TRADING.value

    with pytest.raises(AccountValidationError):
        acct_svc.change_operating_mode(
            db_session,
            user_id=user.id,
            account_id=acc2.id,
            req=OperatingModeChangeRequest(
                operating_mode=OperatingMode.LIVE_TRADING,
                explicit_confirmation="CONFIRM_LIVE_TRADING",
            ),
        )

    # Rotate credentials increments key_version
    rotated = acct_svc.rotate_credentials(
        db_session,
        user_id=user.id,
        account_id=acc1.id,
        req=CredentialRotateRequest(private_key="0xaaaa"),
    )
    assert rotated.key_version == 2
    bundle = acct_svc.get_decrypted_signing_bundle(db_session, user_id=user.id, account_id=acc1.id)
    assert bundle["private_key"] == "0xaaaa"
    assert redact_sensitive_dict(bundle)["private_key"] == "***REDACTED***"

    # Disconnect wipes credentials and resets mode to MONITORING
    disconnected = acct_svc.disconnect_account(db_session, user_id=user.id, account_id=acc1.id)
    assert disconnected.has_signing_key is False
    assert disconnected.operating_mode == OperatingMode.MONITORING.value


def test_cross_tenant_isolation_and_postgresql_rls(db_session):
    auth = AuthService()
    acct_svc = TradingAccountService()
    vault = CredentialVault()

    user_a = auth.register_user(
        db_session,
        UserRegisterRequest(email="user_a@example.com", password="PasswordA123!"),
    ).user
    user_b = auth.register_user(
        db_session,
        UserRegisterRequest(email="user_b@example.com", password="PasswordB123!"),
    ).user

    set_tenant_context(db_session, user_id=user_a.id)
    acc_a = acct_svc.create_trading_account(
        db_session,
        user_id=user_a.id,
        req=TradingAccountCreateRequest(
            display_name="User A Vaulted Account",
            signer_wallet_address="0x90f79bf6eb2c4f870365e785982e1f101e93b906",
            signature_type=SignatureType.EOA,
            private_key="0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
        ),
    )

    set_tenant_context(db_session, user_id=user_b.id)
    acc_b = acct_svc.create_trading_account(
        db_session,
        user_id=user_b.id,
        req=TradingAccountCreateRequest(
            display_name="User B Account",
            signer_wallet_address="0x3333333333333333333333333333333333333333",
            signature_type=SignatureType.EOA,
        ),
    )

    # 1. Service-level cross-tenant access violations are blocked
    with pytest.raises(TenantAccessDeniedError):
        acct_svc.get_account_response(db_session, user_id=user_b.id, account_id=acc_a.id)

    with pytest.raises(TenantAccessDeniedError):
        acct_svc.set_emergency_stop(
            db_session,
            user_id=user_b.id,
            account_id=acc_a.id,
            req=EmergencyStopRequest(emergency_stop=True, reason="Malicious cross-tenant stop"),
        )

    with pytest.raises(TenantAccessDeniedError):
        acct_svc.get_decrypted_signing_bundle(db_session, user_id=user_b.id, account_id=acc_a.id)

    # 2. PostgreSQL Row-Level Security (RLS) blocks direct table reads across tenants
    set_tenant_context(db_session, user_id=user_b.id, is_super_admin=False)
    visible_accounts = db_session.scalars(select(TradingAccount)).all()
    assert [a.id for a in visible_accounts] == [acc_b.id]

    visible_creds = db_session.scalars(select(TradingAccountCredential)).all()
    assert [c.trading_account_id for c in visible_creds] == [acc_b.id]

    # 3. Cryptographic AAD binding prevents User B from decrypting User A's ciphertext
    set_tenant_context(db_session, user_id=None, is_super_admin=True)
    cred_a = db_session.scalar(
        select(TradingAccountCredential).where(TradingAccountCredential.trading_account_id == acc_a.id)
    )
    with pytest.raises(PermissionError):
        vault.decrypt_secret(
            cred_a.encrypted_private_key,
            user_id=user_b.id,
            trading_account_id=acc_a.id,
        )
